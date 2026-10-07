//! A multi-channel audio processor that derives the role envelopes from its
//! input.
//!
//! On the mono mix of the input channels: automatic trim → DC blocker →
//! constant-Q resonator bank → the roles, which follow the bank sample by
//! sample and report once per buffer
//! (see [`crate::roles`]). The role selected by `active_role` feeds the show.
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::bank::ResonatorBank;
use crate::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};
use crate::roles::{NUM_ROLES, Role, Roles};

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
    /// Current envelope value for the show loop (from `active_role`).
    pub envelope: AtomicF32,
    /// The level roles' envelope attack and release half-lives, in seconds.
    pub envelope_attack: AtomicF32,
    pub envelope_release: AtomicF32,
    /// Symmetric output smoothing time constant (seconds). 0 = disabled.
    pub output_smoothing: AtomicF32,

    /// Floor tracking half-life in seconds (slow — adapts to ambient level).
    pub norm_floor_halflife: AtomicF32,
    /// Ceiling tracking half-life, in seconds of music at
    /// `REFERENCE_MOTION_RATE`. The ceiling's memory is really a quantity of
    /// envelope motion, so quiet or still material stretches these seconds
    /// and a silent band holds the ceiling indefinitely.
    pub norm_ceiling_halflife: AtomicF32,

    /// Which role feeds `envelope`, as its index in [`Role::ALL`].
    pub active_role: AtomicU32,
    /// The gain the automatic trim is currently applying, as a multiplier.
    pub trim_gain: AtomicF32,
    /// Runs of samples pinned at full scale on any one input channel,
    /// counted since the processor was built; it wraps on overflow.
    pub input_clips: AtomicU32,
}

impl ProcessorSettingsInner {
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;
    /// Floor half-life: slow enough that a bass line is above its own bed,
    /// fast enough that a held tone stops being news within a phrase or two.
    pub const DEFAULT_FLOOR_HALFLIFE: f32 = 10.0;
    /// Ceiling half-life of about a phrase: each level role is measured
    /// against the loudest thing in the phrase it is part of.
    pub const DEFAULT_CEILING_HALFLIFE: f32 = 8.0;
    /// The role the show follows until told otherwise: the low end's level,
    /// the nearest thing to a whole-song envelope.
    pub const DEFAULT_ROLE: Role = Role::Bass;

    pub fn reset_defaults(&self) {
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
        self.active_role
            .store(Self::DEFAULT_ROLE.index() as u32, Ordering::Relaxed);
        self.norm_floor_halflife.set(Self::DEFAULT_FLOOR_HALFLIFE);
        self.norm_ceiling_halflife
            .set(Self::DEFAULT_CEILING_HALFLIFE);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            norm_floor_halflife: AtomicF32::new(Self::DEFAULT_FLOOR_HALFLIFE),
            norm_ceiling_halflife: AtomicF32::new(Self::DEFAULT_CEILING_HALFLIFE),
            active_role: AtomicU32::new(Self::DEFAULT_ROLE.index() as u32),
            trim_gain: AtomicF32::new(1.0),
            input_clips: AtomicU32::new(0),
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
    update_rate: f32,
    peak_fall_coeff: f32,
    gain_coeff: f32,
}

impl AutoTrim {
    /// Peak level the trim aims for. Unity: downstream is floating point
    /// and a momentary overshoot costs nothing.
    const TARGET: f32 = 1.0;
    /// How far the trim may go. The boost reaches a feed run well below a
    /// mastered level; past that it would be lifting the interface's own
    /// noise toward the gate.
    const MIN_GAIN_DB: f32 = -10.0;
    const MAX_GAIN_DB: f32 = 20.0;
    /// Peak tracker fall half-life.
    const PEAK_FALL_HALFLIFE: f32 = 10.0;
    /// Gain slew half-life, the same in both directions: slow enough that a
    /// stray peak costs only a gentle, brief dip.
    const GAIN_HALFLIFE: f32 = 5.0;
    /// Input peak below which the trim holds still, so silence and idle
    /// noise are never boosted toward the target.
    const SILENCE: f32 = 0.01;

    fn new() -> Self {
        Self {
            peak: 0.0,
            gain_db: 0.0,
            gain: 1.0,
            update_rate: 0.0,
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
    fn set_params(&mut self, update_rate: f32) {
        if update_rate == self.update_rate {
            return;
        }
        self.update_rate = update_rate;
        self.peak_fall_coeff = halflife_to_coeff(Self::PEAK_FALL_HALFLIFE, update_rate);
        self.gain_coeff = halflife_to_coeff(Self::GAIN_HALFLIFE, update_rate);
    }

    /// Update the trim from the peak level of one buffer of input.
    fn update(&mut self, buffer_peak: f32) {
        if buffer_peak < Self::SILENCE {
            return;
        }

        self.peak = if buffer_peak > self.peak {
            buffer_peak
        } else {
            self.peak_fall_coeff * self.peak + (1.0 - self.peak_fall_coeff) * buffer_peak
        };

        let desired_db = Self::linear_to_db(Self::TARGET / self.peak)
            .clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
        self.gain_db = self.gain_coeff * self.gain_db + (1.0 - self.gain_coeff) * desired_db;
        self.gain = Self::db_to_linear(self.gain_db);
    }
}

/// Level at or beyond which an input sample is at the converter's ceiling.
const CLIP_LEVEL: f32 = 0.999;

/// Consecutive samples on one channel at that level before it counts as
/// clipping rather than a signal that happens to touch full scale.
const CLIP_RUN: u32 = 3;

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
/// `halflife_secs` at `update_rate` updates per second. A non-positive
/// half-life means no smoothing.
pub(crate) fn halflife_to_coeff(halflife_secs: f32, update_rate: f32) -> f32 {
    if halflife_secs <= 0.0 || update_rate <= 0.0 {
        return 0.0;
    }
    (-f32::ln(2.0) / (halflife_secs * update_rate)).exp()
}

/// The normalizer's fixed constants: what it treats as silence and how
/// little range it will stretch to full scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizerTuning {
    /// Minimum normalization range as a fraction of the ceiling. Once the
    /// floor has climbed to within this fraction of the ceiling the output
    /// fades instead of being stretched back to full scale, and a band's
    /// reach to full scale never depends on its absolute level.
    pub rel_min_range: f32,
    /// The lowest level the floor may sit at, so a band whose content is
    /// at or below it outputs zero rather than normalizing idle noise up to
    /// full scale.
    pub noise_gate: f32,
}

impl NormalizerTuning {
    pub const DEFAULT: Self = Self {
        rel_min_range: 0.25,
        noise_gate: 0.01,
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
    /// Nepers the ceiling decays per neper of log-envelope motion.
    ceiling_forget: f32,
    /// The half-life the forgetting rate was derived from.
    ceiling_halflife: f32,
    /// Floor follower coefficients: the floor rises at the floor half-life
    /// and falls at a fifth of it.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// The inputs the coefficients were derived from.
    floor_halflife: f32,
    update_rate: f32,
}

impl NormalizerParams {
    /// The floor falls this much faster than it rises.
    const FLOOR_FALL_RATIO: f32 = 0.2;

    fn new(tuning: NormalizerTuning) -> Self {
        Self {
            tuning,
            ceiling_forget: 0.0,
            ceiling_halflife: 0.0,
            floor_rise_coeff: 0.0,
            floor_fall_coeff: 0.0,
            floor_halflife: 0.0,
            update_rate: 0.0,
        }
    }

    /// Refresh from the settings for the given update rate. Safe to call
    /// every buffer.
    fn refresh(&mut self, settings: &ProcessorSettingsInner, update_rate: f32) {
        let ceiling_halflife = settings.norm_ceiling_halflife.get();
        if ceiling_halflife != self.ceiling_halflife {
            self.ceiling_halflife = ceiling_halflife;
            // A non-positive half-life means the ceiling never forgets.
            self.ceiling_forget = if ceiling_halflife > 0.0 {
                std::f32::consts::LN_2 / (REFERENCE_MOTION_RATE * ceiling_halflife)
            } else {
                0.0
            };
        }
        let floor_halflife = settings.norm_floor_halflife.get();
        if update_rate <= 0.0
            || (floor_halflife == self.floor_halflife && update_rate == self.update_rate)
        {
            return;
        }
        self.floor_halflife = floor_halflife;
        self.update_rate = update_rate;
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff =
            halflife_to_coeff(floor_halflife * Self::FLOOR_FALL_RATIO, update_rate);
    }
}

/// Adaptive envelope normalizer: tracks a floor and ceiling,
/// outputs `(envelope - floor) / (ceiling - floor)` clamped to [0, 1].
///
/// The ceiling is a peak follower whose decay is clocked by the envelope's
/// own motion rather than by time: it rises to any envelope above it at once
/// and forgets a fixed number of nepers for every neper the log envelope
/// moves, up or down, set by the ceiling half-life. A hit therefore costs the ceiling the same whether
/// the music is fast or slow, a level drop is forgotten within a few hits
/// at any tempo, and a pause or a held tone — no motion — leaves the
/// ceiling where it was.
pub(crate) struct AdaptiveNormalizer {
    /// Follows the envelope slowly upward and faster downward: the bed the
    /// output is measured from.
    floor: AsymmetricOnePole,
    ceiling: f32,
    /// Log envelope on the previous update, clamped at the noise gate.
    prev_log_envelope: f32,
}

impl AdaptiveNormalizer {
    /// The ceiling a normalizer starts from, before it has heard anything:
    /// half a full-scale tone's envelope. Starting high under-reports until the
    /// real level is learnt, rather than calling the first sound full scale;
    /// louder material costs one hit's worth of clipping.
    const INITIAL_CEILING: f32 = 0.5;

    pub(crate) fn new(tuning: &NormalizerTuning) -> Self {
        Self {
            floor: AsymmetricOnePole::default(),
            ceiling: Self::INITIAL_CEILING,
            prev_log_envelope: tuning.noise_gate.ln(),
        }
    }

    /// Where the normalizer stands, given the envelope it was last fed.
    pub(crate) fn stages(&self, smoothed: f32) -> BandStages {
        BandStages {
            smoothed,
            floor: self.floor.state,
            ceiling: self.ceiling,
        }
    }

    #[inline]
    pub(crate) fn process(&mut self, envelope: f32, p: &NormalizerParams) -> f32 {
        let t = &p.tuning;
        // Update ceiling: instant attack, decay per unit of envelope motion.
        let log_envelope = envelope.max(t.noise_gate).ln();
        let motion = (log_envelope - self.prev_log_envelope).abs();
        self.prev_log_envelope = log_envelope;
        self.ceiling = envelope.max(self.ceiling * (-p.ceiling_forget * motion).exp());

        // Update floor. It never exceeds the ceiling.
        self.floor.rise = p.floor_rise_coeff;
        self.floor.fall = p.floor_fall_coeff;
        let floor = self.floor.step(envelope.min(self.ceiling));

        // The gate is the lowest level the floor can sit at, so the output
        // reaches zero by arriving there rather than by being cut off.
        let floor = floor.max(t.noise_gate);
        // The range is never narrower than the gate either, so a band only
        // reaches full scale once its ceiling stands clear of the noise it
        // would otherwise be stretching.
        let range = (self.ceiling - floor)
            .max(t.rel_min_range * self.ceiling)
            .max(t.noise_gate);
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
/// while `x[n] > y[n-1]` and the fall coefficient otherwise. A coefficient of
/// zero follows the input at once in that direction.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AsymmetricOnePole {
    pub(crate) rise: f32,
    pub(crate) fall: f32,
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
    time_secs: f32,
    update_rate: f32,
    coeff: f32,
}

impl SmootherCoeff {
    /// Refresh the cached coefficient from the current time constant and
    /// update rate. Safe to call every tick.
    fn refresh(&mut self, time_secs: f32, update_rate: f32) {
        if time_secs == self.time_secs && update_rate == self.update_rate {
            return;
        }
        self.time_secs = time_secs;
        self.update_rate = update_rate;
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
    envelope_attack: f32,
    envelope_release: f32,
    /// The buffer rate the roles' timing was last set for.
    update_rate: f32,
    /// Interleaved channels per frame, never zero.
    channel_count: usize,
    sample_rate: f32,

    /// Envelope ring buffer producers, one per role.
    envelope_producers: [EnvelopeProducer; NUM_ROLES],

    /// Brings the input to a consistent level ahead of everything else.
    auto_trim: AutoTrim,
    /// Each channel's samples at full scale so far, for detecting a clipping
    /// run that straddles two buffers.
    clip_runs: Box<[u32]>,
    /// Removes any offset from the mix before it reaches the bank.
    dc_blocker: DcBlocker,
    /// Splits the mix into the bands every role is built from.
    bank: ResonatorBank,
    roles: Roles,
    /// Cached smoother coefficient shared by the level roles.
    smooth_coeff: SmootherCoeff,
    /// Normalizer coefficients shared by the level roles.
    norm_params: NormalizerParams,
}

impl Processor {
    pub fn new(
        handle: ProcessorSettings,
        sample_rate: u32,
        channel_count: usize,
        envelope_producers: [EnvelopeProducer; NUM_ROLES],
    ) -> Self {
        let sample_rate = sample_rate as f32;
        let tuning = NormalizerTuning::DEFAULT;
        let bank = ResonatorBank::new(sample_rate);
        let roles = Roles::new(
            std::array::from_fn(|b| bank.is_live(b)),
            &tuning,
            sample_rate,
        );
        Self {
            envelope_attack: f32::NAN,
            envelope_release: f32::NAN,
            update_rate: 0.0,
            settings: handle,
            channel_count: channel_count.max(1),
            sample_rate,
            envelope_producers,
            auto_trim: AutoTrim::new(),
            clip_runs: vec![0; channel_count.max(1)].into_boxed_slice(),
            dc_blocker: DcBlocker::new(sample_rate),
            bank,
            roles,
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

    /// Count each channel's runs of samples pinned at full scale into
    /// `input_clips`, and return the buffer's peak level across all channels.
    fn count_input_clipping(&mut self, interleaved_buffer: &[f32]) -> f32 {
        let mut clips = 0;
        let mut peak = 0.0_f32;
        for frame in interleaved_buffer.chunks(self.channel_count) {
            for (sample, run) in frame.iter().zip(self.clip_runs.iter_mut()) {
                let level = sample.abs();
                peak = peak.max(level);
                if level >= CLIP_LEVEL {
                    *run += 1;
                    if *run == CLIP_RUN {
                        clips += 1;
                    }
                } else {
                    *run = 0;
                }
            }
        }
        if clips > 0 {
            self.settings
                .input_clips
                .fetch_add(clips, Ordering::Relaxed);
        }
        peak
    }

    /// A level role's intermediate stage values; hit roles have none.
    pub fn stages(&self, role: Role) -> Option<BandStages> {
        self.roles.stages(role)
    }

    fn maybe_update_parameters(&mut self, buffer_rate: UpdateRate) {
        let update_rate = buffer_rate.as_hz();
        let attack = self.settings.envelope_attack.get();
        let release = self.settings.envelope_release.get();
        if attack != self.envelope_attack
            || release != self.envelope_release
            || update_rate != self.update_rate
        {
            self.envelope_attack = attack;
            self.envelope_release = release;
            self.update_rate = update_rate;
            self.roles.set_timing(attack, release, buffer_rate);
        }

        self.smooth_coeff
            .refresh(self.settings.output_smoothing.get(), update_rate);
        self.norm_params.refresh(&self.settings, update_rate);
    }

    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        // Exact digital silence decays the bank and followers into subnormal
        // values, which some CPUs process many times more slowly.
        crate::denormals::flush_subnormals_to_zero();
        if interleaved_buffer.is_empty() {
            return;
        }

        let frames = interleaved_buffer.len() / self.channel_count;
        if frames == 0 {
            return;
        }
        let buffer_rate = UpdateRate::new(self.sample_rate as u32, frames as u32);
        let update_rate = buffer_rate.as_hz();

        self.maybe_update_parameters(buffer_rate);

        // The trim reads the input before its own gain reaches it, so it
        // cannot chase itself.
        let input_peak = self.count_input_clipping(interleaved_buffer);
        self.auto_trim.set_params(update_rate);
        self.auto_trim.update(input_peak);
        self.settings.trim_gain.set(self.auto_trim.gain);
        let gain = self.auto_trim.gain;
        let ch_count_f = self.channel_count as f32;

        for frame in interleaved_buffer.chunks(self.channel_count) {
            // A device that hands us a non-finite sample would otherwise
            // poison the resonators and followers for the rest of the show:
            // their state is recursive, so a NaN in it never washes out.
            let mono = frame.iter().sum::<f32>() / ch_count_f * gain;
            let mono = if mono.is_finite() { mono } else { 0.0 };
            self.bank.push(self.dc_blocker.process(mono));
            self.roles.push_sample(&self.bank);
        }

        let outputs = self
            .roles
            .finish(self.smooth_coeff.get(), &self.norm_params);

        // Push the roles to ring buffers for the GUI viewer.
        for (producer, &val) in self.envelope_producers.iter_mut().zip(&outputs) {
            producer.push(val);
        }

        // Write the active role's value to the shared envelope atomic.
        let active = self.settings.active_role.load(Ordering::Relaxed) as usize;
        let active = Role::from_index(active).unwrap_or(ProcessorSettingsInner::DEFAULT_ROLE);
        self.settings.envelope.set(outputs[active.index()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_producers() -> [EnvelopeProducer; NUM_ROLES] {
        envelope_ring_buffers().producers
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
        let mut processor = Processor::new(settings.clone(), sample_rate, 1, test_producers());
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
            peak = peak.max(settings.envelope.get());
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
            trim.set_params(1000.0);
            trim
        }
        fn feed(trim: &mut AutoTrim, peak: f32, updates: usize) {
            for _ in 0..updates {
                trim.update(peak);
            }
        }

        // Right at target: stays put, and silence afterwards leaves it there.
        let mut trim = trim_at_1khz();
        feed(&mut trim, AutoTrim::TARGET, 5000);
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
    /// there has been through a converter that ran out of range.
    #[test]
    fn only_a_run_at_full_scale_counts_as_clipping() {
        let settings = ProcessorSettings::default();
        let mut processor = Processor::new(settings.clone(), 48000, 1, test_producers());

        let mut buffer = vec![0.5_f32; 64];
        buffer[10] = 1.0;
        buffer[30] = -1.0;
        processor.process(&buffer);
        assert_eq!(
            settings.input_clips.load(Ordering::Relaxed),
            0,
            "isolated samples at full scale are not clipping"
        );

        buffer[20..28].fill(1.0);
        processor.process(&buffer);
        assert_eq!(
            settings.input_clips.load(Ordering::Relaxed),
            1,
            "eight samples pinned at full scale are one clipped run"
        );

        // Stereo: runs are counted per channel, not across the interleave.
        let settings = ProcessorSettings::default();
        let mut processor = Processor::new(settings.clone(), 48000, 2, test_producers());
        let mut stereo = vec![0.5_f32; 128];
        for frame in 10..18 {
            stereo[2 * frame] = 1.0;
        }
        processor.process(&stereo);
        assert_eq!(
            settings.input_clips.load(Ordering::Relaxed),
            1,
            "one channel pinned for eight frames is clipping, whatever the other does"
        );
        let mut stereo = vec![0.5_f32; 128];
        stereo[40..44].fill(1.0);
        processor.process(&stereo);
        assert_eq!(
            settings.input_clips.load(Ordering::Relaxed),
            1,
            "both channels at full scale for two frames is two samples per channel, not a run"
        );
    }

    /// Helper: feed a processor one mono signal in 48-sample buffers for the
    /// given duration. Returns the final envelope value.
    fn run_processor(
        settings: &ProcessorSettings,
        sample_rate: u32,
        duration_secs: f32,
        sample: impl Fn(f32) -> f32,
    ) -> f32 {
        let buffer_size = 48;
        let total_samples = (duration_secs * sample_rate as f32) as usize;
        let mut processor = Processor::new(settings.clone(), sample_rate, 1, test_producers());

        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| sample(i as f32 / sample_rate as f32))
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        settings.envelope.get()
    }
}
