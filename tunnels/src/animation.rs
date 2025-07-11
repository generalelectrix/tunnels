use crate::clock::Clock;
use crate::clock::ControllableClock;
use crate::clock::Ticks;
use crate::clock_bank::{ClockIdxExt, ClockStore};
use crate::master_ui::EmitStateChange as EmitShowStateChange;
use crate::waveforms::WaveformArgs;
use crate::{clock_bank::ClockIdx, waveforms};
use log::error;
use noise::NoiseFn;
use noise::Simplex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};
use tunnels_lib::smooth::Smoother;

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Sawtooth,
    Noise,
    Constant,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Animation {
    waveform: Waveform,
    pulse: bool,
    standing: bool,
    invert: bool,
    n_periods: u16,
    size: UnipolarFloat,
    duty_cycle: UnipolarFloat,
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
            waveform: Waveform::Sine,
            pulse: false,
            standing: false,
            invert: false,
            n_periods: 0,
            size: UnipolarFloat::ZERO,
            duty_cycle: UnipolarFloat::ONE,
            smoothing: Smoother::new(
                UnipolarFloat::new(0.25),
                Self::SMOOTH_SMOOTH_TIME,
                tunnels_lib::smooth::SmoothMode::Linear,
            ),
            internal_clock: Clock::new(),
            clock_source: None,
            use_audio_size: false,
            simplex_gen: get_simplex_gen(),
        }
    }
}

impl Animation {
    const SMOOTH_SMOOTH_TIME: Duration = Duration::from_millis(100);

    /// Return true if this animation has nonzero size.
    fn active(&self) -> bool {
        self.size > 0.0
    }

    fn phase(&self, external_clocks: &impl ClockStore) -> Phase {
        match self.clock_source {
            None => self.internal_clock.phase(),
            Some(id) => external_clocks.phase(id),
        }
    }

    fn ticks(&self, external_clocks: &impl ClockStore) -> Ticks {
        match self.clock_source {
            None => self.internal_clock.ticks(),
            Some(id) => external_clocks.ticks(id),
        }
    }

    /// Return the clock's current rate, scaled into a bipolar float.
    fn clock_speed(&self) -> BipolarFloat {
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

    pub fn get_value(
        &self,
        spatial_phase_offset: Phase,
        offset_index: usize,
        external_clocks: &impl ClockStore,
        audio_envelope: UnipolarFloat,
    ) -> f64 {
        if !self.active() {
            return 0.;
        }

        let mut result = self.size.val()
            * match self.waveform {
                Waveform::Sine => {
                    waveforms::sine(&self.waveform_args(spatial_phase_offset, external_clocks))
                }
                Waveform::Square => {
                    waveforms::square(&self.waveform_args(spatial_phase_offset, external_clocks))
                }
                Waveform::Sawtooth => {
                    waveforms::sawtooth(&self.waveform_args(spatial_phase_offset, external_clocks))
                }
                Waveform::Triangle => {
                    waveforms::triangle(&self.waveform_args(spatial_phase_offset, external_clocks))
                }
                Waveform::Noise => {
                    // Handle duty cycle - this is a bit odd compared to waveforms,
                    // since noise isn't periodic. Rather than trying to compress
                    // the waveform to maintain the waveshape, we just turn off
                    // the animation for a portion of each cycle.
                    let spatial_phase = spatial_phase_offset.val() * self.n_periods as f64;
                    let temporal_phase = self.phase(external_clocks).val();

                    if Phase::new(spatial_phase + temporal_phase) > self.duty_cycle
                        || self.duty_cycle == 0.0
                    {
                        return 0.0;
                    }

                    let x_offset =
                        self.ticks(external_clocks) as f64 + spatial_phase + temporal_phase;

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
                    let y_offset = if self.n_periods == 0 {
                        0.0
                    } else {
                        (1.0 - self.smoothing.val().val()) * offset_index as f64
                    };

                    let mut val = self.simplex_gen.get([x_offset, y_offset]);

                    // Take the square for pulse mode to avoid sharp edges at zero,
                    // and to maintain a bias towards the animation value frequently
                    // touching zero. This produces more of a forest of peaks.
                    // Simply rescaling the full noise spectrum into the unipolar
                    // range would result in very rarely touching zero, which is
                    // unlikely to be what we're looking for, artistically speaking.
                    if self.pulse {
                        val = val.powi(2);
                    }
                    val
                }
                Waveform::Constant => 1.0,
            };

        // scale this animation by submaster level if using external clock
        let mut use_audio_size = self.use_audio_size;
        if let Some(id) = self.clock_source {
            result *= external_clocks.submaster_level(id).val();
            use_audio_size = use_audio_size || external_clocks.use_audio_size(id);
        }
        // scale this animation by audio envelope if set
        if use_audio_size {
            result *= audio_envelope.val();
        }
        if self.invert {
            -result
        } else {
            result
        }
    }

    #[inline(always)]
    fn waveform_args(
        &self,
        spatial_phase_offset: Phase,
        external_clocks: &impl ClockStore,
    ) -> WaveformArgs {
        WaveformArgs {
            phase_spatial: spatial_phase_offset * (self.n_periods as f64),
            phase_temporal: self.phase(external_clocks),
            smoothing: self.smoothing.val(),
            duty_cycle: self.duty_cycle,
            pulse: self.pulse,
            standing: self.standing,
        }
    }

    /// Emit the current value of all controllable animator state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_animation_state_change(Waveform(self.waveform));
        emitter.emit_animation_state_change(Pulse(self.pulse));
        emitter.emit_animation_state_change(Standing(self.standing));
        emitter.emit_animation_state_change(Invert(self.invert));
        emitter.emit_animation_state_change(NPeriods(self.n_periods));
        emitter.emit_animation_state_change(Speed(self.clock_speed()));
        emitter.emit_animation_state_change(Size(self.size));
        emitter.emit_animation_state_change(DutyCycle(self.duty_cycle));
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
                let source: Option<ClockIdx> = match source {
                    Some(s) => match s.try_into() {
                        Ok(s) => Some(s),
                        Err(e) => {
                            error!("could not process animation control message: {e}");
                            return;
                        }
                    },
                    None => None,
                };
                self.handle_state_change(StateChange::ClockSource(source), emitter);
            }
            TogglePulse => {
                self.pulse = !self.pulse;
                emitter.emit_animation_state_change(StateChange::Pulse(self.pulse));
            }
            ToggleStanding => {
                self.standing = !self.standing;
                emitter.emit_animation_state_change(StateChange::Standing(self.standing));
            }
            ToggleInvert => {
                self.invert = !self.invert;
                emitter.emit_animation_state_change(StateChange::Invert(self.invert));
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
            Waveform(v) => self.waveform = v,
            Pulse(v) => self.pulse = v,
            Standing(v) => self.standing = v,
            Invert(v) => self.invert = v,
            NPeriods(v) => self.n_periods = v,
            Speed(v) => self.set_clock_speed(v),
            Size(v) => self.size = v,
            DutyCycle(v) => self.duty_cycle = v,
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
    /// Since clock IDs need to be validated, this path handles the fallible case.
    /// FIXME: it would be nicer to validate this at control message creation time,
    /// but at the moment control message creator functions are infallible and
    /// that's more refactoring than I want to deal with right now.
    SetClockSource(Option<ClockIdxExt>),
    TogglePulse,
    ToggleStanding,
    ToggleInvert,
    ToggleUseAudioSize,
    ToggleUseAudioSpeed,
}

pub trait EmitStateChange {
    fn emit_animation_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_animation_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Animation(sc))
    }
}
