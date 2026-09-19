//! Synthetic test signals: stereo, rendered up front at 48 kHz,
//! deterministic (seeded) where random.

/// A stereo signal rendered up front.
pub type Signal = Vec<[f32; 2]>;

pub const SAMPLE_RATE: u32 = 48000;

pub struct Lcg(pub u64);
impl Lcg {
    pub fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }
}

pub fn silence(sr: u32, secs: f32) -> Signal {
    vec![[0.0, 0.0]; (sr as f32 * secs) as usize]
}

pub fn add_mono(sig: &mut Signal, sr: u32, start: f32, f: impl Fn(f32) -> Option<f32>) {
    let start_idx = (start * sr as f32) as usize;
    for (i, frame) in sig.iter_mut().enumerate().skip(start_idx) {
        let t = (i - start_idx) as f32 / sr as f32;
        match f(t) {
            Some(v) => {
                frame[0] += v;
                frame[1] += v;
            }
            None => break,
        }
    }
}

pub fn sine(sig: &mut Signal, sr: u32, start: f32, len: f32, freq: f32, amp: f32) {
    add_mono(sig, sr, start, |t| {
        (t < len).then(|| amp * (2.0 * std::f32::consts::PI * freq * t).sin())
    });
}

/// Simple kick: fixed-pitch decaying sine.
pub fn kick_simple(sig: &mut Signal, sr: u32, start: f32, amp: f32) {
    add_mono(sig, sr, start, |t| {
        (t < 0.25).then(|| amp * (2.0 * std::f32::consts::PI * 60.0 * t).sin() * (-t / 0.05).exp())
    });
}

/// Realistic kick: pitch sweeps 150 -> 50 Hz with a 40 ms time constant,
/// amplitude decays with an 80 ms time constant, truncated at 400 ms.
pub fn kick_real(sig: &mut Signal, sr: u32, start: f32, amp: f32) {
    add_mono(sig, sr, start, |t| {
        (t < 0.4).then(|| {
            let phase =
                2.0 * std::f32::consts::PI * (50.0 * t + 100.0 * 0.04 * (1.0 - (-t / 0.04).exp()));
            amp * phase.sin() * (-t / 0.08).exp()
        })
    });
}

pub fn onsets(bpm: f32, from: f32, to: f32) -> Vec<f32> {
    let period = 60.0 / bpm;
    let mut v = Vec::new();
    let mut t = from;
    while t < to {
        v.push(t);
        t += period;
    }
    v
}

/// A signal together with the kick onset times it was built from.
pub struct KickSignal {
    pub signal: Signal,
    pub onsets: Vec<f32>,
}

fn kicks(secs: f32, ons: Vec<f32>, amp: impl Fn(usize, f32) -> f32, real: bool) -> KickSignal {
    let mut signal = silence(SAMPLE_RATE, secs);
    for (i, &on) in ons.iter().enumerate() {
        if real {
            kick_real(&mut signal, SAMPLE_RATE, on, amp(i, on));
        } else {
            kick_simple(&mut signal, SAMPLE_RATE, on, amp(i, on));
        }
    }
    KickSignal {
        signal,
        onsets: ons,
    }
}

/// The cases whose responses the golden test pins, by name. Each one
/// exercises a behaviour the chain was tuned for.
pub const GOLDEN_CASES: [&str; 7] = [
    "w04b_realkick_120",
    "w04e_onset_jitter_120",
    "w05_quiet_loud_quiet",
    "w06_one_loud_hit",
    "w06b_one_buffer_spike",
    "w15_kick_under_1khz",
    "w16_hiss_then_silence",
];

/// Build a golden case by name.
pub fn golden_case(name: &str) -> Option<KickSignal> {
    let sr = SAMPLE_RATE;
    Some(match name {
        // Realistic kicks at 120 BPM.
        "w04b_realkick_120" => kicks(10.0, onsets(120.0, 0.5, 10.0), |_, _| 0.8, true),
        // Simple kicks with ±20 ms random onset jitter.
        "w04e_onset_jitter_120" => {
            let mut rng = Lcg(99);
            let ons = onsets(120.0, 0.5, 10.0)
                .into_iter()
                .map(|t| t + 0.02 * rng.next_f32())
                .collect();
            kicks(10.0, ons, |_, _| 0.8, false)
        }
        // Quiet -> loud -> quiet, 10 s each.
        "w05_quiet_loud_quiet" => kicks(
            30.0,
            onsets(120.0, 0.5, 30.0),
            |_, on| if (10.0..20.0).contains(&on) { 0.8 } else { 0.1 },
            false,
        ),
        // One 2x hit at 5 s.
        "w06_one_loud_hit" => kicks(
            20.0,
            onsets(120.0, 0.5, 20.0),
            |_, on| if (on - 5.0).abs() < 0.01 { 1.6 } else { 0.8 },
            false,
        ),
        // One-buffer 2x spike at 5.25 s, between kicks.
        "w06b_one_buffer_spike" => {
            let mut k = kicks(20.0, onsets(120.0, 0.5, 20.0), |_, _| 0.8, false);
            let spike_start = (5.25 * sr as f32) as usize;
            for frame in k.signal.iter_mut().skip(spike_start).take(64) {
                *frame = [1.6, 1.6];
            }
            k
        }
        // Quiet kick under louder 1 kHz content.
        "w15_kick_under_1khz" => {
            let ons = onsets(120.0, 0.5, 20.0);
            let mut signal = silence(sr, 20.0);
            sine(&mut signal, sr, 0.0, 20.0, 1000.0, 0.7);
            for &on in &ons {
                kick_simple(&mut signal, sr, on, 0.3);
            }
            KickSignal {
                signal,
                onsets: ons,
            }
        }
        // -50 dB hiss throughout; kicks for 10 s, then hiss alone for 20 s.
        "w16_hiss_then_silence" => {
            let ons = onsets(120.0, 0.5, 10.0);
            let mut signal = silence(sr, 30.0);
            let mut rng = Lcg(3);
            for frame in signal.iter_mut() {
                let v = 0.003 * rng.next_f32();
                *frame = [v, v];
            }
            for &on in &ons {
                kick_simple(&mut signal, sr, on, 0.8);
            }
            KickSignal {
                signal,
                onsets: ons,
            }
        }
        _ => return None,
    })
}
