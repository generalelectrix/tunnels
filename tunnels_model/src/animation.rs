use crate::animation_target::AnimationTarget;
use crate::clock::Clock;
use crate::clock::ControllableClock;
use crate::clock::Ticks;
use crate::clock_bank::ClockStore;
use crate::noise::{self, NoiseBase};
use crate::spectrum::SpectrumTables;
use crate::waveforms::WaveformArgs;
use crate::{clock_bank::ClockIdx, waveforms};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use strum::VariantArray;
use tunnels_lib::audio::AudioState;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};
use tunnels_lib::smooth::Smoothed;

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq, Eq, VariantArray)]
pub enum Waveform {
    SineSquare,
    TriSaw,
    Noise,
    /// The audio spectrum's band levels, folded out and back across a period.
    ///
    /// Pulsed, a band reads its level; unpulsed, the level is rescaled to the
    /// bipolar range, so a silent band reads -1. Always carried by the clock:
    /// standing does not reach it.
    Spectrum,
    Constant,
}

impl Waveform {
    /// Whether the value depends on the phase it is asked at.
    ///
    /// A constant answers one number for every phase, so a caller that would
    /// otherwise resolve it across a coordinate can resolve it once.
    pub fn varies_with_phase(self) -> bool {
        match self {
            Self::SineSquare | Self::TriSaw | Self::Noise | Self::Spectrum => true,
            Self::Constant => false,
        }
    }
}

/// The animation parameters that are fixed for the duration of a frame.
///
/// These are used as given. The rest of what an animation's value depends on —
/// clock phase, elapsed ticks, where its smoothed controls have got to, the
/// amplitude — has to be resolved before it can be read, and so is held apart
/// from these.
#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub struct StaticParams {
    pub waveform: Waveform,
    pub pulse: bool,
    pub standing: bool,
    pub invert: bool,
    pub n_periods: u16,
}

impl Default for StaticParams {
    fn default() -> Self {
        Self {
            waveform: Waveform::SineSquare,
            pulse: false,
            standing: false,
            invert: false,
            n_periods: 1,
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
    pub fn prepare<'f>(
        &self,
        external_clocks: &impl ClockStore,
        audio: &AudioState,
        spectrum: &'f SpectrumTables,
    ) -> TargetedAnimation<PreparedAnimation<'f>> {
        TargetedAnimation {
            animation: self.animation.prepare(external_clocks, audio, spectrum),
            target: self.target,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Animation {
    static_params: StaticParams,
    size: Smoothed<UnipolarFloat>,
    duty_cycle: Smoothed<UnipolarFloat>,
    smoothing: Smoothed<UnipolarFloat>,
    internal_clock: Clock,
    clock_source: Option<ClockIdx>,
    use_audio_size: bool,
}

/// How much the noise line lengthens as smoothing falls, to
/// `periods × (1 + this × (1 − smoothing))`, so a rough setting is sharp and
/// choppy rather than a slow wobble.
const NOISE_ROUGHENING: f64 = 7.0;

/// An animation's controls and clock as they stand for one frame.
#[derive(Clone, Copy)]
struct FrameControls {
    params: StaticParams,
    smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    /// Where the driving clock is within its period.
    clock: Phase,
    /// Whole periods the driving clock has run.
    ticks: Ticks,
    /// Size, clock submaster and audio envelope, multiplied out.
    scale: f64,
}

/// Noise for one frame: the line through the field that the phase runs along,
/// and what turns a point on it into a value.
///
/// Smoothing turns the line and lengthens it (see [`NOISE_ROUGHENING`]): at one
/// it runs with time, so the clock carries a pattern along the phase, and at
/// zero it stands across time, so the noise boils in place. No periodicity is a
/// line of no length, one value everywhere.
///
/// The field drifts along its first axis with the clock, resolved here once.
/// Every point is then `f32` arithmetic on small numbers, whether it is asked
/// for alone or in a batch, and both give the same bits.
#[derive(Clone, Copy, Debug)]
struct NoiseLine {
    /// Where a phase of zero sits in the field.
    base: NoiseBase,
    /// How far through the field a phase of one carries a point.
    reach: [f32; 2],
    /// Spatial periods across a phase of one.
    periods: f32,
    /// Where the clock is within its period.
    clock: f32,
    /// The furthest place in a cycle that the duty cycle lights; before every
    /// place in a cycle at a duty cycle of zero.
    lit_up_to: f32,
    pulse: bool,
    /// What a value is multiplied by once amplitude and inversion are applied.
    amplitude: f32,
}

impl NoiseLine {
    fn new(frame: FrameControls) -> Self {
        let rough = 1.0 - frame.smoothing.val();
        let angle = rough * std::f64::consts::FRAC_PI_2;
        let length = f64::from(frame.params.n_periods) * (1.0 + NOISE_ROUGHENING * rough);
        let polarity = if frame.params.invert { -1.0 } else { 1.0 };
        Self {
            base: NoiseBase::new(frame.ticks as f64 + frame.clock.val(), 0.0),
            reach: [(length * angle.cos()) as f32, (length * angle.sin()) as f32],
            periods: f32::from(frame.params.n_periods),
            clock: frame.clock.val() as f32,
            lit_up_to: if frame.duty_cycle == 0.0 {
                -1.0
            } else {
                frame.duty_cycle.val() as f32
            },
            pulse: frame.params.pulse,
            amplitude: (polarity * frame.scale) as f32,
        }
    }

    /// The value at `phase`, in [0, 1], multiplied by `gain`.
    #[inline]
    fn value(&self, phase: f32, gain: f32) -> f32 {
        if self.pulse {
            self.at::<true>(phase, gain)
        } else {
            self.at::<false>(phase, gain)
        }
    }

    /// Write the value at each of `phases`, each in [0, 1], at this line's
    /// amplitude, into the same place in `out`.
    #[inline]
    fn fill(&self, phases: impl Iterator<Item = f32>, out: &mut [f32]) {
        if self.pulse {
            self.fill_shaped::<true>(phases, out);
        } else {
            self.fill_shaped::<false>(phases, out);
        }
    }

    #[inline]
    fn fill_shaped<const PULSE: bool>(&self, phases: impl Iterator<Item = f32>, out: &mut [f32]) {
        for (value, phase) in out.iter_mut().zip(phases) {
            *value = self.at::<PULSE>(phase, self.amplitude);
        }
    }

    /// Duty cycle is a bit odd compared to waveforms, since noise isn't
    /// periodic. Rather than trying to compress the waveform to maintain the
    /// waveshape, a point simply goes dark for a portion of each cycle.
    #[inline(always)]
    fn at<const PULSE: bool>(&self, phase: f32, gain: f32) -> f32 {
        let cycle = phase * self.periods + self.clock;
        if cycle - cycle.floor() > self.lit_up_to {
            return 0.0;
        }
        let sample = self
            .base
            .sample(phase * self.reach[0], phase * self.reach[1]);
        let shaped = if PULSE { noise::gate(sample) } else { sample };
        shaped * gain
    }
}

/// A waveform, with whatever it needs worked out for one frame.
#[derive(Clone, Copy, Debug)]
enum PreparedWaveform {
    SineSquare,
    TriSaw,
    /// Noise, with the line through its field that it is sampled along.
    Noise(NoiseLine),
    Spectrum,
    Constant,
}

impl PreparedWaveform {
    fn new(frame: FrameControls) -> Self {
        match frame.params.waveform {
            Waveform::SineSquare => Self::SineSquare,
            Waveform::TriSaw => Self::TriSaw,
            Waveform::Noise => Self::Noise(NoiseLine::new(frame)),
            Waveform::Spectrum => Self::Spectrum,
            Waveform::Constant => Self::Constant,
        }
    }
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            static_params: StaticParams::default(),
            size: UnipolarFloat::ZERO.into(),
            duty_cycle: UnipolarFloat::ONE.into(),
            smoothing: UnipolarFloat::ONE.into(),
            internal_clock: Default::default(),
            clock_source: None,
            use_audio_size: false,
        }
    }
}

impl Animation {
    /// Return the value the size control is set to.
    pub fn size(&self) -> UnipolarFloat {
        self.size.target()
    }

    /// Return the value the duty cycle control is set to.
    pub fn duty_cycle(&self) -> UnipolarFloat {
        self.duty_cycle.target()
    }

    /// Return the value the smoothing control is set to.
    pub fn smoothing(&self) -> UnipolarFloat {
        self.smoothing.target()
    }

    /// Return the current selected number of animation periods.
    pub fn n_periods(&self) -> u16 {
        self.static_params.n_periods
    }

    /// Return true if this animation has nonzero size.
    fn active(&self) -> bool {
        self.size.smoothed() > 0.0
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

    pub fn update_state(&mut self, delta_t: Duration, audio: &AudioState) {
        // The controls are reached whatever the amplitude: an animation held at
        // no size is one being set up, and the controls turned while it is have
        // to arrive. Its clock is a different matter — an animation that is not
        // showing has no time to keep, and starts from the top when it is given
        // a size.
        self.size.update_state(delta_t);
        self.duty_cycle.update_state(delta_t);
        self.smoothing.update_state(delta_t);
        if self.active() {
            self.internal_clock.update_state(delta_t, audio);
        }
    }

    /// Bring every smoothed control to rest on its target at once.
    pub fn settle_controls(&mut self) {
        self.size.settle();
        self.duty_cycle.settle();
        self.smoothing.settle();
    }

    /// Resolve everything that is fixed for a frame, once.
    ///
    /// A render asks an animation for a value once per segment, or on a filled
    /// shape once per vertex — tens of thousands of times. Most of what
    /// answering that costs is the same every time: which clock is driving,
    /// what phase it is at, where the smoother has got to, and the amplitude
    /// the size, submaster and audio envelope multiply out to. None of it
    /// depends on where in the figure the question is being asked.
    pub fn prepare<'f>(
        &self,
        external_clocks: &impl ClockStore,
        audio: &AudioState,
        spectrum: &'f SpectrumTables,
    ) -> PreparedAnimation<'f> {
        let frame = FrameControls {
            params: self.static_params,
            smoothing: self.smoothing.smoothed(),
            duty_cycle: self.duty_cycle.smoothed(),
            clock: self.phase(external_clocks),
            ticks: self.ticks(external_clocks),
            scale: self.scale_value(external_clocks, audio, 1.0),
        };
        PreparedAnimation {
            static_params: frame.params,
            phase_temporal: frame.clock,
            duty_cycle: frame.duty_cycle,
            smoothing: frame.smoothing,
            waveform: PreparedWaveform::new(frame),
            scale: frame.scale,
            spectrum,
            active: self.active(),
        }
    }

    /// Scale a value by the amplitude factors: size, clock submaster, and audio
    /// envelope.
    fn scale_value(
        &self,
        external_clocks: &impl ClockStore,
        audio: &AudioState,
        mut v: f64,
    ) -> f64 {
        v *= self.size.smoothed().val();

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
            v *= audio.envelope().val();
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
        emitter.emit_animation_state_change(Size(self.size.target()));
        emitter.emit_animation_state_change(DutyCycle(self.duty_cycle.target()));
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
            Size(v) => self.size.set_target(v),
            DutyCycle(v) => self.duty_cycle.set_target(v),
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

/// An animation with everything that is constant for a frame already resolved.
///
/// Holds no reference to the animation it came from, so a render can prepare
/// its animations once and then walk a figure. The one thing it borrows is
/// the frame's spectrum.
#[derive(Clone, Copy, Debug)]
pub struct PreparedAnimation<'f> {
    static_params: StaticParams,
    /// Where the driving clock has got to.
    phase_temporal: Phase,
    /// The duty cycle control's current value, not its target.
    duty_cycle: UnipolarFloat,
    /// The smoothing control's current value, not its target.
    smoothing: UnipolarFloat,
    /// The waveform, resolved at that smoothing.
    waveform: PreparedWaveform,
    /// Size, clock submaster and audio envelope, multiplied out.
    scale: f64,
    /// The frame's spectrum, as the spectrum waveform reads it.
    spectrum: &'f SpectrumTables,
    /// A zero-size animation contributes nothing and skips the waveform.
    active: bool,
}

impl PreparedAnimation<'_> {
    /// Whether this animation contributes anything.
    ///
    /// A zero-size animation answers zero everywhere, so a caller that would
    /// otherwise ask it once per point can drop it instead.
    pub fn is_active(&self) -> bool {
        self.active
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

    /// The animation's value at a phase, with amplitude applied.
    pub fn value(&self, spatial_phase_offset: Phase) -> f64 {
        if !self.active {
            return 0.;
        }
        match &self.waveform {
            PreparedWaveform::Noise(line) => {
                f64::from(line.value(spatial_phase_offset.val() as f32, line.amplitude))
            }
            _ => self.unit_value(spatial_phase_offset) * self.scale,
        }
    }

    /// The animation's value at many points on a figure, with amplitude
    /// applied.
    ///
    /// `out[i]` is [`value`](Self::value) at the phase `alongs[i]`, narrowed to
    /// `f32`. As many values are written as the shorter of the two holds.
    pub fn values_on_figure(&self, alongs: &[f32], out: &mut [f32]) {
        match &self.waveform {
            PreparedWaveform::Noise(line) if self.active => {
                line.fill(alongs.iter().map(|&along| along - along.floor()), out);
            }
            _ => {
                for (value, &along) in out.iter_mut().zip(alongs) {
                    *value = self.value(Phase::new(f64::from(along))) as f32;
                }
            }
        }
    }

    /// The animation's value along a run of elements spread evenly across one
    /// unit of phase, with amplitude applied.
    ///
    /// The run is as long as `out`: element `i` of `n` sits at phase `i / n`,
    /// and `out[i]` is [`value`](Self::value) at that phase, narrowed to `f32`.
    pub fn values_on_elements(&self, out: &mut [f32]) {
        match &self.waveform {
            PreparedWaveform::Noise(line) if self.active => {
                let n = out.len() as f32;
                line.fill((0..out.len() as i32).map(|i| i as f32 / n), out);
            }
            _ => {
                let n = out.len() as f64;
                for (i, value) in out.iter_mut().enumerate() {
                    *value = self.value(Phase::new(i as f64 / n)) as f32;
                }
            }
        }
    }

    /// Scale a value by the amplitude factors: size, clock submaster, and audio
    /// envelope.
    pub fn scale_value(&self, v: f64) -> f64 {
        v * self.scale
    }

    /// The waveform's own value at a phase, before amplitude.
    pub fn unit_value(&self, spatial_phase_offset: Phase) -> f64 {
        let result = match self.waveform {
            PreparedWaveform::SineSquare => {
                waveforms::sine_square(&self.waveform_args(spatial_phase_offset))
            }
            PreparedWaveform::TriSaw => {
                waveforms::tri_saw(&self.waveform_args(spatial_phase_offset))
            }
            PreparedWaveform::Noise(line) => {
                f64::from(line.value(spatial_phase_offset.val() as f32, 1.0))
            }
            PreparedWaveform::Spectrum => {
                waveforms::spectrum(&self.waveform_args(spatial_phase_offset), self.spectrum)
            }
            PreparedWaveform::Constant => 1.0,
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
            duty_cycle: self.duty_cycle,
            pulse: self.static_params.pulse,
            standing: self.static_params.standing,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::clock_bank::ClockBank;
    use tunnels_lib::audio::{AudioFrame, UnipolarF32};

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
            animation.update_state(Duration::from_millis(500), &AudioState::default());

            let reached = animation
                .prepare(&clocks, &AudioState::default(), &SpectrumTables::SILENT)
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

    /// A control that shapes an animation glides to the value it is turned to,
    /// while what the animation reports back is that value from the moment it
    /// is set.
    #[test]
    fn shaping_controls_glide_to_the_value_they_report() {
        #[derive(Default)]
        struct Recorder(Vec<String>);
        impl EmitStateChange for Recorder {
            fn emit_animation_state_change(&mut self, sc: StateChange) {
                self.0.push(format!("{sc:?}"));
            }
        }
        let clocks = crate::clock_bank::ClockBank::default();
        /// A control, the value it sets, and how that value reads off a
        /// prepared animation.
        struct Glide {
            control: fn() -> StateChange,
            target: f64,
            rendered: fn(&PreparedAnimation) -> f64,
        }
        let controls: [Glide; 3] = [
            Glide {
                control: || StateChange::Size(UnipolarFloat::new(0.6)),
                target: 0.6,
                rendered: |p| p.scale,
            },
            Glide {
                control: || StateChange::DutyCycle(UnipolarFloat::new(0.3)),
                target: 0.3,
                rendered: |p| p.duty_cycle.val(),
            },
            Glide {
                control: || StateChange::Smoothing(UnipolarFloat::new(0.9)),
                target: 0.9,
                rendered: |p| p.smoothing.val(),
            },
        ];
        for Glide {
            control,
            target,
            rendered,
        } in controls
        {
            let mut animation = Animation::default();
            let prepared = |a: &Animation| {
                rendered(&a.prepare(&clocks, &AudioState::default(), &SpectrumTables::SILENT))
            };
            let before = prepared(&animation);
            animation.control(ControlMessage::Set(control()), &mut Recorder::default());
            assert_eq!(before, prepared(&animation), "{:?} jumped", control());

            let mut talkback = Recorder::default();
            animation.emit_state(&mut talkback);
            assert!(
                talkback.0.contains(&format!("{:?}", control())),
                "{:?} was not reported back: {:?}",
                control(),
                talkback.0
            );

            animation.update_state(Duration::from_millis(16), &AudioState::default());
            let gliding = prepared(&animation);
            assert!(
                gliding != before && gliding != target,
                "{:?} was at {gliding} after one tick",
                control()
            );

            animation.update_state(Duration::from_secs(1), &AudioState::default());
            assert_eq!(target, prepared(&animation), "{:?} did not land", control());
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
            animation.settle_controls();
            animation.prepare(
                &ClockBank::default(),
                &AudioState::default(),
                &SpectrumTables::SILENT,
            )
        };

        assert!(prepare(Waveform::SineSquare, 1, 1.0).varies_in_space());
        assert!(
            !prepare(Waveform::SineSquare, 0, 1.0).varies_in_space(),
            "no periodicity is one value everywhere"
        );
        assert!(
            !prepare(Waveform::SineSquare, 1, 0.0).varies_in_space(),
            "no amplitude is zero everywhere"
        );
        assert!(
            !prepare(Waveform::Constant, 1, 1.0).varies_in_space(),
            "a constant ignores the phase it is asked at, however many periods it is given"
        );
        assert!(prepare(Waveform::Spectrum, 1, 1.0).varies_in_space());
    }

    /// Noise is sampled along a line through its field that smoothing turns
    /// and lengthens. With no periodicity the line has no length, so noise is
    /// one value everywhere however it is smoothed. At full smoothing the line
    /// runs with time, so the clock carries the pattern along the phase; with
    /// none it stands across time, so the pattern boils in place instead. And
    /// the rougher it is set, the further the line reaches through the field:
    /// noise varies more along the phase at every step down in smoothing,
    /// without ever stepping from one point to the next.
    #[test]
    fn noise_roughens_along_its_line_as_smoothing_falls() {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        let running = |n_periods: u16, smoothing: f64| {
            let mut animation = Animation::default();
            for sc in [
                StateChange::Waveform(Waveform::Noise),
                StateChange::NPeriods(n_periods),
                StateChange::Size(UnipolarFloat::ONE),
                StateChange::Smoothing(UnipolarFloat::new(smoothing)),
                StateChange::Speed(BipolarFloat::new(0.3)),
            ] {
                animation.control(ControlMessage::Set(sc), &mut Noop);
            }
            animation.settle_controls();
            animation.update_state(Duration::from_secs(1), &AudioState::default());
            animation
        };
        let prepare = |animation: &Animation| {
            animation.prepare(
                &ClockBank::default(),
                &AudioState::default(),
                &SpectrumTables::SILENT,
            )
        };
        let phases: Vec<Phase> = (0..1000)
            .map(|i| Phase::new(f64::from(i) / 1000.0))
            .collect();
        let smoothings = [0.0, 0.25, 0.5, 0.75, 1.0];

        for smoothing in smoothings {
            let flat = prepare(&running(0, smoothing));
            let first = flat.value(phases[0]);
            assert!(
                phases.iter().all(|&phase| flat.value(phase) == first),
                "noise with no periodicity varied along the phase at smoothing {smoothing}"
            );
        }

        const PERIODS: u16 = 2;

        // After a moment's running, how far the noise anywhere has strayed from
        // the earlier pattern carried along the phase by as far as the clock ran.
        let left_behind = |smoothing: f64| -> f64 {
            let mut animation = running(PERIODS, smoothing);
            let clock = |a: &Animation| {
                let clocks = ClockBank::default();
                a.ticks(&clocks) as f64 + a.phase(&clocks).val()
            };
            let early = prepare(&animation);
            let started = clock(&animation);
            animation.update_state(Duration::from_millis(200), &AudioState::default());
            let late = prepare(&animation);
            let carried = (clock(&animation) - started) / f64::from(PERIODS);
            assert!(carried != 0.0, "the clock did not move");
            phases
                .iter()
                .filter_map(|&phase| {
                    let from = phase.val() + carried;
                    (0.0..1.0)
                        .contains(&from)
                        .then(|| (late.value(phase) - early.value(Phase::new(from))).abs())
                })
                .fold(0.0, f64::max)
        };
        let travelling = left_behind(1.0);
        let boiling = left_behind(0.0);
        assert!(
            travelling < 1e-5,
            "at full smoothing the clock moved noise {travelling} off the line it travels"
        );
        assert!(
            boiling > 0.1,
            "with no smoothing the clock only moved noise {boiling} off the line, as if \
             it travelled along it"
        );

        // How far the value travels along the phase, and its largest step
        // between neighbouring points.
        let walk = |smoothing: f64| -> (f64, f64) {
            let animation = prepare(&running(PERIODS, smoothing));
            let values: Vec<f64> = phases.iter().map(|&p| animation.value(p)).collect();
            values
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold((0.0, 0.0), |(total, largest), step| {
                    (total + step, f64::max(largest, step))
                })
        };
        let walks = smoothings.map(walk);
        for (pair, smoothing) in walks.windows(2).zip(smoothings.windows(2)) {
            let ((rougher, _), (smoother, _)) = (pair[0], pair[1]);
            assert!(
                rougher > smoother,
                "noise travelled {rougher} along the phase at smoothing {} against \
                 {smoother} at {}",
                smoothing[0],
                smoothing[1]
            );
        }
        let (calm, _) = walks[walks.len() - 1];
        let (rough, largest) = walks[0];
        // A line this many times longer crosses about as many times as much
        // of the field; half of that leaves room for what this stretch of the
        // field happens to hold.
        let lengthening = 1.0 + NOISE_ROUGHENING;
        assert!(
            rough > lengthening / 2.0 * calm,
            "rough noise travelled {rough} along the phase against {calm} smooth, on a \
             line {lengthening} times as long"
        );
        assert!(
            largest < 0.1,
            "rough noise stepped by {largest} between neighbouring points"
        );
    }

    /// Noise reads the field where its line puts a point, whichever settings
    /// put it there: the point at phase `p` sits `p` of the way along a line
    /// that smoothing turns and lengthens, from where the clock has got to. The
    /// duty cycle lights it where its place in a cycle is at most the duty
    /// cycle, and never at a duty cycle of zero. A lit point is its sample,
    /// optionally gated, negated if inverted, times the size.
    #[test]
    fn noise_reads_the_field_where_its_line_puts_a_point() {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        // Next to the duty cycle or the end of a cycle, f32 and f64 may light a
        // point differently.
        const EDGE: f64 = 1e-5;
        const TOLERANCE: f64 = 3e-4;
        let clocks = ClockBank::default();
        for (n_periods, smoothing, duty_cycle, pulse, invert) in [
            (16, 0.5, 1.0, false, false),
            (16, 0.5, 0.7, true, false),
            (4, 0.0, 0.3, false, true),
            (1, 1.0, 1.0, true, false),
            (0, 0.2, 0.5, false, false),
            (3, 0.9, 0.0, true, false),
            (64, 0.0, 0.95, false, true),
        ] {
            let mut animation = Animation::default();
            for sc in [
                StateChange::Waveform(Waveform::Noise),
                StateChange::NPeriods(n_periods),
                StateChange::Size(UnipolarFloat::new(0.8)),
                StateChange::Smoothing(UnipolarFloat::new(smoothing)),
                StateChange::DutyCycle(UnipolarFloat::new(duty_cycle)),
                StateChange::Pulse(pulse),
                StateChange::Invert(invert),
                StateChange::Speed(BipolarFloat::new(0.3)),
            ] {
                animation.control(ControlMessage::Set(sc), &mut Noop);
            }
            animation.settle_controls();
            // Far enough for the clock to be many periods along.
            for _ in 0..6000 {
                animation.update_state(Duration::from_millis(123), &AudioState::default());
            }
            let ticks = animation.ticks(&clocks);
            assert!(ticks.abs() > 10, "the clock only ran {ticks} ticks");
            let prepared =
                animation.prepare(&clocks, &AudioState::default(), &SpectrumTables::SILENT);

            let clock = animation.phase(&clocks).val();
            let origin = animation.ticks(&clocks) as f64 + clock;
            let rough = 1.0 - smoothing;
            let angle = rough * std::f64::consts::FRAC_PI_2;
            let periods = f64::from(n_periods);
            let length = periods * (1.0 + NOISE_ROUGHENING * rough);
            let polarity = if invert { -1.0 } else { 1.0 };
            let mut compared = 0;
            for i in 0..4096 {
                let phase = f64::from(i) / 4096.0;
                let place = Phase::new(phase * periods + clock).val();
                if (place - duty_cycle).abs() < EDGE || place < EDGE || place > 1.0 - EDGE {
                    continue;
                }
                compared += 1;
                let expected = if place <= duty_cycle && duty_cycle != 0.0 {
                    let sample = NoiseBase::new(
                        origin + phase * length * angle.cos(),
                        phase * length * angle.sin(),
                    )
                    .sample(0.0, 0.0);
                    let shaped = if pulse { noise::gate(sample) } else { sample };
                    f64::from(shaped) * polarity * 0.8
                } else {
                    0.0
                };
                let value = prepared.value(Phase::new(phase));
                assert!(
                    (value - expected).abs() < TOLERANCE,
                    "at phase {phase}, with {n_periods} periods, smoothing {smoothing}, duty \
                     cycle {duty_cycle}, pulse {pulse} and invert {invert}, noise read {value} \
                     where the field holds {expected}"
                );
            }
            assert!(compared > 4000, "only {compared} points were compared");
        }
    }

    /// Asked about many places at once, an animation answers each exactly as
    /// it does asked about that place alone, whatever waveform and shape it
    /// has.
    #[test]
    fn a_batch_answers_each_place_as_it_is_answered_alone() {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        let tables = SpectrumTables::new(&AudioFrame::new(
            Default::default(),
            std::array::from_fn(|b| UnipolarF32::new(((b * 7) % 26) as f32 / 25.0)),
        ));
        let shapes: [Vec<StateChange>; 12] = [
            vec![StateChange::Waveform(Waveform::SineSquare)],
            vec![
                StateChange::Waveform(Waveform::SineSquare),
                StateChange::Smoothing(UnipolarFloat::new(0.0)),
            ],
            vec![
                StateChange::Waveform(Waveform::TriSaw),
                StateChange::DutyCycle(UnipolarFloat::new(0.4)),
            ],
            vec![
                StateChange::Waveform(Waveform::Noise),
                StateChange::Smoothing(UnipolarFloat::new(0.0)),
            ],
            vec![
                StateChange::Waveform(Waveform::Noise),
                StateChange::Smoothing(UnipolarFloat::new(0.6)),
                StateChange::Pulse(true),
            ],
            vec![
                StateChange::Waveform(Waveform::Noise),
                StateChange::Smoothing(UnipolarFloat::new(0.3)),
                StateChange::DutyCycle(UnipolarFloat::new(0.4)),
                StateChange::Invert(true),
            ],
            vec![
                StateChange::Waveform(Waveform::Noise),
                StateChange::DutyCycle(UnipolarFloat::new(0.0)),
            ],
            vec![
                StateChange::Waveform(Waveform::Noise),
                StateChange::Size(UnipolarFloat::new(0.0)),
            ],
            vec![
                StateChange::Waveform(Waveform::Noise),
                StateChange::NPeriods(0),
            ],
            vec![
                StateChange::Waveform(Waveform::Spectrum),
                StateChange::Invert(true),
            ],
            vec![StateChange::Waveform(Waveform::Constant)],
            vec![StateChange::Size(UnipolarFloat::new(0.0))],
        ];
        // Off either end of the unit interval as well as inside it, since a
        // figure's coordinates can run past both.
        let alongs: Vec<f32> = (-20..140).map(|i| i as f32 / 97.0).collect();
        let elements = 37;
        for changes in shapes {
            let mut animation = Animation::default();
            for sc in [
                StateChange::Size(UnipolarFloat::new(0.8)),
                StateChange::NPeriods(3),
                StateChange::Speed(BipolarFloat::new(0.3)),
            ]
            .into_iter()
            .chain(changes.iter().cloned())
            {
                animation.control(ControlMessage::Set(sc), &mut Noop);
            }
            animation.settle_controls();
            animation.update_state(Duration::from_millis(2300), &AudioState::default());
            let prepared =
                animation.prepare(&ClockBank::default(), &AudioState::default(), &tables);

            let mut on_figure = vec![f32::NAN; alongs.len()];
            prepared.values_on_figure(&alongs, &mut on_figure);
            for (&along, &batched) in alongs.iter().zip(&on_figure) {
                let alone = prepared.value(Phase::new(f64::from(along))) as f32;
                assert_eq!(
                    batched.to_bits(),
                    alone.to_bits(),
                    "{changes:?} answered {batched} on a figure at {along}, and {alone} alone"
                );
            }

            let mut on_elements = vec![f32::NAN; elements];
            prepared.values_on_elements(&mut on_elements);
            for (i, &batched) in on_elements.iter().enumerate() {
                let alone = prepared.value(Phase::new(i as f64 / elements as f64)) as f32;
                assert_eq!(
                    batched.to_bits(),
                    alone.to_bits(),
                    "{changes:?} answered {batched} at element {i} of a run, and {alone} alone"
                );
            }
        }
    }

    /// Pulsed, the spectrum reads its band levels as they are; unpulsed, it
    /// reads each level `v` as `2v - 1`. Either way the clock carries it round
    /// whatever standing is set to. Invert negates it, a duty cycle compresses
    /// the whole period into the front of its window, and with no periodicity
    /// every point reads the one place the clock is at.
    #[test]
    fn a_spectrum_reads_its_levels_carried_by_the_clock() {
        struct Noop;
        impl EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }
        let tables = SpectrumTables::new(&AudioFrame::new(
            Default::default(),
            std::array::from_fn(|b| UnipolarF32::new(((b * 7) % 26) as f32 / 25.0)),
        ));
        let smoothing = UnipolarFloat::new(0.4);

        let mut running = Animation::default();
        for sc in [
            StateChange::Waveform(Waveform::Spectrum),
            StateChange::Size(UnipolarFloat::ONE),
            StateChange::Smoothing(smoothing),
            StateChange::Speed(BipolarFloat::new(0.3)),
        ] {
            running.control(ControlMessage::Set(sc), &mut Noop);
        }
        running.settle_controls();
        // Off the top of its cycle, where a standing wave would be at full
        // amplitude and so indistinguishable from a travelling one.
        running.update_state(Duration::from_millis(700), &AudioState::default());

        let prepare = |changes: &[StateChange]| {
            let mut animation = running.clone();
            for sc in changes {
                animation.control(ControlMessage::Set(sc.clone()), &mut Noop);
            }
            animation.settle_controls();
            animation.prepare(&ClockBank::default(), &AudioState::default(), &tables)
        };
        let plain = prepare(&[]);
        let clock = plain.phase_temporal;
        assert!(
            (std::f64::consts::TAU * clock.val()).cos() < 0.5,
            "the clock sat at {clock:?}, too near a cycle's top for standing to show"
        );

        let pulsed = prepare(&[StateChange::Pulse(true)]);
        let points = (0..400).map(|i| Phase::new(i as f64 / 400.0));
        let (mut lowest, mut highest) = (f64::INFINITY, f64::NEG_INFINITY);
        for x in points.clone() {
            let level = tables.value(x + clock, smoothing);
            assert_eq!(pulsed.unit_value(x), level, "pulsed at {x:?}");
            let read = plain.unit_value(x);
            assert!(
                (read - (2.0 * level - 1.0)).abs() < 1e-12,
                "unpulsed, a level of {level} read {read} at {x:?}"
            );
            lowest = lowest.min(read);
            highest = highest.max(read);
        }
        assert!(
            lowest < -0.5 && highest > 0.5,
            "unpulsed, the spectrum only spanned {lowest} to {highest}, too \
             little of the bipolar range to tell it from a pulse"
        );

        for (flags, changes, reference) in [
            ("standing", vec![StateChange::Standing(true)], &plain),
            (
                "pulse and standing",
                vec![StateChange::Pulse(true), StateChange::Standing(true)],
                &pulsed,
            ),
        ] {
            let flagged = prepare(&changes);
            for x in points.clone() {
                assert_eq!(
                    flagged.unit_value(x),
                    reference.unit_value(x),
                    "{flags} changed the spectrum at {x:?}"
                );
            }
        }

        let inverted = prepare(&[StateChange::Invert(true)]);
        for x in points.clone() {
            assert_eq!(inverted.unit_value(x), -plain.unit_value(x), "at {x:?}");
        }

        let duty = UnipolarFloat::new(0.5);
        let compressed = prepare(&[StateChange::DutyCycle(duty)]);
        for x in points.clone() {
            let p = x + clock;
            let expected = if p >= duty {
                0.0
            } else {
                tables.value(p / duty, smoothing).mul_add(2.0, -1.0)
            };
            assert_eq!(compressed.unit_value(x), expected, "at {x:?}, {p:?} along");
        }

        let still = prepare(&[StateChange::NPeriods(0)]);
        for x in points {
            assert_eq!(
                still.unit_value(x),
                tables.value(clock, smoothing).mul_add(2.0, -1.0),
                "no periodicity read a different place at {x:?}"
            );
        }
    }
}
