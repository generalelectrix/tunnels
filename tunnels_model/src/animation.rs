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
use std::sync::LazyLock;
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
    ) -> TargetedAnimation<PreparedAnimation> {
        TargetedAnimation {
            animation: self.animation.prepare(external_clocks, audio_envelope),
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
        if self.active() {
            self.internal_clock.update_state(delta_t, audio_envelope);
            self.smoothing.update_state(delta_t);
        }
    }

    /// Resolve everything that is fixed for a frame, once.
    ///
    /// A render asks an animation for a value once per segment, or on a filled
    /// shape once per vertex — tens of thousands of times. Most of what
    /// answering that costs is the same every time: which clock is driving,
    /// what phase it is at, where the smoother has got to, and the amplitude
    /// the size, submaster and audio envelope multiply out to. None of it
    /// depends on where in the figure the question is being asked.
    pub fn prepare(
        &self,
        external_clocks: &impl ClockStore,
        audio_envelope: UnipolarFloat,
    ) -> PreparedAnimation {
        PreparedAnimation {
            static_params: self.static_params,
            phase_temporal: self.phase(external_clocks),
            smoothing: self.smoothing.val(),
            ticks: self.ticks(external_clocks),
            scale: self.scale_value(external_clocks, audio_envelope, 1.0),
            simplex_gen: self.simplex_gen,
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

/// An animation with everything that is constant for a frame already resolved.
///
/// Holds no reference to the animation it came from, so a render can prepare
/// its animations once and then walk a figure without borrowing anything.
#[derive(Clone, Copy, Debug)]
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

    /// The animation's value at a point, with amplitude applied.
    pub fn value(&self, spatial_phase_offset: Phase, offset_index: usize) -> f64 {
        if !self.active {
            return 0.;
        }
        self.unit_value(spatial_phase_offset, offset_index) * self.scale
    }

    /// Scale a value by the amplitude factors: size, clock submaster, and audio
    /// envelope.
    pub fn scale_value(&self, v: f64) -> f64 {
        v * self.scale
    }

    /// The waveform's own value, before amplitude.
    pub fn unit_value(&self, spatial_phase_offset: Phase, offset_index: usize) -> f64 {
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

                let x_offset = self.ticks as f64 + spatial_phase + temporal_phase;

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
                    (1.0 - self.smoothing.val()) * offset_index as f64
                };

                let val = self.simplex_gen.get([x_offset, y_offset]);

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
/// its range, so a knee placed a quarter of the way up that range is the value
/// noise spends about a quarter of its time below.
const NOISE_GATE_LOW: f64 = 0.15;

/// Where a noise pulse reaches full, as a magnitude of the noise.
///
/// Placed so that about the top tenth of noise magnitudes saturate. Noise
/// approaches the end of its own range too rarely to arrive at full any other
/// way.
const NOISE_GATE_HIGH: f64 = 0.63;

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

    /// A gated noise pulse rests at each end of the unipolar range rather than
    /// only approaching it, so the two knees are where it arrives, and outside
    /// them it holds still however far the noise goes on.
    #[test]
    fn a_noise_gate_rests_at_both_ends_of_its_range() {
        for v in [0.0, 0.05, NOISE_GATE_LOW, -NOISE_GATE_LOW, 0.1, -0.02] {
            assert_eq!(
                gate_noise(v),
                0.0,
                "noise of {v} lit a pulse from inside the dark zone"
            );
        }
        for v in [NOISE_GATE_HIGH, -NOISE_GATE_HIGH, 0.8, -0.9, 1.0, -1.0] {
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
            animation.prepare(&ClockBank::default(), UnipolarFloat::ZERO)
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
