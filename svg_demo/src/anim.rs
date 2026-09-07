//! Driving the show's real animation system.
//!
//! The demo does not reimplement waveforms. It builds a `tunnels_model`
//! `Animation`, drives it through the same `ControlMessage`s the console sends,
//! and ticks it with the same `update_state`. Anything that looks right here
//! looks the same in a show.

use crate::params::WaveParams;
use std::time::Duration;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};
use tunnels_model::animation::{
    Animation, ControlMessage, EmitStateChange, StateChange, Waveform,
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

/// Entries in the per-frame waveform table.
///
/// The table is indexed by spatial phase over one full sweep, and a mesh never
/// resolves anything finer than this, so the sampling is not what limits
/// quality.
const TABLE_SIZE: usize = 1024;

/// A running animation, plus the parameters it was built from.
pub struct LiveWave {
    params: WaveParams,
    animation: Animation,
    /// No external clocks: the demo drives each animation's internal clock, so
    /// this stays empty and `clock_source` stays `None`.
    clocks: StaticClockBank,
    /// The waveform sampled across one sweep of spatial phase, rebuilt once a
    /// frame — for sine only.
    ///
    /// Sine is the only waveform whose cost is the waveform. Triangle, square
    /// and sawtooth are a few multiplies and a compare; what made them look
    /// expensive in a profile was `get_value` rederiving frame-constant state
    /// on every call, not the arithmetic.
    ///
    /// And sine is the only one it is safe to tabulate. Interpolating between
    /// samples turns a jump into a ramp one cell wide, which is exactly the
    /// edge a square or a sawtooth exists to have. Sine has no edge to lose.
    table: Vec<f32>,
}

impl LiveWave {
    pub fn new(params: &WaveParams) -> Self {
        let mut wave = Self {
            params: params.clone(),
            animation: build(params),
            clocks: StaticClockBank::default(),
            table: Vec::new(),
        };
        wave.tabulate(UnipolarFloat::ZERO);
        wave
    }

    /// Resample the waveform for this frame.
    ///
    /// Cheap enough to do unconditionally: a thousand evaluations against the
    /// tens of thousands it saves.
    fn tabulate(&mut self, audio: UnipolarFloat) {
        if !matches!(self.params.waveform, WaveformKind::Sine) {
            self.table.clear();
            return;
        }
        self.table.clear();
        self.table.reserve(TABLE_SIZE + 1);
        for i in 0..=TABLE_SIZE {
            let phase = i as f64 / TABLE_SIZE as f64;
            self.table.push(self.animation.get_value(
                Phase::new(phase),
                0,
                &self.clocks,
                audio,
            ) as f32);
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
        self.tabulate(audio);
    }

    /// The animation's value at a point in its spatial phase.
    ///
    /// `index` stands in for a tunnel segment's ordinal, which only `Noise`
    /// reads — it uses it as the second axis of the simplex field, with
    /// smoothing as the correlation between neighbours.
    pub fn value(&self, phase: f64, index: usize, audio: UnipolarFloat) -> f64 {
        f64::from(self.value_f32(phase as f32, index, audio))
    }

    /// The animation's value, from the table where there is one.
    ///
    /// This is the hot path: it runs once per vertex per animation per frame,
    /// so the table is the difference between a transcendental and a load.
    #[inline]
    pub fn value_f32(&self, phase: f32, index: usize, audio: UnipolarFloat) -> f32 {
        if self.table.is_empty() {
            // Everything but sine: either cheap to evaluate exactly, or noise,
            // which reads the index and so has no table to read.
            return self.animation.get_value(
                Phase::new(f64::from(phase)),
                index,
                &self.clocks,
                audio,
            ) as f32;
        }
        let t = phase.rem_euclid(1.0) * TABLE_SIZE as f32;
        let i = t as usize;
        let frac = t - i as f32;
        let a = self.table[i.min(TABLE_SIZE)];
        let b = self.table[(i + 1).min(TABLE_SIZE)];
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
