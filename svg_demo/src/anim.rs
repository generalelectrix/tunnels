//! Driving the show's real animation system.
//!
//! The demo does not reimplement waveforms. It builds a `tunnels_model`
//! `Animation`, drives it through the same `ControlMessage`s the console sends,
//! and ticks it with the same `update_state`. Anything that looks right here
//! looks the same in a show.

use crate::params::WaveParams;

/// Samples across one sweep of spatial phase in the sine table.
const SINE_TABLE: usize = 1024;
use std::time::Duration;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};
use tunnels_model::animation::{
    Animation, ControlMessage, EmitStateChange, PreparedAnimation, StateChange, Waveform,
    WaveformState,
};
use tunnels_model::clock_bank::StaticClockBank;

/// Discards the state changes `Animation::control` reports.
///
/// The console echoes these back to its control surface to light up buttons.
/// The demo's controls are the source of the change, so there is nothing to
/// tell.
struct Discard;

impl EmitStateChange for Discard {
    fn emit_animation_state_change(&mut self, _: StateChange) {}
}

/// A running animation, plus the parameters it was built from.
pub struct LiveWave {
    params: WaveParams,
    animation: Animation,
    /// No external clocks: the demo drives each animation's internal clock, so
    /// this stays empty and `clock_source` stays `None`.
    clocks: StaticClockBank,
    /// Everything about the animation that is fixed for this frame.
    ///
    /// Resolving it once is what makes a per-vertex animation affordable: what
    /// answering costs is mostly deciding which clock drives it and what the
    /// amplitude works out to, and none of that depends on the vertex asking.
    prepared: PreparedAnimation,
    /// Sine, sampled across one sweep of spatial phase.
    ///
    /// Not because `sin` is slow — a polynomial replacing it saved a fifth of
    /// what this does. The cost is the plumbing a waveform is reached through:
    /// the args struct, the duty-cycle branches, the standing-wave envelope,
    /// the dispatch. A table short-circuits all of it, and sine is the only
    /// waveform it is safe to short-circuit, since interpolating turns a jump
    /// into a ramp one cell wide and a sine has no jump to lose. Its error here
    /// is around 1e-7.
    sine_table: Vec<f32>,
}

impl LiveWave {
    pub fn new(params: &WaveParams) -> Self {
        let animation = build(params);
        let clocks = StaticClockBank::default();
        let mut wave = Self {
            params: params.clone(),
            prepared: animation.prepare(&clocks, UnipolarFloat::ZERO),
            animation,
            clocks,
            sine_table: Vec::new(),
        };
        wave.resample();
        wave
    }

    /// Resample sine for this frame. A thousand evaluations against the tens of
    /// thousands it saves.
    fn resample(&mut self) {
        self.sine_table.clear();
        if !matches!(self.params.waveform, WaveformKind::Sine) {
            return;
        }
        self.sine_table.reserve(SINE_TABLE + 1);
        for i in 0..=SINE_TABLE {
            let phase = Phase::new(i as f64 / SINE_TABLE as f64);
            self.sine_table.push(self.prepared.value(phase, 0) as f32);
        }
    }

    /// Advance the internal clock, rebuilding first if the knobs moved.
    ///
    /// Rebuilding restarts the clock, so it only happens on an actual change —
    /// otherwise a parameter held steady would freeze the animation.
    pub fn update(&mut self, params: &WaveParams, delta: Duration, audio: UnipolarFloat) {
        if self.params != *params {
            // Carry the speed and shape across, but the phase is lost. That is
            // a visible hitch when a knob moves, and it is the same hitch the
            // console has.
            self.animation = build(params);
            self.params = params.clone();
        }
        self.animation.update_state(delta, audio);
        self.prepared = self.animation.prepare(&self.clocks, audio);
        self.resample();
    }

    /// This frame's resolved waveform parameters.
    ///
    /// What a renderer needs to evaluate the same waveform somewhere the
    /// animation cannot follow it.
    pub fn state(&self) -> WaveformState {
        self.prepared.state()
    }

    /// The animation's value at a point in its spatial phase.
    ///
    /// The hot path: once per vertex per animation per frame.
    #[inline]
    pub fn value_f32(&self, phase: f32, index: usize) -> f32 {
        if self.sine_table.is_empty() {
            return self.prepared.value(Phase::new(f64::from(phase)), index) as f32;
        }
        let t = phase.rem_euclid(1.0) * SINE_TABLE as f32;
        let i = t as usize;
        let frac = t - i as f32;
        let a = self.sine_table[i.min(SINE_TABLE)];
        let b = self.sine_table[(i + 1).min(SINE_TABLE)];
        a + (b - a) * frac
    }
}

/// Build an `Animation` by sending it the same control messages the console
/// would.
fn build(p: &WaveParams) -> Animation {
    let mut animation = Animation::default();
    let mut sink = Discard;
    let mut set = |sc: StateChange| animation.control(ControlMessage::Set(sc), &mut sink);

    set(StateChange::Waveform(p.waveform.into()));
    set(StateChange::NPeriods(p.n_periods));
    set(StateChange::Size(UnipolarFloat::new(p.size)));
    set(StateChange::DutyCycle(UnipolarFloat::new(p.duty_cycle)));
    set(StateChange::Smoothing(UnipolarFloat::new(p.smoothing)));
    set(StateChange::Speed(BipolarFloat::new(p.speed)));
    set(StateChange::Pulse(p.pulse));
    set(StateChange::Standing(p.standing));
    set(StateChange::Invert(p.invert));

    // Smoothing is behind a smoother, so it eases toward the value it was told
    // rather than taking it. Settle it before the first frame, otherwise a
    // noise animation starts at the wrong correlation and drifts into place.
    animation.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
    animation
}

/// The demo's mirror of `tunnels_model`'s `Waveform`.
///
/// Exists only because the original does not derive `PartialEq`, which the
/// parameter struct needs to notice a change.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default,
)]
pub enum WaveformKind {
    #[default]
    Sine,
    Triangle,
    Square,
    Sawtooth,
    Noise,
    Constant,
}

impl WaveformKind {
    pub const ALL: [Self; 6] = [
        Self::Sine,
        Self::Triangle,
        Self::Square,
        Self::Sawtooth,
        Self::Noise,
        Self::Constant,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "tri",
            Self::Square => "sqr",
            Self::Sawtooth => "saw",
            Self::Noise => "noise",
            Self::Constant => "const",
        }
    }
}

impl From<WaveformKind> for Waveform {
    fn from(k: WaveformKind) -> Self {
        match k {
            WaveformKind::Sine => Waveform::Sine,
            WaveformKind::Triangle => Waveform::Triangle,
            WaveformKind::Square => Waveform::Square,
            WaveformKind::Sawtooth => Waveform::Sawtooth,
            WaveformKind::Noise => Waveform::Noise,
            WaveformKind::Constant => Waveform::Constant,
        }
    }
}
