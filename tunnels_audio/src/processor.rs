//! A multi-channel audio processor that derives the role envelopes from its
//! input.
//!
//! On the mono mix of the input channels: automatic trim → DC blocker →
//! constant-Q resonator bank → the roles, which follow the bank sample by
//! sample and report once per buffer
//! (see [`crate::roles`]). The same bank feeds the spectrum (see
//! [`crate::spectrum`]). Each buffer, every role and band is published as one
//! [`AudioFrame`].
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use tunnels_lib::transient_indicator::TransientIndicator;

use tunnels_lib::audio::{AudioFrame, UnipolarF32};

use crate::bank::{NUM_BANDS, ResonatorBank};
use crate::frame_buffer::FrameProducer;
use crate::input_meter::InputMeter;
use crate::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};
use crate::roles::{NUM_ROLES, Role, Roles};
use crate::spectrum::Spectrum;
use crate::time::{AtomicDuration, AtomicHalfLife, HalfLife};

/// An `f32` shared between threads, stored as its bit pattern.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Ring buffer capacity: ~16 seconds of history at ~1kHz buffer rate.
pub const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// One envelope ring buffer per role: the producing ends and the reading
/// ends, each in [`Role::ALL`] order.
pub struct EnvelopeRingBuffers {
    pub producers: [EnvelopeProducer; NUM_ROLES],
    pub streams: [EnvelopeStream; NUM_ROLES],
}

/// Create one envelope ring buffer per role, in [`Role::ALL`] order, each
/// holding `ENVELOPE_HISTORY_CAPACITY` values.
pub fn envelope_ring_buffers() -> EnvelopeRingBuffers {
    let (producers, streams): (Vec<_>, Vec<_>) = (0..NUM_ROLES)
        .map(|_| envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY))
        .unzip();
    EnvelopeRingBuffers {
        producers: producers.try_into().ok().expect("one producer per role"),
        streams: streams.try_into().ok().expect("one stream per role"),
    }
}

/// Audio callback rate in Hz (sample_rate / frames_per_buffer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdateRate(f32);

impl UpdateRate {
    pub fn new(sample_rate: u32, frames_per_buffer: u32) -> Self {
        Self(sample_rate as f32 / frames_per_buffer as f32)
    }

    pub fn as_hz(self) -> f32 {
        self.0
    }

    pub fn interval_secs(self) -> f64 {
        1.0 / self.0 as f64
    }
}

pub struct ProcessorSettingsInner {
    /// The level roles' envelope attack and release half-lives.
    pub envelope_attack: AtomicDuration,
    pub envelope_release: AtomicDuration,
    /// Symmetric output smoothing time constant. Zero disables smoothing.
    pub output_smoothing: AtomicDuration,

    /// Floor tracking half-life (slow — adapts to ambient level).
    pub norm_floor_halflife: AtomicHalfLife,
    /// Ceiling tracking half-life, as a span of music at
    /// `REFERENCE_MOTION_RATE`. The ceiling's memory is really a quantity of
    /// envelope motion, so quiet or still material stretches this span and a
    /// silent band holds the ceiling indefinitely.
    pub norm_ceiling_halflife: AtomicHalfLife,
}

impl ProcessorSettingsInner {
    const DEFAULT_ENVELOPE_ATTACK: Duration = Duration::from_millis(10);
    const DEFAULT_ENVELOPE_RELEASE: Duration = Duration::from_millis(50);
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: Duration = Duration::from_millis(8);
    /// Floor half-life: slow enough that a bass line is above its own bed,
    /// fast enough that a held tone stops being news within a phrase or two.
    pub const DEFAULT_FLOOR_HALFLIFE: HalfLife =
        HalfLife::from_millis(NonZeroU64::new(10_000).expect("non-zero"));
    /// Ceiling half-life of about a phrase: each level role is measured
    /// against the loudest thing in the phrase it is part of.
    pub const DEFAULT_CEILING_HALFLIFE: HalfLife =
        HalfLife::from_millis(NonZeroU64::new(8_000).expect("non-zero"));

    pub fn reset_defaults(&self) {
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
        self.norm_floor_halflife.set(Self::DEFAULT_FLOOR_HALFLIFE);
        self.norm_ceiling_halflife
            .set(Self::DEFAULT_CEILING_HALFLIFE);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope_attack: AtomicDuration::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicDuration::new(Self::DEFAULT_ENVELOPE_RELEASE),
            output_smoothing: AtomicDuration::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            norm_floor_halflife: AtomicHalfLife::new(Self::DEFAULT_FLOOR_HALFLIFE),
            norm_ceiling_halflife: AtomicHalfLife::new(Self::DEFAULT_CEILING_HALFLIFE),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// A slow automatic gain that brings the input's peaks toward a fixed target,
/// within limits, so the level the interface is set to does not decide what
/// sits above the noise gate. It holds still on silence.
struct AutoTrim {
    /// Tracked input peak: instant attack, slow decay.
    peak: f32,
    gain_db: f32,
    /// `gain_db` as a multiplier.
    gain: f32,
    /// The update rate the coefficients were derived for, if they have been.
    update_rate: Option<UpdateRate>,
    /// Per-update coefficients in [0, 1): the fraction of the previous peak
    /// and gain kept each update.
    peak_fall_coeff: f32,
    gain_coeff: f32,
}

impl AutoTrim {
    /// Peak level the trim aims for. Unity: downstream is floating point
    /// and a momentary overshoot costs nothing.
    const TARGET: UnipolarF32 = UnipolarF32::ONE;
    /// How far the trim may go. The boost reaches a feed run well below a
    /// mastered level; past that it would be lifting the interface's own
    /// noise toward the gate.
    const MIN_GAIN_DB: f32 = -10.0;
    const MAX_GAIN_DB: f32 = 20.0;
    /// Peak tracker fall half-life.
    const PEAK_FALL_HALFLIFE: Duration = Duration::from_secs(10);
    /// Gain slew half-life, the same in both directions: slow enough that a
    /// stray peak costs only a gentle, brief dip.
    const GAIN_HALFLIFE: Duration = Duration::from_secs(5);
    /// Input peak below which the trim holds still, so silence and idle
    /// noise are never boosted toward the target.
    const SILENCE: UnipolarF32 = UnipolarF32::new(0.01);

    fn new() -> Self {
        Self {
            peak: 0.0,
            gain_db: 0.0,
            gain: 1.0,
            update_rate: None,
            peak_fall_coeff: 0.0,
            gain_coeff: 0.0,
        }
    }

    fn db_to_linear(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    fn linear_to_db(linear: f32) -> f32 {
        20.0 * linear.log10()
    }

    /// Refresh the cached coefficients for the update rate. Safe to call
    /// every buffer.
    fn set_params(&mut self, update_rate: UpdateRate) {
        if self.update_rate == Some(update_rate) {
            return;
        }
        self.update_rate = Some(update_rate);
        self.peak_fall_coeff = halflife_to_coeff(Self::PEAK_FALL_HALFLIFE, update_rate);
        self.gain_coeff = halflife_to_coeff(Self::GAIN_HALFLIFE, update_rate);
    }

    /// Update the trim from the peak level of one buffer of input.
    fn update(&mut self, buffer_peak: f32) {
        if buffer_peak < Self::SILENCE.val() {
            return;
        }

        self.peak = if buffer_peak > self.peak {
            buffer_peak
        } else {
            self.peak_fall_coeff * self.peak + (1.0 - self.peak_fall_coeff) * buffer_peak
        };

        let desired_db = Self::linear_to_db(Self::TARGET.val() / self.peak)
            .clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
        self.gain_db = self.gain_coeff * self.gain_db + (1.0 - self.gain_coeff) * desired_db;
        self.gain = Self::db_to_linear(self.gain_db);
    }
}

/// Level at or beyond which an input sample is at the converter's ceiling.
const CLIP_LEVEL: UnipolarF32 = UnipolarF32::new(0.999);

/// Consecutive samples on one channel at that level before it counts as
/// clipping rather than a signal that happens to touch full scale.
const CLIP_RUN: u32 = 3;

/// How long the clip indicator stays lit after the last buffer that clipped.
const CLIP_HOLD: Duration = Duration::from_millis(300);

/// Whether any one channel of an interleaved buffer of `channel_count`
/// channels holds a run of [`CLIP_RUN`] consecutive samples at
/// [`CLIP_LEVEL`] or beyond.
fn buffer_clips(interleaved_buffer: &[f32], channel_count: NonZeroUsize) -> bool {
    let channel_count = channel_count.get();
    (0..channel_count).any(|channel| {
        let mut run = 0;
        interleaved_buffer
            .iter()
            .skip(channel)
            .step_by(channel_count)
            .any(|sample| {
                if sample.abs() >= CLIP_LEVEL.val() {
                    run += 1;
                } else {
                    run = 0;
                }
                run >= CLIP_RUN
            })
    })
}

/// Nepers per second a level role's log envelope moves on music: the median
/// over twelve tracks of varied genres and the three level roles, at 64
/// frames per buffer. A role with nothing in its range barely moves; a busy
/// one moves up to about three times this. The ceiling's memory is a quantity
/// of envelope motion, and this rate converts it to a half-life in seconds.
///
/// Motion accrues once per buffer, so a longer buffer misses ripple finer
/// than its period and forgets a little more slowly than the half-life says.
pub const REFERENCE_MOTION_RATE: f32 = 4.2;

/// One-pole EMA coefficient that halves the distance to the target every
/// `halflife` at `update_rate`: the fraction of the previous value kept each
/// update, in [0, 1). A zero half-life means no smoothing.
pub(crate) fn halflife_to_coeff(halflife: Duration, update_rate: UpdateRate) -> f32 {
    let halflife_secs = halflife.as_secs_f32();
    let update_rate = update_rate.as_hz();
    if halflife_secs <= 0.0 || update_rate <= 0.0 {
        return 0.0;
    }
    (-f32::ln(2.0) / (halflife_secs * update_rate)).exp()
}

/// A count of seconds as a duration; zero for a count that is negative, not
/// a number, or too large to represent.
pub(crate) fn secs_to_duration(secs: f32) -> Duration {
    Duration::try_from_secs_f32(secs).unwrap_or(Duration::ZERO)
}

/// The normalizer's fixed constants: what it treats as silence and how
/// little range it will stretch to full scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizerTuning {
    /// Minimum normalization range as a fraction of the ceiling. Once the
    /// floor has climbed to within this fraction of the ceiling the output
    /// fades instead of being stretched back to full scale, and a band's
    /// reach to full scale never depends on its absolute level.
    pub rel_min_range: UnipolarF32,
    /// The lowest level the floor may sit at, so a band whose content is
    /// at or below it outputs zero rather than normalizing idle noise up to
    /// full scale.
    pub noise_gate: UnipolarF32,
}

impl NormalizerTuning {
    pub const DEFAULT: Self = Self {
        rel_min_range: UnipolarF32::new(0.25),
        noise_gate: UnipolarF32::new(0.01),
    };
}

impl Default for NormalizerTuning {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The normalizer coefficients every band shares, derived from the settings
/// and the buffer update rate. The expensive `exp()`s are recomputed only
/// when a half-life or the update rate changes.
pub(crate) struct NormalizerParams {
    tuning: NormalizerTuning,
    /// Nepers the ceiling decays per neper of log-envelope motion: zero
    /// until derived, positive after.
    ceiling_forget: f32,
    /// The half-life the forgetting rate was derived from, if it has been.
    ceiling_halflife: Option<HalfLife>,
    /// Floor follower coefficients, each in [0, 1): the floor rises at the
    /// floor half-life and falls at a fifth of it.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// The inputs the floor coefficients were derived from, if they have
    /// been.
    floor_halflife: Option<HalfLife>,
    update_rate: Option<UpdateRate>,
}

impl NormalizerParams {
    /// The floor's fall half-life as a fraction of its rise half-life.
    const FLOOR_FALL_RATIO: UnipolarF32 = UnipolarF32::new(0.2);

    fn new(tuning: NormalizerTuning) -> Self {
        Self {
            tuning,
            ceiling_forget: 0.0,
            ceiling_halflife: None,
            floor_rise_coeff: 0.0,
            floor_fall_coeff: 0.0,
            floor_halflife: None,
            update_rate: None,
        }
    }

    /// Nepers the ceiling decays per neper of log-envelope motion.
    pub(crate) fn ceiling_forget(&self) -> f32 {
        self.ceiling_forget
    }

    /// Refresh from the settings for the given update rate. Safe to call
    /// every buffer.
    fn refresh(&mut self, settings: &ProcessorSettingsInner, update_rate: UpdateRate) {
        let ceiling_halflife = settings.norm_ceiling_halflife.get();
        if self.ceiling_halflife != Some(ceiling_halflife) {
            self.ceiling_halflife = Some(ceiling_halflife);
            self.ceiling_forget = std::f32::consts::LN_2
                / (REFERENCE_MOTION_RATE * ceiling_halflife.get().as_secs_f32());
        }
        let floor_halflife = settings.norm_floor_halflife.get();
        if self.floor_halflife == Some(floor_halflife) && self.update_rate == Some(update_rate) {
            return;
        }
        self.floor_halflife = Some(floor_halflife);
        self.update_rate = Some(update_rate);
        let floor_halflife = floor_halflife.get();
        let fall_halflife =
            secs_to_duration(floor_halflife.as_secs_f32() * Self::FLOOR_FALL_RATIO.val());
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff = halflife_to_coeff(fall_halflife, update_rate);
    }
}

/// A peak follower whose decay is clocked by the envelope's own motion
/// rather than by time: it rises to any envelope above it at once and forgets
/// a fixed number of nepers for every neper the log envelope moves, up or
/// down. A hit therefore costs the ceiling the same whether the music is fast
/// or slow, a level drop is forgotten within a few hits at any tempo, and a
/// pause or a held tone — no motion — leaves the ceiling where it was.
///
/// Motion below the gate is not counted, so noise under it never wears the
/// ceiling down.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MotionCeiling {
    ceiling: f32,
    gate: f32,
    /// Log envelope on the previous update, clamped at the gate.
    prev_log_envelope: f32,
}

impl MotionCeiling {
    /// A ceiling starting at `initial`, which counts no motion below `gate`.
    pub(crate) fn new(initial: f32, gate: f32) -> Self {
        Self {
            ceiling: initial,
            gate,
            prev_log_envelope: gate.ln(),
        }
    }

    /// The ceiling's current level.
    pub(crate) fn level(&self) -> f32 {
        self.ceiling
    }

    /// Follow one more envelope value, forgetting `forget` nepers per neper
    /// of motion, and return the new ceiling.
    #[inline]
    pub(crate) fn step(&mut self, envelope: f32, forget: f32) -> f32 {
        let log_envelope = envelope.max(self.gate).ln();
        let motion = (log_envelope - self.prev_log_envelope).abs();
        self.prev_log_envelope = log_envelope;
        self.ceiling = envelope.max(self.ceiling * (-forget * motion).exp());
        self.ceiling
    }
}

/// Adaptive envelope normalizer: tracks a floor and ceiling,
/// outputs `(envelope - floor) / (ceiling - floor)` clamped to [0, 1].
///
/// The ceiling is a [`MotionCeiling`] forgetting at the rate the ceiling
/// half-life sets, gated at the noise gate.
pub(crate) struct AdaptiveNormalizer {
    /// Follows the envelope slowly upward and faster downward: the bed the
    /// output is measured from.
    floor: AsymmetricOnePole,
    ceiling: MotionCeiling,
}

impl AdaptiveNormalizer {
    /// The ceiling a normalizer starts from, before it has heard anything:
    /// half a full-scale tone's envelope. Starting high under-reports until the
    /// real level is learnt, rather than calling the first sound full scale;
    /// louder material costs one hit's worth of clipping.
    const INITIAL_CEILING: UnipolarF32 = UnipolarF32::new(0.5);

    pub(crate) fn new(tuning: &NormalizerTuning) -> Self {
        Self {
            floor: AsymmetricOnePole::default(),
            ceiling: MotionCeiling::new(Self::INITIAL_CEILING.val(), tuning.noise_gate.val()),
        }
    }

    /// Where the normalizer stands, given the envelope it was last fed.
    pub(crate) fn stages(&self, smoothed: f32) -> BandStages {
        BandStages {
            smoothed,
            floor: self.floor.state,
            ceiling: self.ceiling.level(),
        }
    }

    #[inline]
    pub(crate) fn process(&mut self, envelope: f32, p: &NormalizerParams) -> f32 {
        let noise_gate = p.tuning.noise_gate.val();
        let rel_min_range = p.tuning.rel_min_range.val();
        let ceiling = self.ceiling.step(envelope, p.ceiling_forget);

        // Update floor. It never exceeds the ceiling.
        self.floor.rise = p.floor_rise_coeff;
        self.floor.fall = p.floor_fall_coeff;
        let floor = self.floor.step(envelope.min(ceiling));

        // The gate is the lowest level the floor can sit at, so the output
        // reaches zero by arriving there rather than by being cut off.
        let floor = floor.max(noise_gate);
        // The range is never narrower than the gate either, so a band only
        // reaches full scale once its ceiling stands clear of the noise it
        // would otherwise be stretching.
        let range = (ceiling - floor)
            .max(rel_min_range * ceiling)
            .max(noise_gate);
        ((envelope - floor) / range).clamp(0.0, 1.0)
    }
}

/// One-pole DC blocker: `y[n] = x[n] - x[n-1] + coeff * y[n-1]`, the
/// difference equation of a series capacitor, removing any constant offset.
struct DcBlocker {
    coeff: f32,
    x_prev: f32,
    y_prev: f32,
}

impl DcBlocker {
    /// Corner frequency: within 0.2 dB of flat from a few tens of hertz up.
    const CORNER_HZ: f32 = 5.0;

    fn new(sample_rate: f32) -> Self {
        Self {
            coeff: 1.0 - std::f32::consts::TAU * Self::CORNER_HZ / sample_rate,
            x_prev: 0.0,
            y_prev: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = input - self.x_prev + self.coeff * self.y_prev;
        self.x_prev = input;
        self.y_prev = output;
        output
    }
}

/// Symmetric one-pole IIR smoother: `y[n] = coeff * y[n-1] + (1 - coeff) * x[n]`.
/// Coefficient is supplied per update since it is typically shared across
/// many smoother instances.
#[derive(Default)]
pub(crate) struct OnePoleSmoother {
    state: f32,
}

impl OnePoleSmoother {
    pub(crate) fn update(&mut self, coeff: f32, input: f32) -> f32 {
        self.state = coeff * self.state + (1.0 - coeff) * input;
        self.state
    }

    pub(crate) fn state(&self) -> f32 {
        self.state
    }
}

/// One-pole follower with separate coefficients for rising and falling
/// input: `y[n] = c * y[n-1] + (1 - c) * x[n]` with `c` the rise coefficient
/// while `x[n] > y[n-1]` and the fall coefficient otherwise.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AsymmetricOnePole {
    /// The coefficient while the input is above the state: dimensionless, in
    /// [0, 1), the fraction of the previous state kept each update. Zero
    /// follows a rising input at once.
    pub(crate) rise: f32,
    /// The coefficient while the input is at or below the state, in the same
    /// terms. Zero follows a falling input at once.
    pub(crate) fall: f32,
    /// The latest output, on the input's scale.
    state: f32,
}

impl AsymmetricOnePole {
    pub(crate) fn new(rise: f32, fall: f32) -> Self {
        Self {
            rise,
            fall,
            state: 0.0,
        }
    }

    #[inline]
    pub(crate) fn step(&mut self, input: f32) -> f32 {
        let c = if input > self.state {
            self.rise
        } else {
            self.fall
        };
        self.state = c * self.state + (1.0 - c) * input;
        self.state
    }
}

/// A one-pole smoother coefficient derived from a time constant and the audio
/// buffer update rate. Recomputes the expensive `exp()` only when either
/// changes.
#[derive(Default)]
struct SmootherCoeff {
    /// The inputs the coefficient was derived from, if it has been.
    time_constant: Duration,
    update_rate: Option<UpdateRate>,
    /// Dimensionless, in [0, 1): the fraction of the previous output kept
    /// each update. Zero passes the input through.
    coeff: f32,
}

impl SmootherCoeff {
    /// Refresh the cached coefficient from the current time constant and
    /// update rate. Safe to call every tick.
    fn refresh(&mut self, time_constant: Duration, update_rate: UpdateRate) {
        if time_constant == self.time_constant && self.update_rate == Some(update_rate) {
            return;
        }
        self.time_constant = time_constant;
        self.update_rate = Some(update_rate);
        let time_secs = time_constant.as_secs_f32();
        let update_rate = update_rate.as_hz();
        self.coeff = if time_secs <= 0.0 || update_rate <= 0.0 {
            0.0
        } else {
            (-1.0 / (time_secs * update_rate)).exp()
        };
    }

    fn get(&self) -> f32 {
        self.coeff
    }
}

/// A level role's intermediate values as of the most recently processed
/// buffer: the smoothed envelope entering the normalizer, and the floor and
/// ceiling the normalizer is currently tracking.
#[derive(Debug, Clone, Copy)]
pub struct BandStages {
    pub smoothed: f32,
    pub floor: f32,
    pub ceiling: f32,
}

pub struct Processor {
    settings: ProcessorSettings,
    /// The envelope attack and release, and the buffer rate, the roles'
    /// timing was last set for, if it has been.
    envelope_attack: Option<Duration>,
    envelope_release: Option<Duration>,
    update_rate: Option<UpdateRate>,
    /// Interleaved channels per frame.
    channel_count: NonZeroUsize,
    sample_rate: f32,

    /// Envelope ring buffer producers, one per role.
    envelope_producers: [EnvelopeProducer; NUM_ROLES],
    /// Where each buffer's frame is published.
    frames: FrameProducer,

    /// Brings the input to a consistent level ahead of everything else.
    auto_trim: AutoTrim,
    /// Lit by a buffer in which the input clipped, and held for [`CLIP_HOLD`].
    clip_indicator: TransientIndicator,
    /// Where the trim and the clip indicator are published each buffer.
    input_meter: Arc<InputMeter>,
    /// Removes any offset from the mix before it reaches the bank.
    dc_blocker: DcBlocker,
    /// Splits the mix into the bands every role is built from.
    bank: ResonatorBank,
    roles: Roles,
    spectrum: Spectrum,
    /// Cached smoother coefficient shared by the level roles.
    smooth_coeff: SmootherCoeff,
    /// Normalizer coefficients shared by the level roles.
    norm_params: NormalizerParams,
}

impl Processor {
    pub fn new(
        handle: ProcessorSettings,
        sample_rate: u32,
        channel_count: NonZeroUsize,
        envelope_producers: [EnvelopeProducer; NUM_ROLES],
        frames: FrameProducer,
    ) -> Self {
        let per_sample = UpdateRate::new(sample_rate, 1);
        let sample_rate = sample_rate as f32;
        let tuning = NormalizerTuning::DEFAULT;
        let bank = ResonatorBank::new(sample_rate);
        let live = std::array::from_fn(|b| bank.is_live(b));
        let roles = Roles::new(live, &tuning, per_sample);
        let spectrum = Spectrum::new(live, sample_rate);
        Self {
            envelope_attack: None,
            envelope_release: None,
            update_rate: None,
            settings: handle,
            channel_count,
            sample_rate,
            envelope_producers,
            frames,
            auto_trim: AutoTrim::new(),
            clip_indicator: TransientIndicator::new(CLIP_HOLD),
            input_meter: Arc::new(InputMeter::default()),
            dc_blocker: DcBlocker::new(sample_rate),
            bank,
            roles,
            spectrum,
            smooth_coeff: SmootherCoeff::default(),
            norm_params: NormalizerParams::new(tuning),
        }
    }

    /// Replace the normalizer tuning and restart the level roles' normalizers
    /// from their initial state.
    pub fn set_normalizer_tuning(&mut self, tuning: NormalizerTuning) {
        self.norm_params = NormalizerParams::new(tuning);
        self.roles.set_normalizer_tuning(&tuning);
    }

    /// The meter this processor publishes its trim and clip indicator to.
    ///
    /// The processor holds the meter's only strong reference, so the handle
    /// upgrades exactly as long as the processor exists.
    pub fn input_meter(&self) -> Weak<InputMeter> {
        Arc::downgrade(&self.input_meter)
    }

    /// A level role's intermediate stage values; hit roles have none.
    pub fn stages(&self, role: Role) -> Option<BandStages> {
        self.roles.stages(role)
    }

    /// Each spectrum band's level in dB per octave at the end of the latest
    /// buffer, ahead of any whitening.
    pub fn spectrum_levels_db(&self) -> [f32; NUM_BANDS] {
        self.spectrum.levels_db()
    }

    fn maybe_update_parameters(&mut self, buffer_rate: UpdateRate) {
        let attack = self.settings.envelope_attack.get();
        let release = self.settings.envelope_release.get();
        if self.envelope_attack != Some(attack)
            || self.envelope_release != Some(release)
            || self.update_rate != Some(buffer_rate)
        {
            self.envelope_attack = Some(attack);
            self.envelope_release = Some(release);
            self.update_rate = Some(buffer_rate);
            self.roles.set_timing(attack, release, buffer_rate);
        }

        self.smooth_coeff
            .refresh(self.settings.output_smoothing.get(), buffer_rate);
        self.norm_params.refresh(&self.settings, buffer_rate);
    }

    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        // Exact digital silence decays the bank and followers into subnormal
        // values, which some CPUs process many times more slowly.
        crate::denormals::flush_subnormals_to_zero();
        if interleaved_buffer.is_empty() {
            return;
        }

        let channel_count = self.channel_count.get();
        let frames = interleaved_buffer.len() / channel_count;
        if frames == 0 {
            return;
        }
        let buffer_rate = UpdateRate::new(self.sample_rate as u32, frames as u32);

        self.maybe_update_parameters(buffer_rate);

        // The trim reads the input before its own gain reaches it, so it
        // cannot chase itself.
        let input_peak = interleaved_buffer
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        self.auto_trim.set_params(buffer_rate);
        self.auto_trim.update(input_peak);
        let gain = self.auto_trim.gain;

        self.clip_indicator.update_state(
            secs_to_duration(frames as f32 / self.sample_rate),
            buffer_clips(interleaved_buffer, self.channel_count),
        );
        self.input_meter
            .set(self.auto_trim.gain_db, self.clip_indicator.state());

        let ch_count_f = channel_count as f32;

        for frame in interleaved_buffer.chunks(channel_count) {
            // A device that hands us a non-finite sample would otherwise
            // poison the resonators and followers for the rest of the show:
            // their state is recursive, so a NaN in it never washes out.
            let mono = frame.iter().sum::<f32>() / ch_count_f * gain;
            let mono = if mono.is_finite() { mono } else { 0.0 };
            self.bank.push(self.dc_blocker.process(mono));
            self.roles.push_sample(&self.bank);
            self.spectrum.push_sample(&self.bank);
        }

        let roles = self
            .roles
            .finish(self.smooth_coeff.get(), &self.norm_params)
            .map(UnipolarF32::new);
        let spectrum = self
            .spectrum
            .finish(buffer_rate.as_hz(), self.norm_params.ceiling_forget());

        for (producer, &value) in self.envelope_producers.iter_mut().zip(&roles) {
            producer.push(value);
        }
        self.frames.publish(AudioFrame::new(roles, spectrum));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_buffer::{FrameReader, frame_buffer};

    fn channels(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("a test processor has at least one channel")
    }

    /// A processor of `channel_count` channels at `sample_rate`, and the
    /// reading end of its frames.
    fn new_processor(
        settings: &ProcessorSettings,
        sample_rate: u32,
        channel_count: usize,
    ) -> (Processor, FrameReader) {
        let (producer, reader) = frame_buffer();
        let processor = Processor::new(
            settings.clone(),
            sample_rate,
            channels(channel_count),
            envelope_ring_buffers().producers,
            producer,
        );
        (processor, reader)
    }

    #[test]
    fn processor_produces_envelope_from_sine() {
        let settings = ProcessorSettings::default();
        let envelope = run_processor(&settings, 48000, 1.0, |t| {
            0.7 * (std::f32::consts::TAU * 100.0 * t).sin()
        });
        assert!(
            envelope > 0.3,
            "Envelope should be non-trivial after 1s of 100Hz sine, got {:.3}",
            envelope
        );
    }

    /// A band whose input is a constant offset is not hearing anything, so
    /// its envelope stays at zero.
    #[test]
    fn dc_offset_produces_no_envelope() {
        let settings = ProcessorSettings::default();
        let envelope = run_processor(&settings, 48000, 1.0, |_| 0.5);
        assert!(
            envelope < 0.05,
            "a 0.5 DC offset should read as silence, got {envelope:.3}"
        );
    }

    /// A processor that has heard nothing has no idea how loud the music
    /// is, so it starts quiet and works its way up rather than reporting
    /// full scale from the first sample it sees.
    #[test]
    fn a_new_processor_does_not_start_at_full_scale() {
        let settings = ProcessorSettings::default();
        let sample_rate = 48000_u32;
        let (mut processor, mut frames) = new_processor(&settings, sample_rate, 1);
        // A third of full scale, so full scale is unambiguously wrong.
        let mut peak = 0.0_f32;
        for buffer in 0..80 {
            let samples: Vec<f32> = (0..64)
                .map(|i| {
                    let t = (buffer * 64 + i) as f32 / sample_rate as f32;
                    0.3 * (2.0 * std::f32::consts::PI * 60.0 * t).sin()
                })
                .collect();
            processor.process(&samples);
            peak = peak.max(frames.latest().role(Role::Bass).val() as f32);
        }
        assert!(
            peak < 0.9,
            "the first 107 ms of a quiet tone should not read as full scale, got {peak:.3}"
        );
    }

    /// The trim follows the input's level, stops at its limits, and treats
    /// silence as nothing to act on.
    #[test]
    fn auto_trim_follows_level_and_holds_on_silence() {
        fn trim_at_1khz() -> AutoTrim {
            let mut trim = AutoTrim::new();
            trim.set_params(UpdateRate::new(48_000, 48));
            trim
        }
        fn feed(trim: &mut AutoTrim, peak: f32, updates: usize) {
            for _ in 0..updates {
                trim.update(peak);
            }
        }

        // Right at target: stays put, and silence afterwards leaves it there.
        let mut trim = trim_at_1khz();
        feed(&mut trim, AutoTrim::TARGET.val(), 5000);
        assert!(
            (trim.gain - 1.0).abs() < 0.05,
            "at target the trim should hold near 1.0, got {:.3}",
            trim.gain
        );
        feed(&mut trim, 0.0, 5000);
        feed(&mut trim, 0.005, 5000);
        assert!(
            (trim.gain - 1.0).abs() < 0.01,
            "silence should leave the trim where it was, got {:.3}",
            trim.gain
        );

        // A quiet feed is boosted, up to the limit.
        let mut trim = trim_at_1khz();
        feed(&mut trim, 0.02, 60000);
        let max = AutoTrim::db_to_linear(AutoTrim::MAX_GAIN_DB);
        assert!(
            trim.gain > 0.9 * max && trim.gain <= max + 0.01,
            "a feed 34 dB down should reach the boost limit {max:.1}, got {:.3}",
            trim.gain
        );

        // A hot feed is reduced, down to the limit.
        let mut trim = trim_at_1khz();
        feed(&mut trim, 1.5, 15000);
        let min = AutoTrim::db_to_linear(AutoTrim::MIN_GAIN_DB);
        assert!(
            trim.gain < 0.75 && trim.gain >= min - 0.01,
            "a hot feed should be reduced but not below {min:.2}, got {:.3}",
            trim.gain
        );
    }

    /// A signal that touches full scale is not clipping; one that sits
    /// there has been through a converter that ran out of range. Runs are
    /// counted per channel, and any one channel clipping is enough.
    #[test]
    fn a_run_at_full_scale_on_any_channel_is_clipping() {
        let mono = channels(1);
        let mut buffer = vec![0.5_f32; 64];
        buffer[10] = 1.0;
        buffer[30] = -1.0;
        assert!(
            !buffer_clips(&buffer, mono),
            "isolated samples at full scale are not clipping"
        );
        buffer[20..22].fill(1.0);
        assert!(
            !buffer_clips(&buffer, mono),
            "two samples at full scale are not a run"
        );
        buffer[40..43].fill(-1.0);
        assert!(
            buffer_clips(&buffer, mono),
            "three samples pinned at negative full scale are a run"
        );

        let stereo = channels(2);
        for channel in 0..2 {
            let mut buffer = vec![0.5_f32; 128];
            for frame in 10..18 {
                buffer[2 * frame + channel] = 1.0;
            }
            assert!(
                buffer_clips(&buffer, stereo),
                "channel {channel} pinned for eight frames is clipping, whatever the other does"
            );
        }
        let mut buffer = vec![0.5_f32; 128];
        buffer[40..44].fill(1.0);
        assert!(
            !buffer_clips(&buffer, stereo),
            "both channels at full scale for two frames is two samples per channel, not a run"
        );
    }

    /// The clip indicator lights on a buffer that clipped and stays lit for
    /// the hold time of clean input after it.
    #[test]
    fn clip_indicator_holds_after_a_clipped_buffer() {
        // 48 frames at 48 kHz: each buffer is 1 ms.
        let (mut processor, _frames) = new_processor(&ProcessorSettings::default(), 48000, 1);
        let clean = vec![0.5_f32; 48];
        let mut clipped = clean.clone();
        clipped[20..28].fill(1.0);

        processor.process(&clean);
        assert!(
            !processor.input_meter.clip_lit(),
            "clean input leaves the indicator dark"
        );
        processor.process(&clipped);
        assert!(
            processor.input_meter.clip_lit(),
            "a clipped buffer lights the indicator"
        );
        for _ in 0..299 {
            processor.process(&clean);
        }
        assert!(
            processor.input_meter.clip_lit(),
            "299 ms of clean input after clipping is within the hold"
        );
        for _ in 0..2 {
            processor.process(&clean);
        }
        assert!(
            !processor.input_meter.clip_lit(),
            "301 ms of clean input after clipping is past the hold"
        );
    }

    /// The meter handle stops upgrading once its processor is dropped.
    #[test]
    fn input_meter_lives_as_long_as_its_processor() {
        let (processor, _frames) = new_processor(&ProcessorSettings::default(), 48000, 1);
        let meter = processor.input_meter();
        assert!(
            meter.upgrade().is_some(),
            "the processor keeps its meter alive"
        );
        drop(processor);
        assert!(
            meter.upgrade().is_none(),
            "nothing but the processor keeps its meter alive"
        );
    }

    /// Helper: feed a processor one mono signal in 48-sample buffers for the
    /// given duration. Returns Bass's final value.
    fn run_processor(
        settings: &ProcessorSettings,
        sample_rate: u32,
        duration_secs: f32,
        sample: impl Fn(f32) -> f32,
    ) -> f32 {
        let buffer_size = 48;
        let total_samples = (duration_secs * sample_rate as f32) as usize;
        let (mut processor, mut frames) = new_processor(settings, sample_rate, 1);

        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| sample(i as f32 / sample_rate as f32))
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        frames.latest().role(Role::Bass).val() as f32
    }
}
