//! Offline characterisation of the envelope chain.
//!
//! Drives `Processor` with synthetic waveforms under production conditions
//! (stereo, 64-frame buffers) and records every stage per
//! buffer: raw peak, the lowpass band's smoothed envelope, floor
//! and ceiling, and all eight normalized bands. Each waveform writes a CSV to
//! the output directory and prints a metrics block to stdout.
//!
//! Usage: `cargo run -p tunnels_audio --release --example envelope_suite -- <out_dir>`
//!
//! With `--music <file.clip> --loops N` it instead loops a packed real-music
//! clip N times through one processor and prints a per-loop convergence table
//! for the adaptive parameters, plus a shift experiment: the same clip with a
//! few samples of silence prepended, comparing per-band kick peaks. The chain
//! is shift-invariant, so the experiment is a standing check that per-band
//! peaks do not depend on where the input falls relative to any buffer or
//! filter grid.
//!
//! `--floor-limit` runs either mode with the normalizer floor in limit mode
//! instead of the default average mode.

// Shared with the integration tests by path; this binary uses only parts.
#[allow(dead_code)]
#[path = "../tests/common/clip.rs"]
mod clip;
#[path = "../tests/common/offline.rs"]
mod offline;
#[allow(dead_code)]
#[path = "../tests/common/signals.rs"]
mod signals;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

use tunnels_audio::processor::{BandStages, NUM_OUTPUT_BANDS, ProcessorSettings, TrackingMode};

use signals::{KickSignal, Lcg, Signal, kick_real, kick_simple, kicks, onsets, silence, sine};

/// One recorded buffer.
struct Row {
    t: f32,
    raw_peak: f32,
    /// Normalized output per band.
    bands: [f32; NUM_OUTPUT_BANDS],
    /// Intermediate stages per band.
    stages: [BandStages; NUM_OUTPUT_BANDS],
}

impl Row {
    /// The lowpass band's stages.
    fn lowpass(&self) -> &BandStages {
        &self.stages[0]
    }
}

#[derive(Clone, Copy)]
struct RunConfig {
    sample_rate: u32,
    frames: usize,
    /// Run the normalizer floor in limit mode instead of average mode.
    floor_limit: bool,
}

const PROD: RunConfig = RunConfig {
    sample_rate: 48000,
    frames: 64,
    floor_limit: false,
};

fn run(cfg: RunConfig, signal: &Signal) -> Vec<Row> {
    let settings = ProcessorSettings::default();
    if cfg.floor_limit {
        settings
            .norm_floor_mode
            .store(TrackingMode::Limit, Ordering::Relaxed);
    }
    let mut rows = Vec::with_capacity(signal.len() / cfg.frames + 1);
    offline::run_stereo(
        cfg.sample_rate,
        cfg.frames,
        settings.clone(),
        signal,
        |buf_idx, processor, outputs| {
            let chunk =
                &signal[buf_idx * cfg.frames..((buf_idx + 1) * cfg.frames).min(signal.len())];
            let raw_peak = chunk
                .iter()
                .map(|f| f[0].abs().max(f[1].abs()))
                .fold(0.0, f32::max);
            rows.push(Row {
                t: (buf_idx * cfg.frames) as f32 / cfg.sample_rate as f32,
                raw_peak,
                bands: *outputs,
                stages: std::array::from_fn(|b| processor.band_stages(b).expect("band in range")),
            });
        },
    );
    rows
}

fn write_csv(path: &Path, rows: &[Row]) {
    let mut s = String::from("t,raw_peak,smoothed,floor,ceiling");
    for b in 0..NUM_OUTPUT_BANDS {
        let _ = write!(s, ",band{b}");
    }
    for b in 1..NUM_OUTPUT_BANDS {
        let _ = write!(s, ",sm{b},fl{b},ceil{b}");
    }
    s.push('\n');
    for r in rows {
        let lp = r.lowpass();
        let _ = write!(
            s,
            "{:.5},{:.5},{:.5},{:.5},{:.5}",
            r.t, r.raw_peak, lp.smoothed, lp.floor, lp.ceiling
        );
        for b in r.bands {
            let _ = write!(s, ",{b:.5}");
        }
        for st in &r.stages[1..] {
            let _ = write!(s, ",{:.5},{:.5},{:.5}", st.smoothed, st.floor, st.ceiling);
        }
        s.push('\n');
    }
    fs::write(path, s).expect("write csv");
}

// ------------------------------------------------------------------ metrics

fn window(rows: &[Row], from: f32, to: f32) -> impl Iterator<Item = &Row> {
    rows.iter().filter(move |r| r.t >= from && r.t < to)
}

struct Stats {
    mean: f32,
    min: f32,
    max: f32,
}

fn stats(vals: impl Iterator<Item = f32>) -> Stats {
    let mut n = 0.0_f32;
    let mut sum = 0.0_f32;
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for v in vals {
        n += 1.0;
        sum += v;
        min = min.min(v);
        max = max.max(v);
    }
    Stats {
        mean: sum / n.max(1.0),
        min,
        max,
    }
}

struct KickStat {
    onset: f32,
    peak: f32,
    peak_smoothed: f32,
    trough: f32,
    ceiling: f32,
    floor: f32,
    bands_peak: [f32; NUM_OUTPUT_BANDS],
    latency_ms: f32,
}

fn kick_stats(rows: &[Row], onsets: &[f32]) -> Vec<KickStat> {
    onsets
        .iter()
        .map(|&on| {
            let trough = window(rows, on - 0.05, on)
                .map(|r| r.bands[0])
                .fold(f32::MAX, f32::min);
            let mut peak = 0.0_f32;
            let mut peak_t = on;
            let mut peak_smoothed = 0.0_f32;
            let mut bands_peak = [0.0_f32; NUM_OUTPUT_BANDS];
            for r in window(rows, on, on + 0.15) {
                if r.bands[0] > peak {
                    peak = r.bands[0];
                    peak_t = r.t;
                }
                peak_smoothed = peak_smoothed.max(r.lowpass().smoothed);
                for (b, v) in bands_peak.iter_mut().zip(r.bands) {
                    *b = b.max(v);
                }
            }
            let at_onset = rows.iter().find(|r| r.t >= on).expect("onset within run");
            KickStat {
                onset: on,
                peak,
                peak_smoothed,
                trough: if trough == f32::MAX { 0.0 } else { trough },
                ceiling: at_onset.lowpass().ceiling,
                floor: at_onset.lowpass().floor,
                bands_peak,
                latency_ms: (peak_t - on) * 1000.0,
            }
        })
        .collect()
}

fn cv(vals: &[f32]) -> f32 {
    let n = vals.len() as f32;
    let mean = vals.iter().sum::<f32>() / n;
    if mean == 0.0 {
        return 0.0;
    }
    let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
    var.sqrt() / mean
}

fn report_kicks(out: &mut String, label: &str, ks: &[KickStat]) {
    let peaks: Vec<f32> = ks.iter().map(|k| k.peak).collect();
    let p = stats(peaks.iter().copied());
    let t = stats(ks.iter().map(|k| k.trough));
    let l = stats(ks.iter().map(|k| k.latency_ms));
    let registered = ks.iter().filter(|k| k.peak - k.trough >= 0.2).count();
    let _ = writeln!(
        out,
        "  {label}: {} kicks, {registered} registered (rise>=0.2)\n    band0 peak mean {:.3} min {:.3} max {:.3} CV {:.3}\n    trough mean {:.3} min {:.3} max {:.3}\n    onset->peak latency ms mean {:.1} min {:.1} max {:.1}",
        ks.len(),
        p.mean,
        p.min,
        p.max,
        cv(&peaks),
        t.mean,
        t.min,
        t.max,
        l.mean,
        l.min,
        l.max
    );
    let _ = writeln!(
        out,
        "    per-kick: onset  band0  smoothed  floor  ceil   | band1  band2  band3"
    );
    for k in ks {
        let _ = writeln!(
            out,
            "             {:5.2}  {:.3}  {:.3}     {:.3}  {:.3}  | {:.3}  {:.3}  {:.3}",
            k.onset,
            k.peak,
            k.peak_smoothed,
            k.floor,
            k.ceiling,
            k.bands_peak[1],
            k.bands_peak[2],
            k.bands_peak[3]
        );
    }
}

/// Suppression after an event: first post-event kick peak relative to the
/// mean of the kicks in the 2 s before it, and time until a post-event kick
/// regains 90 % of that pre-event mean.
fn report_suppression(out: &mut String, ks: &[KickStat], event: f32) {
    let pre: Vec<f32> = ks
        .iter()
        .filter(|k| k.onset >= event - 2.0 && k.onset < event - 0.01)
        .map(|k| k.peak)
        .collect();
    let pre_mean = pre.iter().sum::<f32>() / pre.len().max(1) as f32;
    let post: Vec<&KickStat> = ks.iter().filter(|k| k.onset > event + 0.01).collect();
    let first = post.first().map(|k| k.peak).unwrap_or(0.0);
    let recovered = post
        .iter()
        .find(|k| k.peak >= 0.9 * pre_mean)
        .map(|k| k.onset - event);
    let _ = writeln!(
        out,
        "  suppression @ {event:.2}s: pre-event mean peak {pre_mean:.3}, first post-event peak {first:.3} (depth {:.2}x), recovery to 90%: {}",
        first / pre_mean.max(1e-6),
        match recovered {
            Some(s) => format!("{s:.2}s"),
            None => "never".to_string(),
        }
    );
}

fn report_steady(out: &mut String, label: &str, rows: &[Row], from: f32, to: f32) {
    let b = stats(window(rows, from, to).map(|r| r.bands[0]));
    let sm = stats(window(rows, from, to).map(|r| r.lowpass().smoothed));
    let last = window(rows, from, to).last().expect("rows in window");
    let _ = writeln!(
        out,
        "  {label} [{from:.1}-{to:.1}s]: band0 mean {:.3} ripple {:.4} | smoothed mean {:.4} ripple {:.4} | floor {:.4} ceil {:.4}",
        b.mean,
        b.max - b.min,
        sm.mean,
        sm.max - sm.min,
        last.lowpass().floor,
        last.lowpass().ceiling,
    );
}

fn report_burst(out: &mut String, rows: &[Row], on: f32, off: f32) {
    let peak = window(rows, on, off)
        .map(|r| r.bands[0])
        .fold(0.0, f32::max);
    let cross = |thr: f32, from: f32, rising: bool| -> Option<f32> {
        rows.iter()
            .find(|r| {
                r.t >= from
                    && if rising {
                        r.bands[0] >= thr
                    } else {
                        r.bands[0] <= thr
                    }
            })
            .map(|r| r.t)
    };
    let t10 = cross(0.1 * peak, on, true);
    let t90 = cross(0.9 * peak, on, true);
    let f90 = cross(0.9 * peak, off, false);
    let f10 = cross(0.1 * peak, off, false);
    let fmt = |a: Option<f32>, b: Option<f32>| match (a, b) {
        (Some(a), Some(b)) => format!("{:.1} ms", (b - a) * 1000.0),
        _ => "n/a".to_string(),
    };
    let _ = writeln!(
        out,
        "  burst: peak {peak:.3}, onset->10% {}, rise 10->90% {}, off->90% {}, fall 90->10% {}",
        fmt(Some(on), t10),
        fmt(t10, t90),
        fmt(Some(off), f90),
        fmt(f90, f10)
    );
}

fn report_band_peaks(out: &mut String, label: &str, rows: &[Row], from: f32, to: f32) {
    let mut peaks = [0.0_f32; NUM_OUTPUT_BANDS];
    for r in window(rows, from, to) {
        for (p, v) in peaks.iter_mut().zip(r.bands) {
            *p = p.max(v);
        }
    }
    let _ = writeln!(
        out,
        "  {label} band peaks [{from:.1}-{to:.1}s]: {peaks:.3?}"
    );
}

// -------------------------------------------------------------------- suite

/// Writes a waveform's metrics block from its recorded rows.
type Report = Box<dyn Fn(&mut String, &[Row])>;

struct Case {
    name: &'static str,
    cfg: RunConfig,
    signal: Signal,
    report: Report,
}

/// A case built from a kick signal, reported as kicks plus `extra`.
fn kick_case(
    name: &'static str,
    cfg: RunConfig,
    KickSignal { signal, onsets }: KickSignal,
    extra: impl Fn(&mut String, &[KickStat], &[Row]) + 'static,
) -> Case {
    Case {
        name,
        cfg,
        signal,
        report: Box::new(move |out, rows| {
            let ks = kick_stats(rows, &onsets);
            report_kicks(out, name, &ks);
            extra(out, &ks, rows);
        }),
    }
}

/// A case built from one of the shared golden signals.
fn golden_kick_case(
    name: &'static str,
    extra: impl Fn(&mut String, &[KickStat], &[Row]) + 'static,
) -> Case {
    kick_case(
        name,
        PROD,
        signals::golden_case(name).expect("golden case"),
        extra,
    )
}

fn suite() -> Vec<Case> {
    let sr = PROD.sample_rate;
    let mut cases = Vec::new();

    // W1: steady 60 Hz sine, 30 s.
    let mut sig = silence(sr, 30.0);
    sine(&mut sig, sr, 0.0, 30.0, 60.0, 0.8);
    cases.push(Case {
        name: "w01_steady_60hz",
        cfg: PROD,
        signal: sig,
        report: Box::new(|out, rows| {
            for (a, b) in [
                (0.5, 1.0),
                (2.0, 3.0),
                (5.0, 6.0),
                (10.0, 11.0),
                (20.0, 21.0),
                (28.0, 30.0),
            ] {
                report_steady(out, "steady", rows, a, b);
            }
        }),
    });

    // W2: 60 Hz burst 0.2-0.5 s.
    let mut sig = silence(sr, 1.5);
    sine(&mut sig, sr, 0.2, 0.3, 60.0, 0.8);
    cases.push(Case {
        name: "w02_burst_60hz",
        cfg: PROD,
        signal: sig,
        report: Box::new(|out, rows| report_burst(out, rows, 0.2, 0.5)),
    });

    // W3: single-sample impulse at 0.3 s.
    let mut sig = silence(sr, 1.0);
    let idx = (0.3 * sr as f32) as usize;
    sig[idx] = [1.0, 1.0];
    cases.push(Case {
        name: "w03_impulse",
        cfg: PROD,
        signal: sig,
        report: Box::new(|out, rows| {
            let max = stats(window(rows, 0.3, 0.5).map(|r| r.bands[0])).max;
            let smax = stats(window(rows, 0.3, 0.5).map(|r| r.lowpass().smoothed)).max;
            let _ = writeln!(
                out,
                "  impulse: band0 peak {max:.3}, smoothed peak {smax:.4}"
            );
            report_band_peaks(out, "impulse", rows, 0.3, 0.5);
        }),
    });

    // W4: simple kicks 120 BPM, 10 s.
    cases.push(kick_case(
        "w04_kicks_120",
        PROD,
        kicks(
            PROD.sample_rate,
            10.0,
            onsets(120.0, 0.5, 10.0),
            |_, _| 0.8,
            kick_simple,
        ),
        |out, ks, rows| {
            // Startup: how long until the ceiling is within 10 % of the
            // kicks it is normalizing.
            let target = 0.9 * ks[ks.len() - 1].peak_smoothed;
            let caught = rows
                .iter()
                .find(|r| r.lowpass().ceiling >= target)
                .map(|r| r.t);
            let _ = writeln!(
                out,
                "    ceiling reaches 90% of kick level at {}",
                caught.map_or("never".into(), |t| format!("{t:.2}s"))
            );
        },
    ));
    // W4b: realistic kicks.
    cases.push(golden_kick_case("w04b_realkick_120", |_, _, _| {}));
    // W4c: 140 BPM with a 16th-note double hit on beat 1 of every bar.
    let mut ons = Vec::new();
    let period = 60.0 / 140.0;
    let mut t = 0.5;
    let mut beat = 0;
    while t < 10.0 {
        ons.push(t);
        if beat % 4 == 0 {
            ons.push(t + period / 4.0);
        }
        t += period;
        beat += 1;
    }
    cases.push(kick_case(
        "w04c_double_hits_140",
        PROD,
        kicks(PROD.sample_rate, 10.0, ons, |_, _| 0.8, kick_real),
        |_, _, _| {},
    ));
    // W4d: ±10 % amplitude jitter.
    let mut rng = Lcg(42);
    let ons = onsets(120.0, 0.5, 10.0);
    let amps: Vec<f32> = ons
        .iter()
        .map(|_| 0.8 * (1.0 + 0.1 * rng.next_f32()))
        .collect();
    let amps2 = amps.clone();
    cases.push(kick_case(
        "w04d_jitter_120",
        PROD,
        kicks(PROD.sample_rate, 10.0, ons, move |i, _| amps[i], kick_real),
        move |out, ks, _| {
            let _ = writeln!(out, "    input amp vs band0 peak (ratio):");
            for (k, a) in ks.iter().zip(&amps2) {
                let _ = writeln!(out, "      {:.3} -> {:.3}  ({:.3})", a, k.peak, k.peak / a);
            }
            let xs: Vec<f32> = amps2.clone();
            let ys: Vec<f32> = ks.iter().map(|k| k.peak).collect();
            let n = xs.len() as f32;
            let mx = xs.iter().sum::<f32>() / n;
            let my = ys.iter().sum::<f32>() / n;
            let cov: f32 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
            let vx: f32 = xs.iter().map(|x| (x - mx).powi(2)).sum();
            let vy: f32 = ys.iter().map(|y| (y - my).powi(2)).sum();
            let _ = writeln!(
                out,
                "    corr(input amp, band0 peak) = {:.3}, output CV {:.3} vs input CV {:.3}",
                cov / (vx * vy).sqrt(),
                cv(&ys),
                cv(&xs)
            );
        },
    ));

    // W4e: simple kicks with ±20 ms random onset jitter, so kick onsets are
    // not phase-locked to the buffer grid.
    cases.push(golden_kick_case("w04e_onset_jitter_120", |out, ks, _| {
        for b in 0..4 {
            let peaks: Vec<f32> = ks.iter().map(|k| k.bands_peak[b]).collect();
            let p = stats(peaks.iter().copied());
            let _ = writeln!(
                out,
                "    band{b} kick peaks: mean {:.3} min {:.3} max {:.3} CV {:.3}",
                p.mean,
                p.min,
                p.max,
                cv(&peaks)
            );
        }
    }));

    // W5: quiet -> loud -> quiet, 10 s each.
    cases.push(golden_kick_case("w05_quiet_loud_quiet", |out, ks, _| {
        report_suppression(out, ks, 10.0);
        report_suppression(out, ks, 20.0);
    }));

    // W6: one 2x hit at 5 s, 20 s run.
    cases.push(golden_kick_case("w06_one_loud_hit", |out, ks, _| {
        report_suppression(out, ks, 5.0)
    }));
    // W6b: one-buffer 2x spike at 5.25 s (between kicks).
    cases.push(golden_kick_case("w06b_one_buffer_spike", |out, ks, _| {
        report_suppression(out, ks, 5.25)
    }));

    // W7: kicks on a sustained 55 Hz tone.
    let ons = onsets(120.0, 0.5, 20.0);
    let mut sig = silence(sr, 20.0);
    sine(&mut sig, sr, 0.0, 20.0, 55.0, 0.4);
    for &on in &ons {
        kick_simple(&mut sig, sr, on, 0.8);
    }
    cases.push(Case {
        name: "w07_kick_on_bass_tone",
        cfg: PROD,
        signal: sig,
        report: Box::new(move |out, rows| {
            let ks = kick_stats(rows, &ons);
            report_kicks(out, "kick+tone", &ks);
        }),
    });

    // W8: 1 kHz sine only.
    let mut sig = silence(sr, 5.0);
    sine(&mut sig, sr, 0.0, 5.0, 1000.0, 0.8);
    cases.push(Case {
        name: "w08_1khz_only",
        cfg: PROD,
        signal: sig,
        report: Box::new(|out, rows| {
            report_steady(out, "1kHz", rows, 3.0, 5.0);
            report_band_peaks(out, "1kHz", rows, 3.0, 5.0);
        }),
    });

    // W9: white-noise 20 ms bursts at 8th notes (120 BPM).
    let mut sig = silence(sr, 5.0);
    let mut rng = Lcg(7);
    for on in onsets(240.0, 0.25, 5.0) {
        let start = (on * sr as f32) as usize;
        for frame in sig.iter_mut().skip(start).take((0.02 * sr as f32) as usize) {
            let v = 0.8 * rng.next_f32();
            *frame = [v, v];
        }
    }
    cases.push(Case {
        name: "w09_noise_bursts",
        cfg: PROD,
        signal: sig,
        report: Box::new(|out, rows| {
            report_steady(out, "noise", rows, 3.0, 5.0);
            report_band_peaks(out, "noise", rows, 3.0, 5.0);
        }),
    });

    // W10: ripple sweep.
    let freqs = [30.0, 40.0, 60.0, 100.0, 150.0, 250.0];
    let mut sig = silence(sr, 3.0 * freqs.len() as f32);
    for (i, &f) in freqs.iter().enumerate() {
        sine(&mut sig, sr, 3.0 * i as f32, 3.0, f, 0.8);
    }
    cases.push(Case {
        name: "w10_ripple_sweep",
        cfg: PROD,
        signal: sig,
        report: Box::new(move |out, rows| {
            for (i, &f) in freqs.iter().enumerate() {
                let a = 3.0 * i as f32 + 2.0;
                report_steady(out, &format!("{f:.0} Hz"), rows, a, a + 1.0);
            }
        }),
    });

    // W11: anti-phase stereo kicks.
    let ons = onsets(120.0, 0.5, 10.0);
    let mut sig = silence(sr, 10.0);
    for &on in &ons {
        kick_simple(&mut sig, sr, on, 0.8);
    }
    for frame in sig.iter_mut() {
        frame[1] = -frame[1];
    }
    cases.push(Case {
        name: "w11_antiphase_stereo",
        cfg: PROD,
        signal: sig,
        report: Box::new(move |out, rows| {
            let ks = kick_stats(rows, &ons);
            report_kicks(out, "antiphase", &ks);
            report_band_peaks(out, "antiphase", rows, 5.0, 10.0);
        }),
    });

    // W12: overload.
    cases.push(kick_case(
        "w12_overload_2x",
        PROD,
        kicks(
            PROD.sample_rate,
            10.0,
            onsets(120.0, 0.5, 10.0),
            |_, _| 2.0,
            kick_simple,
        ),
        |_, _, _| {},
    ));
    // W13: 512-frame buffers.
    cases.push(kick_case(
        "w13_kicks_512frames",
        RunConfig {
            frames: 512,
            ..PROD
        },
        kicks(
            48000,
            10.0,
            onsets(120.0, 0.5, 10.0),
            |_, _| 0.8,
            kick_simple,
        ),
        |_, _, _| {},
    ));
    // W14: 44.1 kHz.
    let cfg441 = RunConfig {
        sample_rate: 44100,
        ..PROD
    };
    cases.push(kick_case(
        "w14_kicks_44k1",
        cfg441,
        kicks(
            cfg441.sample_rate,
            10.0,
            onsets(120.0, 0.5, 10.0),
            |_, _| 0.8,
            kick_simple,
        ),
        |_, _, _| {},
    ));

    // W15: quiet kick under louder 1 kHz content — the sub-bass envelope is
    // small relative to the full-band level.
    cases.push(golden_kick_case("w15_kick_under_1khz", |out, _, rows| {
        report_band_peaks(out, "kick under 1kHz", rows, 15.0, 20.0)
    }));
    // W15b: same kick alone, for comparison.
    cases.push(kick_case(
        "w15b_quiet_kick_alone",
        PROD,
        kicks(
            PROD.sample_rate,
            20.0,
            onsets(120.0, 0.5, 20.0),
            |_, _| 0.3,
            kick_simple,
        ),
        |out, _, rows| report_band_peaks(out, "quiet kick alone", rows, 15.0, 20.0),
    ));

    // W16: -50 dB hiss throughout; kicks for 10 s, then hiss alone for 20 s.
    cases.push(golden_kick_case("w16_hiss_then_silence", |out, _, rows| {
        for (a, b) in [(10.5, 12.0), (15.0, 20.0), (25.0, 30.0)] {
            report_steady(out, "hiss only", rows, a, b);
        }
        report_band_peaks(out, "hiss only", rows, 25.0, 30.0);
    }));

    // W17: kicks, then a 50 Hz sub held for 3 s (a drop), 2 s of silence,
    // then kicks again. How does a sustained sub-bass hit read and release?
    let mut sig = silence(sr, 16.0);
    for on in onsets(120.0, 0.5, 5.0) {
        kick_simple(&mut sig, sr, on, 0.8);
    }
    sine(&mut sig, sr, 5.0, 3.0, 50.0, 0.8);
    for on in onsets(120.0, 10.0, 16.0) {
        kick_simple(&mut sig, sr, on, 0.8);
    }
    cases.push(Case {
        name: "w17_sustained_sub_drop",
        cfg: PROD,
        signal: sig,
        report: Box::new(|out, rows| {
            for (a, b) in [(5.0, 5.5), (5.5, 6.0), (6.0, 7.0), (7.0, 8.0)] {
                report_steady(out, "held sub", rows, a, b);
            }
            report_burst(out, rows, 5.0, 8.0);
            let ks = kick_stats(rows, &onsets(120.0, 10.0, 16.0));
            report_kicks(out, "kicks after the drop", &ks);
        }),
    });

    // W18: sparse kicks — 70 BPM (half-time at 140) from cold, then 30 BPM.
    // Does the ceiling ever catch up when hits are further apart than the
    // confirmation window?
    for (name, bpm) in [("w18a_kicks_70", 70.0), ("w18b_kicks_30", 30.0)] {
        cases.push(kick_case(
            name,
            PROD,
            kicks(
                PROD.sample_rate,
                30.0,
                onsets(bpm, 0.5, 30.0),
                |_, _| 0.8,
                kick_simple,
            ),
            |out, ks, rows| {
                let target = 0.9 * ks[ks.len() - 1].peak_smoothed;
                let caught = rows
                    .iter()
                    .find(|r| r.lowpass().ceiling >= target)
                    .map(|r| r.t);
                let _ = writeln!(
                    out,
                    "    ceiling reaches 90% of kick level at {}",
                    caught.map_or("never".into(), |t| format!("{t:.2}s"))
                );
            },
        ));
    }
    // W19: steady kicks with one 4-hit 16th-note fill at 1.6x at 5 s.
    let mut ons = onsets(120.0, 0.5, 15.0);
    let mut amps: Vec<f32> = vec![0.8; ons.len()];
    for i in 0..4 {
        ons.push(5.0 + 0.125 * i as f32);
        amps.push(1.6);
    }
    let mut order: Vec<usize> = (0..ons.len()).collect();
    order.sort_by(|a, b| ons[*a].partial_cmp(&ons[*b]).expect("finite"));
    let ons: Vec<f32> = order.iter().map(|&i| ons[i]).collect();
    let amps: Vec<f32> = order.iter().map(|&i| amps[i]).collect();
    cases.push(kick_case(
        "w19_fill_1p6x",
        PROD,
        kicks(
            PROD.sample_rate,
            15.0,
            ons,
            move |i, _| amps[i],
            kick_simple,
        ),
        |out, ks, _| report_suppression(out, ks, 5.4),
    ));

    // W20: a tone at the centre of each band in turn. The report is the
    // cross-talk matrix: each band's pre-normalizer level under each tone,
    // in dB relative to that band's level under its own tone.
    let centres: [f32; NUM_OUTPUT_BANDS] =
        [60.0, 265.0, 530.0, 1061.0, 2121.0, 4243.0, 8485.0, 16971.0];
    let mut sig = silence(sr, 2.0 * centres.len() as f32);
    for (i, &f) in centres.iter().enumerate() {
        sine(&mut sig, sr, 2.0 * i as f32, 2.0, f, 0.5);
    }
    cases.push(Case {
        name: "w20_band_centres",
        cfg: PROD,
        signal: sig,
        report: Box::new(move |out, rows| {
            let level = |tone: usize, band: usize| {
                let a = 2.0 * tone as f32 + 1.0;
                stats(window(rows, a, a + 1.0).map(|r| r.stages[band].smoothed)).mean
            };
            let _ = writeln!(
                out,
                "  cross-talk, dB re own band (rows: tone at band centre; cols: band read)"
            );
            let _ = write!(out, "  tone Hz  ");
            for b in 0..NUM_OUTPUT_BANDS {
                let _ = write!(out, "   b{b}  ");
            }
            let _ = writeln!(out);
            for (tone, &f) in centres.iter().enumerate() {
                let _ = write!(out, "  {f:>7.0}  ");
                for b in 0..NUM_OUTPUT_BANDS {
                    let db = 20.0 * (level(tone, b) / level(b, b).max(1e-9)).max(1e-9).log10();
                    let _ = write!(out, "{db:>6.1} ");
                }
                let _ = writeln!(out);
            }
            let _ = writeln!(
                out,
                "  own-band levels (0.5 amplitude tone): {:?}",
                (0..NUM_OUTPUT_BANDS)
                    .map(|b| (level(b, b) * 1000.0).round() / 1000.0)
                    .collect::<Vec<_>>()
            );
        }),
    });

    cases
}

// -------------------------------------------------------------------- music

fn load_clip(path: &str) -> Signal {
    let bytes = fs::read(path).expect("read clip");
    let clip = clip::decode(&bytes).expect("decode clip");
    assert_eq!(clip.sample_rate, PROD.sample_rate, "clip sample rate");
    clip.stereo_frames()
}

/// Per-band peak within 150 ms after each band-0 onset. An onset is a buffer
/// where band 0 rises at least 0.2 above the minimum of the preceding 50 ms.
/// A kick detected in a recording: its onset and each band's peak after it.
struct MusicKick {
    onset: f32,
    peaks: [f32; NUM_OUTPUT_BANDS],
}

fn music_kicks(rows: &[Row]) -> Vec<MusicKick> {
    let mut kicks = Vec::new();
    let mut last_onset = f32::MIN;
    for (i, r) in rows.iter().enumerate() {
        if r.t - last_onset < 0.15 {
            continue;
        }
        let trough = window(rows, r.t - 0.05, r.t)
            .map(|x| x.bands[0])
            .fold(f32::MAX, f32::min);
        if trough == f32::MAX || r.bands[0] - trough < 0.2 {
            continue;
        }
        let mut peaks = [0.0_f32; NUM_OUTPUT_BANDS];
        for x in rows[i..].iter().take_while(|x| x.t < r.t + 0.15) {
            for (p, v) in peaks.iter_mut().zip(x.bands) {
                *p = p.max(v);
            }
        }
        kicks.push(MusicKick { onset: r.t, peaks });
        last_onset = r.t;
    }
    kicks
}

fn rms_distance(a: &[Row], b: &[Row]) -> f32 {
    let n = a.len().min(b.len());
    (a.iter()
        .zip(b)
        .take(n)
        .map(|(x, y)| (x.bands[0] - y.bands[0]).powi(2))
        .sum::<f32>()
        / n as f32)
        .sqrt()
}

fn run_music(out_dir: &Path, path: &str, loops: usize, cfg: RunConfig) {
    let clip = load_clip(path);
    let loop_len = clip.len();
    let mut signal = Vec::with_capacity(loop_len * loops);
    for _ in 0..loops {
        signal.extend_from_slice(&clip);
    }
    let rows = run(cfg, &signal);
    write_csv(&out_dir.join("music_loops.csv"), &rows);

    let buffers_per_loop = loop_len / PROD.frames;
    let mut report = String::new();
    let _ = writeln!(
        report,
        "=== music {path}: {loop_len} frames/loop ({:.2}s), {loops} loops ===",
        loop_len as f32 / PROD.sample_rate as f32,
    );
    let _ = writeln!(
        report,
        "  loop   floor   ceil   |  dfloor   dceil   | b0 dist | b0 min  b0 max  kicks"
    );
    let mut prev: Option<&[Row]> = None;
    let mut prev_end: Option<&Row> = None;
    for l in 0..loops {
        let chunk = &rows[l * buffers_per_loop..((l + 1) * buffers_per_loop).min(rows.len())];
        let end = chunk.last().expect("rows in loop");
        let b0 = stats(chunk.iter().map(|r| r.bands[0]));
        let kicks = music_kicks(chunk).len();
        let deltas = prev_end.map(|p| {
            (
                end.lowpass().floor - p.lowpass().floor,
                end.lowpass().ceiling - p.lowpass().ceiling,
            )
        });
        let dist = prev.map(|p| rms_distance(p, chunk));
        let _ = writeln!(
            report,
            "  {l:>4}   {:.4}  {:.4} | {:>8} {:>8} | {:>7} | {:.3}   {:.3}   {kicks}",
            end.lowpass().floor,
            end.lowpass().ceiling,
            deltas.map_or("-".into(), |d| format!("{:+.4}", d.0)),
            deltas.map_or("-".into(), |d| format!("{:+.4}", d.1)),
            dist.map_or("-".into(), |d| format!("{d:.4}")),
            b0.min,
            b0.max
        );
        prev = Some(chunk);
        prev_end = Some(end);
    }

    // Shift experiment: the same clip with SHIFT samples of silence in front,
    // compared kick by kick against the unshifted run's first loop.
    const SHIFT: usize = 37;
    let mut shifted = vec![[0.0, 0.0]; SHIFT];
    shifted.extend_from_slice(&clip);
    let shifted_rows = run(cfg, &shifted);
    let base_kicks = music_kicks(&rows[..buffers_per_loop]);
    let shift_kicks = music_kicks(&shifted_rows);
    let shift_secs = SHIFT as f32 / PROD.sample_rate as f32;
    let _ = writeln!(
        report,
        "  shift experiment: {SHIFT} samples ({:.2} ms) prepended; {} vs {} kicks detected",
        shift_secs * 1000.0,
        base_kicks.len(),
        shift_kicks.len()
    );
    let mut matched = 0;
    let mut abs_diff = [0.0_f32; NUM_OUTPUT_BANDS];
    let mut max_ratio = [1.0_f32; NUM_OUTPUT_BANDS];
    let mut mean_peak = [0.0_f32; NUM_OUTPUT_BANDS];
    for base in &base_kicks {
        let Some(shifted) = shift_kicks
            .iter()
            .find(|k| (k.onset - shift_secs - base.onset).abs() < 0.01)
        else {
            continue;
        };
        let (a, b) = (&base.peaks, &shifted.peaks);
        matched += 1;
        for band in 0..NUM_OUTPUT_BANDS {
            abs_diff[band] += (a[band] - b[band]).abs();
            mean_peak[band] += a[band];
            let (lo, hi) = if a[band] < b[band] {
                (a[band], b[band])
            } else {
                (b[band], a[band])
            };
            if lo > 0.05 {
                max_ratio[band] = max_ratio[band].max(hi / lo);
            }
        }
    }
    let _ = writeln!(
        report,
        "  {matched} kicks matched; per band: mean|diff|/mean peak, max ratio"
    );
    for band in 0..NUM_OUTPUT_BANDS {
        let _ = writeln!(
            report,
            "    band{band}: {:.3}  {:.2}x",
            abs_diff[band] / mean_peak[band].max(1e-6),
            max_ratio[band]
        );
    }
    print!("{report}");
    fs::write(out_dir.join("music_metrics.txt"), report).expect("write metrics");
}

/// The W6b click at several positions within the beat: does a click's
/// effect on the following kick depend on where it lands?
fn click_offset_sweep(out_dir: &Path, floor_limit: bool) {
    let cfg = RunConfig {
        floor_limit,
        ..PROD
    };
    let sr = PROD.sample_rate;
    let ons = onsets(120.0, 0.5, 8.0);
    let mut report = String::from(
        "=== click offset sweep: 1.6 one-buffer click at 5.0 s + offset; next kick's peak per band ===\n  offset  band0  band1  band2  band3\n",
    );
    for step in 0..10 {
        let offset = 0.05 * step as f32;
        let mut sig = silence(sr, 8.0);
        for &on in &ons {
            kick_simple(&mut sig, sr, on, 0.8);
        }
        let start = ((5.0 + offset) * sr as f32) as usize;
        for frame in sig.iter_mut().skip(start).take(64) {
            *frame = [1.6, 1.6];
        }
        let rows = run(cfg, &sig);
        let ks = kick_stats(&rows, &ons);
        let next = ks
            .iter()
            .find(|k| k.onset > 5.0 + offset + 0.01)
            .expect("a kick after the click");
        let _ = writeln!(
            report,
            "  {offset:.2}    {:.3}  {:.3}  {:.3}  {:.3}",
            next.peak, next.bands_peak[1], next.bands_peak[2], next.bands_peak[3]
        );
    }
    print!("{report}");
    fs::write(out_dir.join("click_offset_sweep.txt"), report).expect("write sweep");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out_dir = args
        .get(1)
        .expect("usage: envelope_suite <out_dir> [--music <clip> --loops N]");
    let out_dir = Path::new(out_dir);
    fs::create_dir_all(out_dir).expect("create out dir");
    let floor_limit = args.iter().any(|a| a == "--floor-limit");

    if let Some(i) = args.iter().position(|a| a == "--music") {
        let path = args.get(i + 1).expect("--music <clip>");
        let loops = args
            .iter()
            .position(|a| a == "--loops")
            .and_then(|j| args.get(j + 1))
            .map_or(6, |n| n.parse().expect("--loops N"));
        run_music(
            out_dir,
            path,
            loops,
            RunConfig {
                floor_limit,
                ..PROD
            },
        );
        return;
    }

    let mut report = String::new();
    click_offset_sweep(out_dir, floor_limit);

    for case in suite() {
        let cfg = RunConfig {
            floor_limit,
            ..case.cfg
        };
        let rows = run(cfg, &case.signal);
        write_csv(&out_dir.join(format!("{}.csv", case.name)), &rows);
        let _ = writeln!(
            report,
            "=== {} ({} Hz, {} frames/buffer, {} buffers) ===",
            case.name,
            case.cfg.sample_rate,
            case.cfg.frames,
            rows.len()
        );
        (case.report)(&mut report, &rows);
        report.push('\n');
    }
    print!("{report}");
    fs::write(out_dir.join("metrics.txt"), report).expect("write metrics");
}
