use crate::animation_target::AnimationTarget;
use crate::clock::Clock;
use crate::clock::ControllableClock;
use crate::clock::Ticks;
use crate::clock_bank::ClockStore;
use crate::waveforms::WaveformArgs;
use crate::{clock_bank::ClockIdx, waveforms};
use noise::NoiseFn;
use noise::Simplex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use strum::VariantArray;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};
use tunnels_lib::smooth::Smoother;

#[derive(Copy, Clone, Serialize, Deserialize, Debug, VariantArray)]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Sawtooth,
    Noise,
    Constant,
}

impl Waveform {
    /// Whether the value depends on the phase it is asked at.
    ///
    /// A constant answers one number for every phase, so a caller that would
    /// otherwise resolve it across a coordinate can resolve it once.
    pub fn varies_with_phase(self) -> bool {
        match self {
            Self::Sine | Self::Triangle | Self::Square | Self::Sawtooth | Self::Noise => true,
            Self::Constant => false,
        }
    }
}

/// The animation parameters that are fixed for the duration of a frame.
///
/// These are used as given. The rest of what an animation's value depends on —
/// clock phase, elapsed ticks, the smoother's position, the amplitude — has to
/// be resolved against the clocks before it can be read, and so is held apart
/// from these.
#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub struct StaticParams {
    pub waveform: Waveform,
    pub pulse: bool,
    pub standing: bool,
    pub invert: bool,
    pub n_periods: u16,
    pub duty_cycle: UnipolarFloat,
}

impl Default for StaticParams {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            pulse: false,
            standing: false,
            invert: false,
            n_periods: 1,
            duty_cycle: UnipolarFloat::ONE,
        }
    }
}

/// An animation and the parameter it drives.
///
/// Generic over the animation so that one already resolved for a frame pairs
/// with its target the same way an unresolved one does: it is the same
/// association either side of `prepare`, and naming it twice would make two
/// things out of one.
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct TargetedAnimation<A = Animation> {
    pub animation: A,
    pub target: AnimationTarget,
}

impl TargetedAnimation {
    /// Resolve everything that is fixed for a frame, keeping the target.
    pub fn prepare(
        &self,
        external_clocks: &impl ClockStore,
        audio_envelope: UnipolarFloat,
        span: OffsetSpan,
    ) -> TargetedAnimation<PreparedAnimation> {
        TargetedAnimation {
            animation: self
                .animation
                .prepare(external_clocks, audio_envelope, span),
            target: self.target,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Animation {
    static_params: StaticParams,
    size: UnipolarFloat,
    /// Use a smoother for the smoothing parameter.
    /// This is only necessary when used as the noise cross-correlation parameter,
    /// since small changes imply significant movements in the noise distribution.
    /// TODO: consider if we want to turn smoothing of this parameter off when
    /// we're in anything besides noise.
    smoothing: Smoother<UnipolarFloat>,
    internal_clock: Clock,
    clock_source: Option<ClockIdx>,
    use_audio_size: bool,
    #[serde(skip, default = "get_simplex_gen")]
    simplex_gen: &'static Simplex,
}

fn get_simplex_gen() -> &'static Simplex {
    static SIMPLEX_GEN: LazyLock<Simplex> = LazyLock::new(Default::default);

    &SIMPLEX_GEN
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            static_params: StaticParams::default(),
            size: UnipolarFloat::ZERO,
            smoothing: Smoother::new(
                UnipolarFloat::new(0.25),
                Self::SMOOTH_SMOOTH_TIME,
                tunnels_lib::smooth::SmoothMode::Linear,
            ),
            internal_clock: Default::default(),
            clock_source: None,
            use_audio_size: false,
            simplex_gen: get_simplex_gen(),
        }
    }
}

impl Animation {
    const SMOOTH_SMOOTH_TIME: Duration = Duration::from_millis(100);

    /// Return the current value of the internal animation size.
    pub fn size(&self) -> UnipolarFloat {
        self.size
    }

    /// Return the current value of the duty cycle.
    pub fn duty_cycle(&self) -> UnipolarFloat {
        self.static_params.duty_cycle
    }

    /// Return the current value of the smoothing parameter.
    pub fn smoothing(&self) -> UnipolarFloat {
        self.smoothing.target()
    }

    /// Return the current selected number of animation periods.
    pub fn n_periods(&self) -> u16 {
        self.static_params.n_periods
    }

    /// Return true if this animation has nonzero size.
    fn active(&self) -> bool {
        self.size > 0.0
    }

    fn phase(&self, external_clocks: &impl ClockStore) -> Phase {
        match self.clock_source {
            None => self.internal_clock.phase(),
            // A selected clock that no longer exists reads as the neutral default.
            Some(id) => external_clocks.phase(id).unwrap_or_default(),
        }
    }

    fn ticks(&self, external_clocks: &impl ClockStore) -> Ticks {
        match self.clock_source {
            None => self.internal_clock.ticks(),
            Some(id) => external_clocks.ticks(id).unwrap_or_default(),
        }
    }

    /// Return the clock's current rate, scaled into a bipolar float.
    pub fn clock_speed(&self) -> BipolarFloat {
        BipolarFloat::new(self.internal_clock.rate_coarse / ControllableClock::RATE_SCALE)
    }

    /// Set the clock's current rate, scaling by our scale factor.
    fn set_clock_speed(&mut self, speed: BipolarFloat) {
        self.internal_clock.rate_coarse = speed.val() * ControllableClock::RATE_SCALE;
    }

    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        // Reached whatever the amplitude, because it shapes the waveform rather
        // than driving it: an animation held at no size is one being set up,
        // and the controls turned while it is have to arrive. Its clock is a
        // different matter — an animation that is not showing has no time to
        // keep, and starts from the top when it is given a size.
        self.smoothing.update_state(delta_t);
        if self.active() {
            self.internal_clock.update_state(delta_t, audio_envelope);
        }
    }

    /// Resolve everything that is fixed for a frame, once, for a beam of the
    /// given shape.
    ///
    /// A render asks an animation for a value once per segment, or on a filled
    /// shape once per vertex — tens of thousands of times. Most of what
    /// answering that costs is the same every time: which clock is driving,
    /// what phase it is at, where the smoother has got to, and the amplitude
    /// the size, submaster and audio envelope multiply out to. None of it
    /// depends on where in the figure the question is being asked.
    ///
    /// The span is what the beam brings to that, and it is more than a number.
    /// A noise field is spread over it, so it decides both what an offset
    /// means and whether the field is worth tabulating before the walk begins
    /// — which is why anything that means to read what a beam is drawn with,
    /// a preview included, has to prepare against that beam's own span rather
    /// than one of its choosing.
    pub fn prepare(
        &self,
        external_clocks: &impl ClockStore,
        audio_envelope: UnipolarFloat,
        span: OffsetSpan,
    ) -> PreparedAnimation {
        let smoothing = self.smoothing.val();
        let phase_temporal = self.phase(external_clocks);
        let ticks = self.ticks(external_clocks);
        let tabulate = self.active()
            && matches!(self.static_params.waveform, Waveform::Noise)
            && self.static_params.n_periods > 0
            && span.is_tabulated();
        PreparedAnimation {
            static_params: self.static_params,
            phase_temporal,
            smoothing,
            ticks,
            scale: self.scale_value(external_clocks, audio_envelope, 1.0),
            simplex_gen: self.simplex_gen,
            noise: tabulate.then(|| {
                Arc::new(NoiseTable::build(
                    self.simplex_gen,
                    ticks as f64 + phase_temporal.val(),
                    f64::from(self.static_params.n_periods),
                    (1.0 - smoothing.val()) * span.extent(),
                ))
            }),
            active: self.active(),
        }
    }

    /// Scale a value by the amplitude factors: size, clock submaster, and audio
    /// envelope.
    fn scale_value(
        &self,
        external_clocks: &impl ClockStore,
        audio_envelope: UnipolarFloat,
        mut v: f64,
    ) -> f64 {
        v *= self.size.val();

        // scale this animation by submaster level if using external clock
        let mut use_audio_size = self.use_audio_size;
        if let Some(id) = self.clock_source {
            // A selected clock that no longer exists reads as the neutral default
            // (submaster 0 → this animation contributes nothing).
            v *= external_clocks
                .submaster_level(id)
                .unwrap_or_default()
                .val();
            use_audio_size =
                use_audio_size || external_clocks.use_audio_size(id).unwrap_or_default();
        }
        // scale this animation by audio envelope if set
        if use_audio_size {
            v *= audio_envelope.val();
        }

        v
    }

    /// Emit the current value of all controllable animator state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_animation_state_change(Waveform(self.static_params.waveform));
        emitter.emit_animation_state_change(Pulse(self.static_params.pulse));
        emitter.emit_animation_state_change(Standing(self.static_params.standing));
        emitter.emit_animation_state_change(Invert(self.static_params.invert));
        emitter.emit_animation_state_change(NPeriods(self.static_params.n_periods));
        emitter.emit_animation_state_change(Speed(self.clock_speed()));
        emitter.emit_animation_state_change(Size(self.size));
        emitter.emit_animation_state_change(DutyCycle(self.static_params.duty_cycle));
        emitter.emit_animation_state_change(Smoothing(self.smoothing.target()));
        emitter.emit_animation_state_change(ClockSource(self.clock_source));
        emitter.emit_animation_state_change(UseAudioSize(self.use_audio_size));
        emitter.emit_animation_state_change(UseAudioSpeed(self.internal_clock.use_audio));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
            SetClockSource(source) => {
                self.handle_state_change(StateChange::ClockSource(source), emitter);
            }
            TogglePulse => {
                self.static_params.pulse = !self.static_params.pulse;
                emitter.emit_animation_state_change(StateChange::Pulse(self.static_params.pulse));
            }
            ToggleStanding => {
                self.static_params.standing = !self.static_params.standing;
                emitter.emit_animation_state_change(StateChange::Standing(
                    self.static_params.standing,
                ));
            }
            ToggleInvert => {
                self.static_params.invert = !self.static_params.invert;
                emitter.emit_animation_state_change(StateChange::Invert(self.static_params.invert));
            }
            ToggleUseAudioSize => {
                self.use_audio_size = !self.use_audio_size;
                emitter.emit_animation_state_change(StateChange::UseAudioSize(self.use_audio_size));
            }
            ToggleUseAudioSpeed => {
                self.internal_clock.use_audio = !self.internal_clock.use_audio;
                emitter.emit_animation_state_change(StateChange::UseAudioSpeed(
                    self.internal_clock.use_audio,
                ));
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            Waveform(v) => self.static_params.waveform = v,
            Pulse(v) => self.static_params.pulse = v,
            Standing(v) => self.static_params.standing = v,
            Invert(v) => self.static_params.invert = v,
            NPeriods(v) => self.static_params.n_periods = v,
            Speed(v) => self.set_clock_speed(v),
            Size(v) => self.size = v,
            DutyCycle(v) => self.static_params.duty_cycle = v,
            Smoothing(v) => self.smoothing.set_target(v),
            ClockSource(v) => self.clock_source = v,
            UseAudioSize(v) => self.use_audio_size = v,
            UseAudioSpeed(v) => self.internal_clock.use_audio = v,
        };
        emitter.emit_animation_state_change(sc);
    }
}

#[derive(Debug, Clone)]
pub enum StateChange {
    Waveform(Waveform),
    Pulse(bool),
    Standing(bool),
    Invert(bool),
    NPeriods(u16),
    Speed(BipolarFloat),
    Size(UnipolarFloat),
    DutyCycle(UnipolarFloat),
    Smoothing(UnipolarFloat),
    ClockSource(Option<ClockIdx>),
    UseAudioSize(bool),
    UseAudioSpeed(bool),
}

#[derive(Debug, Clone)]
pub enum ControlMessage {
    Set(StateChange),
    /// Set the clock that drives this animation, or `None` for the internal clock.
    SetClockSource(Option<ClockIdx>),
    TogglePulse,
    ToggleStanding,
    ToggleInvert,
    ToggleUseAudioSize,
    ToggleUseAudioSpeed,
}

pub trait EmitStateChange {
    fn emit_animation_state_change(&mut self, sc: StateChange);
}

/// How many units of noise a figure spreads across itself.
///
/// A run of segments spends one unit per segment, so a hundred-odd of them
/// reach that many. A figure has no segments to count, so it is given a span of
/// its own in the same units, chosen to decorrelate across it about as much as
/// a run of segments does along its length.
pub const NOISE_SPREAD: f64 = 64.0;

/// Where along the axis an animation decorrelates across a point sits.
///
/// Noise reads this as its second coordinate: two points a whole unit apart
/// take independent values, and two points sharing one take the same value.
/// What a unit means is the difference between the two kinds of beam, which is
/// why this is a type rather than a number — a run of segments counts them,
/// and a figure spreads a fixed span over itself however finely it is divided.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Default)]
pub struct SpreadOffset(f64);

impl SpreadOffset {
    /// The offset of one segment of a run of them.
    pub fn segment(index: usize) -> Self {
        Self(index as f64)
    }

    /// The offset of a point on a figure, from a coordinate running 0 to 1
    /// across it.
    ///
    /// Taking a coordinate rather than an index is what keeps a figure's noise
    /// the figure's: a point halfway across reads the same value however many
    /// vertices the figure was divided into, so the density a mesh was cut at
    /// does not reach the noise.
    pub fn across_figure(t: f64) -> Self {
        Self(t * NOISE_SPREAD)
    }

    /// The offset itself, in those units.
    pub fn val(self) -> f64 {
        self.0
    }
}

/// How far a beam's offset axis runs, and how it is divided.
///
/// A beam resolves an animation at a set of places along itself, and the offset
/// says where each of them sits. The two kinds of beam answer that differently
/// — a run of segments counts its segments, and a figure spreads a fixed span
/// across itself — so a caller that means to sweep the axis, or to tabulate
/// anything over it, has to be told which it is looking at.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OffsetSpan {
    /// A run of this many segments, one unit apart.
    Segments(u8),
    /// A figure, over which [`NOISE_SPREAD`] units are spread.
    Figure,
}

impl Default for OffsetSpan {
    fn default() -> Self {
        Self::Segments(0)
    }
}

impl OffsetSpan {
    /// How far the axis runs, in the units an offset is measured in.
    pub fn extent(self) -> f64 {
        match self {
            Self::Segments(count) => f64::from(count),
            Self::Figure => NOISE_SPREAD,
        }
    }

    /// How many segments the beam is drawn as.
    ///
    /// A figure is drawn as none. It is a continuum rather than a run of
    /// places, so there is nothing to count and nowhere a point of its own
    /// would go.
    pub fn segment_count(self) -> usize {
        match self {
            Self::Segments(count) => usize::from(count),
            Self::Figure => 0,
        }
    }

    /// The offset a given fraction of the way along the beam.
    ///
    /// A run of segments has places rather than a continuum, so a fraction
    /// lands on the segment it falls inside and every point of that segment
    /// answers alike. A figure is continuous and takes the fraction itself.
    pub fn at(self, along: UnipolarFloat) -> SpreadOffset {
        match self {
            Self::Segments(count) => {
                SpreadOffset::segment((along.val() * f64::from(count)) as usize)
            }
            Self::Figure => SpreadOffset::across_figure(along.val()),
        }
    }

    /// Whether a beam of this shape reads noise at more places than tabulating
    /// it would cost.
    ///
    /// A figure is resolved at every vertex of a mesh whose density comes from
    /// screen pixels — tens of thousands of them, and more on a larger frame —
    /// so a table bounds what it costs and keeps that cost off the resolution.
    /// A run of segments is resolved once per segment, which is fewer places
    /// than the table would have entries, so it reads the field directly: a
    /// table there would be more work than it saves, and would put a hundred-odd
    /// values that shows are built on behind an interpolation.
    ///
    /// **This has to be answered from the beam and nowhere else.** A table and
    /// the field it stands for are two ways of answering the same question, and
    /// they agree only to within what interpolating costs — so every consumer
    /// of an animation has to take the same one. Deriving the choice from the
    /// span means a preview and the wall arrive at it alike without either
    /// knowing about the other. Deciding it at each site instead, however
    /// simply, is what would let them drift.
    fn is_tabulated(self) -> bool {
        matches!(self, Self::Figure)
    }
}

/// Samples per unit of noise in a tabulated animation.
///
/// Simplex features run about a unit across, so two samples a unit is the
/// coarsest that represents one at all. From there the error falls as the
/// square of the spacing, which is what a field this smooth interpolated
/// bilinearly is worth: measured against the field itself across every
/// periodicity and smoothing a control can reach, two samples a unit sits 0.86
/// from it, four 0.31, eight 0.082, sixteen 0.023 — on a waveform whose own
/// range is two.
///
/// Eight is where that stops being the largest error in the picture and where
/// the table is still cheap. The offset axis alone spans [`NOISE_SPREAD`]
/// units, so each doubling from here doubles the rows, and
/// [`NOISE_TABLE_TOLERANCE`] is the other end of the same choice.
const SAMPLES_PER_NOISE_UNIT: f64 = 8.0;

/// How far a tabulated animation may sit from the noise it stands for.
///
/// Held beside the density that buys it, because a change to one without the
/// other is a change to what a table promises.
pub const NOISE_TABLE_TOLERANCE: f64 = 0.1;

/// Samples along an axis that spans no noise at all.
///
/// An axis of no extent still needs two samples for an interpolation to have
/// something to sit between; both hold the same value, so what it reads is that
/// value everywhere.
const MIN_TABLE_SAMPLES: usize = 2;

/// One frame of an animation's noise, tabulated over the beam it is drawn on.
///
/// The field is read on a grid and interpolated between, so what an animation
/// costs follows the noise it actually spans rather than how many places the
/// beam is divided into. Everything that grows without bound — the elapsed
/// ticks the field drifts along — is spent building the table, so what a
/// reader hands back is bounded by the beam.
///
/// Holds the noise itself and not the waveform: the duty cycle gates and a
/// pulse's knees are applied to what comes out, where they stay as sharp as
/// they are asked for rather than being blurred across a cell.
struct NoiseTable {
    /// Samples in row-major order, a row per offset and `cols` to a row.
    samples: Vec<f64>,
    cols: usize,
    rows: usize,
    /// How far the phase axis runs, in noise units.
    phase_extent: f64,
    /// How far the offset axis runs, in noise units.
    offset_extent: f64,
}

impl fmt::Debug for NoiseTable {
    /// Names the table's shape rather than its contents, which run to tens of
    /// thousands of samples.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoiseTable")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("phase_extent", &self.phase_extent)
            .field("offset_extent", &self.offset_extent)
            .finish()
    }
}

impl NoiseTable {
    /// Tabulate a rectangle of `field`, starting at `phase_base` along the
    /// axis it drifts on.
    fn build(field: &Simplex, phase_base: f64, phase_extent: f64, offset_extent: f64) -> Self {
        let cols = Self::samples_across(phase_extent);
        let rows = Self::samples_across(offset_extent);
        let mut samples = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            let y = offset_extent * row as f64 / (rows - 1) as f64;
            for col in 0..cols {
                let x = phase_extent * col as f64 / (cols - 1) as f64;
                samples.push(field.get([phase_base + x, y]));
            }
        }
        Self {
            samples,
            cols,
            rows,
            phase_extent,
            offset_extent,
        }
    }

    /// How many samples an axis spanning `extent` noise units is given.
    fn samples_across(extent: f64) -> usize {
        let spanned = (SAMPLES_PER_NOISE_UNIT * extent).ceil();
        // A NaN or an unbounded extent saturates rather than wrapping, and the
        // minimum then puts it on a real count.
        (spanned as usize).saturating_add(1).max(MIN_TABLE_SAMPLES)
    }

    /// The field at a point of the rectangle, interpolated between the four
    /// samples around it.
    ///
    /// A point outside the rectangle reads its edge. Nothing on a beam asks for
    /// one — the extents are what the beam spans — but a coordinate arriving
    /// from geometry is not worth trusting to the last bit during a show.
    fn sample(&self, phase: f64, offset: f64) -> f64 {
        let (col, along) = Self::cell(phase, self.phase_extent, self.cols);
        let (row, down) = Self::cell(offset, self.offset_extent, self.rows);
        let top = self.row(row);
        let bottom = self.row(row + 1);
        let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
        lerp(
            lerp(top[col], top[col + 1], along),
            lerp(bottom[col], bottom[col + 1], along),
            down,
        )
    }

    /// Which cell of an axis a coordinate falls in, and how far across it.
    ///
    /// The cell is always one the axis has, so the pair either side of it is
    /// always there to interpolate between.
    fn cell(at: f64, extent: f64, samples: usize) -> (usize, f64) {
        let cells = samples - 1;
        // An extent of nothing is one cell wide with both ends equal, so
        // anywhere in it reads the same value.
        if extent <= 0.0 {
            return (0, 0.0);
        }
        let scaled = (at / extent * cells as f64).clamp(0.0, cells as f64);
        let cell = (scaled as usize).min(cells - 1);
        (cell, scaled - cell as f64)
    }

    /// One row of samples, clamped to the rows there are.
    fn row(&self, row: usize) -> &[f64] {
        let row = row.min(self.rows - 1);
        &self.samples[row * self.cols..(row + 1) * self.cols]
    }
}

/// An animation with everything that is constant for a frame already resolved.
///
/// Holds no reference to the animation it came from, so a render can prepare
/// its animations once and then walk a figure without borrowing anything.
#[derive(Clone, Debug)]
pub struct PreparedAnimation {
    static_params: StaticParams,
    /// Where the driving clock has got to.
    phase_temporal: Phase,
    /// The smoother's current value, not its target.
    smoothing: UnipolarFloat,
    /// Whole periods elapsed, which noise uses to drift its field.
    ticks: Ticks,
    /// Size, clock submaster and audio envelope, multiplied out.
    scale: f64,
    simplex_gen: &'static Simplex,
    /// The frame's noise, where the beam reads it at more places than
    /// tabulating it costs.
    noise: Option<Arc<NoiseTable>>,
    /// A zero-size animation contributes nothing and skips the waveform.
    active: bool,
}

impl PreparedAnimation {
    /// Whether this animation contributes anything.
    ///
    /// A zero-size animation answers zero everywhere, so a caller that would
    /// otherwise ask it once per point can drop it instead.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The parameters that hold for as long as the frame does.
    pub fn static_params(&self) -> StaticParams {
        self.static_params
    }

    /// Where the driving clock has got to.
    pub fn phase_temporal(&self) -> Phase {
        self.phase_temporal
    }

    /// The smoother's current value, not its target.
    pub fn smoothing(&self) -> UnipolarFloat {
        self.smoothing
    }

    /// What the amplitude factors — size, clock submaster and audio envelope —
    /// multiply out to.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Whether the value depends on where along a coordinate it is asked.
    ///
    /// A periodicity of zero holds the spatial phase at zero for every
    /// waveform, noise included, so the animation answers one number for the
    /// whole frame; a constant waveform answers one number whatever the
    /// periodicity. A caller that would otherwise resolve it across a
    /// coordinate can resolve it once, and one that sizes a table by how
    /// finely the answer varies needs no table at all.
    pub fn varies_in_space(&self) -> bool {
        self.active
            && self.static_params.n_periods > 0
            && self.static_params.waveform.varies_with_phase()
    }

    /// Whether the value depends on where along the offset axis it is asked.
    ///
    /// Only noise reads that axis, and only while its samples are not held
    /// together: at full smoothing every point of a beam takes the same noise,
    /// and with no periodicity the offset is held at zero whatever it was
    /// given. A caller that pays for the coordinate the offset is measured
    /// along can skip it when nothing reads it.
    pub fn varies_across_spread(&self) -> bool {
        self.active
            && matches!(self.static_params.waveform, Waveform::Noise)
            && self.static_params.n_periods > 0
            && self.smoothing < UnipolarFloat::ONE
    }

    /// The animation's value at a point, with amplitude applied.
    pub fn value(&self, spatial_phase_offset: Phase, offset: SpreadOffset) -> f64 {
        if !self.active {
            return 0.;
        }
        self.unit_value(spatial_phase_offset, offset) * self.scale
    }

    /// Scale a value by the amplitude factors: size, clock submaster, and audio
    /// envelope.
    pub fn scale_value(&self, v: f64) -> f64 {
        v * self.scale
    }

    /// The waveform's own value, before amplitude.
    pub fn unit_value(&self, spatial_phase_offset: Phase, offset: SpreadOffset) -> f64 {
        let result = match self.static_params.waveform {
            Waveform::Sine => waveforms::sine(&self.waveform_args(spatial_phase_offset)),
            Waveform::Square => waveforms::square(&self.waveform_args(spatial_phase_offset)),
            Waveform::Sawtooth => waveforms::sawtooth(&self.waveform_args(spatial_phase_offset)),
            Waveform::Triangle => waveforms::triangle(&self.waveform_args(spatial_phase_offset)),
            Waveform::Noise => {
                // Handle duty cycle - this is a bit odd compared to waveforms,
                // since noise isn't periodic. Rather than trying to compress
                // the waveform to maintain the waveshape, we just turn off
                // the animation for a portion of each cycle.
                let spatial_phase =
                    spatial_phase_offset.val() * self.static_params.n_periods as f64;
                let temporal_phase = self.phase_temporal.val();

                if Phase::new(spatial_phase + temporal_phase) > self.static_params.duty_cycle
                    || self.static_params.duty_cycle == 0.0
                {
                    return 0.0;
                }

                // Use smoothing parameter as a "cross-correlation" term;
                // increased smoothing means a smaller Y-offset between
                // samples. Smoothing of zero offsets each sample by a full
                // interval, which should produce fairly uncorrelated noise
                // for different offsets.
                // Always use a Y-offset of 0 in periodicity of 0 to preserve
                // the expected behavior.
                //
                // Because of the smooth 2D landscape, smoothing parameters
                // modestly lower than 1 tend to look similar to an
                // increase in periodicity.
                let y_offset = if self.static_params.n_periods == 0 {
                    0.0
                } else {
                    (1.0 - self.smoothing.val()) * offset.val()
                };

                // A tabulated animation was built over exactly this rectangle
                // of the field, with the elapsed ticks already spent, so what
                // it is handed here is the beam's own coordinates and stays
                // bounded by the beam.
                let val = match &self.noise {
                    Some(table) => table.sample(spatial_phase, y_offset),
                    None => self
                        .simplex_gen
                        .get([self.ticks as f64 + spatial_phase + temporal_phase, y_offset]),
                };

                if self.static_params.pulse {
                    gate_noise(val)
                } else {
                    val
                }
            }
            Waveform::Constant => 1.0,
        };

        if self.static_params.invert {
            -result
        } else {
            result
        }
    }

    #[inline(always)]
    fn waveform_args(&self, spatial_phase_offset: Phase) -> WaveformArgs {
        WaveformArgs {
            phase_spatial: spatial_phase_offset * (self.static_params.n_periods as f64),
            phase_temporal: self.phase_temporal,
            smoothing: self.smoothing,
            duty_cycle: self.static_params.duty_cycle,
            pulse: self.static_params.pulse,
            standing: self.static_params.standing,
        }
    }
}

/// Where a noise pulse stops being dark, as a magnitude of the noise.
///
/// The magnitude of simplex noise is close to uniform across the lower part of
/// its range, so where this knee sits in that range is directly how much of the
/// time a pulse rests: about a seventh of the way up it, and so dark about a
/// seventh of the time.
const NOISE_GATE_LOW: f64 = 0.09;

/// Where a noise pulse reaches full, as a magnitude of the noise.
///
/// Placed so that about the top twentieth of noise magnitudes saturate. Noise
/// approaches the end of its own range too rarely to arrive at full any other
/// way, and the flat top that arriving costs is worth no more of the period
/// than it takes.
const NOISE_GATE_HIGH: f64 = 0.71;

/// Shape bipolar noise into a unipolar pulse.
///
/// Noise reaches the ends of its own range only rarely, so a curve that merely
/// rescales it settles into a dim middle: never resolving to black, never
/// arriving at full. The gate is flat at both ends instead — dark below one
/// knee, full above the other — which is what buys a pulse that starts and
/// ends at rest and a peak that lands rather than approaches.
///
/// Between the knees it is smoothstepped, so brightness enters and leaves a
/// pulse with no kink. The dark zone also holds the fold that taking a
/// magnitude puts at zero, keeping that corner off the sloped part of the
/// curve.
fn gate_noise(v: f64) -> f64 {
    let t = ((v.abs() - NOISE_GATE_LOW) / (NOISE_GATE_HIGH - NOISE_GATE_LOW)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::clock_bank::ClockBank;

    /// What the first few segments of a run read, taken before a figure had a
    /// coordinate of its own.
    const PINNED_SEGMENT_NOISE: [f64; 4] = [
        0.0,
        -0.2650758166508769,
        -0.26703366624431013,
        -0.26222119586893344,
    ];

    /// A noise animation shaped by `n_periods` and `smoothing`, resolved
    /// against `span`.
    fn noise_animation(n_periods: u16, smoothing: f64, span: OffsetSpan) -> PreparedAnimation {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        let mut animation = Animation::default();
        for sc in [
            StateChange::Waveform(Waveform::Noise),
            StateChange::NPeriods(n_periods),
            StateChange::Size(UnipolarFloat::ONE),
            StateChange::Smoothing(UnipolarFloat::new(smoothing)),
        ] {
            animation.control(ControlMessage::Set(sc), &mut Noop);
        }
        // Smoothing is reached over time rather than set, and the animation
        // runs at no speed, so nothing else moves while it gets there.
        animation.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        animation.prepare(&ClockBank::default(), UnipolarFloat::ZERO, span)
    }

    /// A tabulated animation stands for the field it tabulates.
    ///
    /// Everything a figure is drawn with comes off the table, so how far it can
    /// sit from the noise it was built from is the whole of what tabulating
    /// costs in the picture. Read against the same animation resolved without a
    /// table, across every periodicity and smoothing a control can reach.
    #[test]
    fn a_tabulated_animation_stands_for_the_noise_it_tabulates() {
        // Finer than the table on both axes, so the reading falls between
        // samples rather than on them, which is where interpolation is worst.
        let steps = 237;
        let mut worst: f64 = 0.0;
        for n_periods in [1u16, 3, 8, 15] {
            for smoothing in [0.0, 0.5, 0.9] {
                let tabulated = noise_animation(n_periods, smoothing, OffsetSpan::Figure);
                assert!(
                    tabulated.noise.is_some(),
                    "a figure's noise was not tabulated, so this compared the field with itself"
                );
                // A run of segments reads the field directly, which is what
                // gives the same animation an untabulated reading to be held
                // against.
                let direct = noise_animation(n_periods, smoothing, OffsetSpan::Segments(1));

                for i in 0..=steps {
                    for j in 0..=steps {
                        let phase = Phase::new(i as f64 / (steps + 1) as f64);
                        let offset = SpreadOffset::across_figure(j as f64 / steps as f64);
                        let error = (tabulated.unit_value(phase, offset)
                            - direct.unit_value(phase, offset))
                        .abs();
                        assert!(
                            error <= NOISE_TABLE_TOLERANCE,
                            "a table of noise over {n_periods} periods at a smoothing of \
                             {smoothing} sat {error} from the field it stands for"
                        );
                        worst = worst.max(error);
                    }
                }
            }
        }
        // The tolerance is what the sample density was chosen to buy, so it has
        // to be reached and not merely respected: one that nothing approaches
        // would let the density fall a long way before anything noticed.
        assert!(
            worst > NOISE_TABLE_TOLERANCE / 4.0,
            "the worst a table sat from its field was {worst}, so far inside the \
             {NOISE_TABLE_TOLERANCE} it promises that the promise says nothing"
        );
    }

    /// A run of segments reads the noise field where it always did.
    ///
    /// Segments were never the thing tessellation moved — a segment index is
    /// the beam's own and does not follow a screen — so nothing about how a
    /// figure is answered is allowed to reach them. Every show that exists is
    /// built on these numbers.
    #[test]
    fn noise_on_a_run_of_segments_is_where_it_has_always_been() {
        let anim = noise_animation(3, 0.25, OffsetSpan::Segments(126));
        assert!(
            anim.noise.is_none(),
            "a run of segments was answered from a table rather than the field"
        );
        let smoothing = anim.smoothing.val();

        for seg_num in [0usize, 1, 7, 64, 125] {
            let phase = Phase::new(seg_num as f64 / 126.0);
            // The coordinates a segment has always been asked at: the field
            // drifts along the elapsed periods and the phase, and steps across
            // by a segment at a time, held together by the smoothing.
            let expected = get_simplex_gen().get([
                anim.ticks as f64 + phase.val() * 3.0 + anim.phase_temporal.val(),
                (1.0 - smoothing) * seg_num as f64,
            ]);
            let value = anim.unit_value(phase, SpreadOffset::segment(seg_num));
            assert_eq!(
                value, expected,
                "segment {seg_num} of a run reads the noise field at a different place than it did"
            );
        }

        // And the numbers themselves, as they stood before a figure had a
        // coordinate of its own.
        let pinned: Vec<f64> = (0..4)
            .map(|seg_num| {
                anim.unit_value(
                    Phase::new(seg_num as f64 / 126.0),
                    SpreadOffset::segment(seg_num),
                )
            })
            .collect();
        for (value, pin) in pinned.iter().zip(PINNED_SEGMENT_NOISE) {
            assert!(
                (value - pin).abs() < 1e-12,
                "a run of segments reads {pinned:?} where it used to read \
                 {PINNED_SEGMENT_NOISE:?}"
            );
        }
    }

    /// A figure spreads a span of noise across itself, and the smoothing
    /// control is what decides how much of that span it spends.
    ///
    /// The spread is the whole reason a figure's coordinate is scaled rather
    /// than run from zero to one: a coordinate that spent under a unit of noise
    /// on the whole figure would hold every point of it at the same value, and
    /// a control that turned nothing would be worse than one that was missing.
    #[test]
    fn smoothing_decorrelates_a_figure_across_itself() {
        let across = |smoothing: f64| -> Vec<f64> {
            let anim = noise_animation(1, smoothing, OffsetSpan::Figure);
            (0..64)
                .map(|i| {
                    let t = i as f64 / 64.0;
                    anim.unit_value(Phase::new(t), SpreadOffset::across_figure(t))
                })
                .collect()
        };

        // Held together at full smoothing, the figure reads a single line of
        // the field and neighbouring points barely differ. Spent in full, they
        // are a step apart in it.
        let held = across(1.0);
        let spread = across(0.0);
        // How far the value moves from one point of the figure to the next,
        // which is what decorrelation means where it is looked at.
        let step = |values: &[f64]| {
            values.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (values.len() - 1) as f64
        };
        assert!(
            step(&spread) > 10.0 * step(&held),
            "an unsmoothed figure moved {} between neighbouring points against \
             the {} a fully smoothed one moves, so the control has nothing to spend",
            step(&spread),
            step(&held)
        );

        // Turning the control moves what every part of the figure is doing,
        // not merely how far the extremes reach.
        let moved = spread
            .iter()
            .zip(&held)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            moved > 0.5,
            "turning the smoothing control across a figure moved the waveform by \
             {moved}, which is nothing a viewer would see"
        );
    }

    /// A gated noise pulse rests at each end of the unipolar range rather than
    /// only approaching it, so the two knees are where it arrives, and outside
    /// them it holds still however far the noise goes on.
    #[test]
    fn a_noise_gate_rests_at_both_ends_of_its_range() {
        for v in [0.0, NOISE_GATE_LOW / 2.0, NOISE_GATE_LOW, -NOISE_GATE_LOW] {
            assert_eq!(
                gate_noise(v),
                0.0,
                "noise of {v} lit a pulse from inside the dark zone"
            );
        }
        let above = (NOISE_GATE_HIGH + 1.0) / 2.0;
        for v in [NOISE_GATE_HIGH, -NOISE_GATE_HIGH, above, -above, 1.0, -1.0] {
            assert_eq!(
                gate_noise(v),
                1.0,
                "noise of {v} fell short of full from above the high knee"
            );
        }
    }

    /// The gate reads a magnitude, so noise displaced either way lights a pulse
    /// the same amount, and a pulse rises without pause between its knees.
    #[test]
    fn a_noise_gate_is_symmetric_and_rises_between_its_knees() {
        let step = (NOISE_GATE_HIGH - NOISE_GATE_LOW) / 64.0;
        let mut previous = 0.0;
        for i in 1..64 {
            let v = NOISE_GATE_LOW + step * i as f64;
            let value = gate_noise(v);
            assert_eq!(
                value,
                gate_noise(-v),
                "noise of {v} and of its negation lit a pulse differently"
            );
            assert!(
                value > previous,
                "a pulse stalled at {value} between its knees, at noise of {v}"
            );
            assert!(
                (0.0..=1.0).contains(&value),
                "noise of {v} lit a pulse to {value}, outside the unipolar range"
            );
            previous = value;
        }
    }

    /// A pulse leaves and reaches rest smoothly, so brightness has no kink at
    /// either knee where a viewer would read it as a corner in the light.
    #[test]
    fn a_noise_gate_has_no_kink_at_either_knee() {
        let span = NOISE_GATE_HIGH - NOISE_GATE_LOW;
        let step = span / 4096.0;
        // The slope just inside a knee, against the slope of a straight ramp
        // between the knees, which is what the gate would have if it did not
        // ease in and out.
        let ramp = step / span;
        for knee in [NOISE_GATE_LOW, NOISE_GATE_HIGH] {
            let inside = if knee == NOISE_GATE_LOW { step } else { -step };
            let slope = (gate_noise(knee + inside) - gate_noise(knee)).abs();
            assert!(
                slope < ramp / 100.0,
                "a pulse turned a corner at the {knee} knee: it moved {slope} \
                 over a step a straight ramp would move {ramp}"
            );
        }
    }

    /// A control that shapes an animation is reached over time rather than set,
    /// and it has to be reached whether or not the animation is currently
    /// showing. An animation at no amplitude is still one an operator is
    /// looking at while they set it up — the waveform it would draw is exactly
    /// what a preview is for — so a control turned then must arrive, not sit
    /// where it was until the animation is given a size.
    #[test]
    fn a_shaping_control_is_reached_at_any_amplitude() {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        let clocks = crate::clock_bank::ClockBank::default();

        for size in [UnipolarFloat::ZERO, UnipolarFloat::ONE] {
            let mut animation = Animation::default();
            let set = |a: &mut Animation, sc| a.control(ControlMessage::Set(sc), &mut Noop);
            set(&mut animation, StateChange::Size(size));
            set(
                &mut animation,
                StateChange::Smoothing(UnipolarFloat::new(0.75)),
            );

            // Longer than the control takes to be reached.
            animation.update_state(Duration::from_millis(500), UnipolarFloat::ZERO);

            let reached = animation
                .prepare(&clocks, UnipolarFloat::ZERO, OffsetSpan::default())
                .smoothing;
            assert_eq!(
                reached,
                animation.smoothing(),
                "at a size of {}, smoothing stalled at {} short of the {} it was turned to",
                size.val(),
                reached.val(),
                animation.smoothing().val()
            );
        }
    }

    /// Whether an animation varies in space is what decides how finely a
    /// caller has to resolve it, so the two ways of answering "not at all" —
    /// no periodicity and no amplitude — both have to read that way.
    #[test]
    fn periodicity_is_what_makes_an_animation_vary_in_space() {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        let prepare = |waveform: Waveform, n_periods: u16, size: f64| {
            let mut animation = Animation::default();
            animation.control(
                ControlMessage::Set(StateChange::Waveform(waveform)),
                &mut Noop,
            );
            animation.control(
                ControlMessage::Set(StateChange::NPeriods(n_periods)),
                &mut Noop,
            );
            animation.control(
                ControlMessage::Set(StateChange::Size(UnipolarFloat::new(size))),
                &mut Noop,
            );
            animation.prepare(
                &ClockBank::default(),
                UnipolarFloat::ZERO,
                OffsetSpan::default(),
            )
        };

        assert!(prepare(Waveform::Sine, 1, 1.0).varies_in_space());
        assert!(
            !prepare(Waveform::Sine, 0, 1.0).varies_in_space(),
            "no periodicity is one value everywhere"
        );
        assert!(
            !prepare(Waveform::Sine, 1, 0.0).varies_in_space(),
            "no amplitude is zero everywhere"
        );
        assert!(
            !prepare(Waveform::Constant, 1, 1.0).varies_in_space(),
            "a constant ignores the phase it is asked at, however many periods it is given"
        );
    }
}
