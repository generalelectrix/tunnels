//! Offline characterisation of the envelope chain.
//!
//! Drives `Processor` with synthetic waveforms under production conditions
//! (stereo, 64-frame buffers, auto-trim on) and records every stage per
//! buffer: raw peak, trim gain, the lowpass band's smoothed envelope, floor
//! and ceiling, and all eight normalized bands. Each waveform writes a CSV to
//! the output directory and prints a metrics block to stdout.
//!
//! Usage: `cargo run -p tunnels_audio --release --example envelope_suite -- <out_dir>`
//!
//! With `--music <file.clip> --loops N` it instead loops a packed real-music
//! clip N times through one processor and prints a per-loop convergence table
//! for the adaptive parameters, plus a shift experiment: the same clip with a
//! few samples of silence prepended, comparing per-band kick peaks.
//!
//! `--floor-limit` runs either mode with the normalizer floor in limit mode
//! instead of the default average mode.

// Shared with the integration tests by path; this binary uses only parts.
#[allow(dead_code)]
#[path = "../tests/common/clip.rs"]
mod clip;
#[allow(dead_code)]
#[path = "../tests/common/signals.rs"]
mod signals;

use signals::{Lcg, Signal, kick_real, kick_simple, onsets, silence, sine};

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tunnels_audio::processor::{
    ENVELOPE_HISTORY_CAPACITY, NUM_OUTPUT_BANDS, Processor, ProcessorSettings, TrackingMode,
};

/// Whether runs use the limit-mode floor (`--floor-limit`).
static FLOOR_LIMIT: AtomicBool = AtomicBool::new(false);
use tunnels_audio::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};

/// One recorded buffer.
struct Row {
    t: f32,
    raw_peak: f32,
    trim: f32,
    smoothed: f32,
    floor: f32,
    ceiling: f32,
    bands: [f32; NUM_OUTPUT_BANDS],
    /// Per-band (smoothed, floor, ceiling) for the wavelet bands 1..8.
    stages: [[f32; 3]; NUM_OUTPUT_BANDS],
}

#[derive(Clone, Copy)]
struct RunConfig {
    sample_rate: u32,
    frames: usize,
}

const PROD: RunConfig = RunConfig {
    sample_rate: 48000,
    frames: 64,
};

fn run(cfg: RunConfig, signal: &Signal) -> Vec<Row> {
    let settings = ProcessorSettings::default();
    if FLOOR_LIMIT.load(Ordering::Relaxed) {
        settings
            .norm_floor_mode
            .store(TrackingMode::Limit, Ordering::Relaxed);
    }
    let mut producers = Vec::with_capacity(NUM_OUTPUT_BANDS);
    let mut streams: Vec<EnvelopeStream> = Vec::with_capacity(NUM_OUTPUT_BANDS);
    for _ in 0..NUM_OUTPUT_BANDS {
        let (p, c) = envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY);
        producers.push(p);
        streams.push(c);
    }
    let producers: [EnvelopeProducer; NUM_OUTPUT_BANDS] =
        producers.try_into().ok().expect("correct count");
    let mut processor = Processor::new(settings.clone(), cfg.sample_rate, 2, producers);

    let mut rows = Vec::with_capacity(signal.len() / cfg.frames + 1);
    let mut interleaved = Vec::with_capacity(cfg.frames * 2);
    let mut drained = Vec::new();
    for (buf_idx, chunk) in signal.chunks(cfg.frames).enumerate() {
        interleaved.clear();
        let mut raw_peak = 0.0_f32;
        for frame in chunk {
            raw_peak = raw_peak.max(frame[0].abs()).max(frame[1].abs());
            interleaved.extend_from_slice(frame);
        }
        processor.process(&interleaved);
        let mut bands = [0.0; NUM_OUTPUT_BANDS];
        for (band, stream) in streams.iter_mut().enumerate() {
            drained.clear();
            stream.drain_into(&mut drained);
            bands[band] = *drained.last().expect("one value per buffer");
        }
        let stages = processor.band_stages(0);
        let all_stages = std::array::from_fn(|b| {
            let st = processor.band_stages(b);
            [st.smoothed, st.floor, st.ceiling]
        });
        rows.push(Row {
            t: (buf_idx * cfg.frames) as f32 / cfg.sample_rate as f32,
            raw_peak,
            trim: settings.auto_trim_gain.get(),
            smoothed: stages.smoothed,
            floor: stages.floor,
            ceiling: stages.ceiling,
            bands,
            stages: all_stages,
        });
    }
    rows
}

fn write_csv(path: &Path, rows: &[Row]) {
    let mut s = String::from("t,raw_peak,trim,smoothed,floor,ceiling");
    for b in 0..NUM_OUTPUT_BANDS {
        let _ = write!(s, ",band{b}");
    }
    for b in 1..NUM_OUTPUT_BANDS {
        let _ = write!(s, ",sm{b},fl{b},ceil{b}");
    }
    s.push('\n');
    for r in rows {
        let _ = write!(
            s,
            "{:.5},{:.5},{:.5},{:.5},{:.5},{:.5}",
            r.t, r.raw_peak, r.trim, r.smoothed, r.floor, r.ceiling
        );
        for b in r.bands {
            let _ = write!(s, ",{b:.5}");
        }
        for st in &r.stages[1..] {
            let _ = write!(s, ",{:.5},{:.5},{:.5}", st[0], st[1], st[2]);
        }
        s.push('\n');
    }
    fs::write(path, s).expect("write csv");
}

// ------------------------------------------------------------------ metrics

fn band0(rows: &[Row]) -> Vec<(f32, f32)> {
    rows.iter().map(|r| (r.t, r.bands[0])).collect()
}

fn window(rows: &[Row], from: f32, to: f32) -> impl Iterator<Item = &Row> {
    rows.iter().filter(move |r| r.t >= from && r.t < to)
}

fn stats(vals: impl Iterator<Item = f32>) -> (f32, f32, f32) {
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
    (sum / n.max(1.0), min, max)
}

struct KickStat {
    onset: f32,
    peak: f32,
    peak_smoothed: f32,
    trough: f32,
    trim: f32,
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
                peak_smoothed = peak_smoothed.max(r.smoothed);
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
                trim: at_onset.trim,
                ceiling: at_onset.ceiling,
                floor: at_onset.floor,
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
    let (mean, min, max) = stats(peaks.iter().copied());
    let (tmean, tmin, tmax) = stats(ks.iter().map(|k| k.trough));
    let (lmean, lmin, lmax) = stats(ks.iter().map(|k| k.latency_ms));
    let registered = ks.iter().filter(|k| k.peak - k.trough >= 0.2).count();
    let _ = writeln!(
        out,
        "  {label}: {} kicks, {registered} registered (rise>=0.2)\n    band0 peak mean {mean:.3} min {min:.3} max {max:.3} CV {:.3}\n    trough mean {tmean:.3} min {tmin:.3} max {tmax:.3}\n    onset->peak latency ms mean {lmean:.1} min {lmin:.1} max {lmax:.1}",
        ks.len(),
        cv(&peaks)
    );
    let _ = writeln!(
        out,
        "    per-kick: onset  band0  smoothed  floor  ceil   trim   | band1  band2  band3"
    );
    for k in ks {
        let _ = writeln!(
            out,
            "             {:5.2}  {:.3}  {:.3}     {:.3}  {:.3}  {:.3}  | {:.3}  {:.3}  {:.3}",
            k.onset,
            k.peak,
            k.peak_smoothed,
            k.floor,
            k.ceiling,
            k.trim,
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
    let (bm, bmin, bmax) = stats(window(rows, from, to).map(|r| r.bands[0]));
    let (sm, smin, smax) = stats(window(rows, from, to).map(|r| r.smoothed));
    let last = window(rows, from, to).last().expect("rows in window");
    let _ = writeln!(
        out,
        "  {label} [{from:.1}-{to:.1}s]: band0 mean {bm:.3} ripple {:.4} | smoothed mean {sm:.4} ripple {:.4} | floor {:.4} ceil {:.4} trim {:.3}",
        bmax - bmin,
        smax - smin,
        last.floor,
        last.ceiling,
        last.trim
    );
}

fn report_burst(out: &mut String, rows: &[Row], on: f32, off: f32) {
    let b = band0(rows);
    let peak = b
        .iter()
        .filter(|(t, _)| *t >= on && *t < off)
        .map(|(_, v)| *v)
        .fold(0.0, f32::max);
    let cross = |thr: f32, from: f32, rising: bool| -> Option<f32> {
        b.iter()
            .find(|(t, v)| *t >= from && if rising { *v >= thr } else { *v <= thr })
            .map(|(t, _)| *t)
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

fn kick_case(
    name: &'static str,
    cfg: RunConfig,
    secs: f32,
    ons: Vec<f32>,
    amp: impl Fn(usize, f32) -> f32,
    real: bool,
    extra: impl Fn(&mut String, &[KickStat], &[Row]) + 'static,
) -> Case {
    let sr = cfg.sample_rate;
    let mut sig = silence(sr, secs);
    for (i, &on) in ons.iter().enumerate() {
        if real {
            kick_real(&mut sig, sr, on, amp(i, on));
        } else {
            kick_simple(&mut sig, sr, on, amp(i, on));
        }
    }
    Case {
        name,
        cfg,
        signal: sig,
        report: Box::new(move |out, rows| {
            let ks = kick_stats(rows, &ons);
            report_kicks(out, name, &ks);
            extra(out, &ks, rows);
        }),
    }
}

/// A case built from one of the shared golden signals, reported as kicks.
fn golden_kick_case(
    name: &'static str,
    extra: impl Fn(&mut String, &[KickStat], &[Row]) + 'static,
) -> Case {
    let signals::KickSignal { signal, onsets } = signals::golden_case(name).expect("golden case");
    Case {
        name,
        cfg: PROD,
        signal,
        report: Box::new(move |out, rows| {
            let ks = kick_stats(rows, &onsets);
            report_kicks(out, name, &ks);
            extra(out, &ks, rows);
        }),
    }
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
            let (_, _, max) = stats(window(rows, 0.3, 0.5).map(|r| r.bands[0]));
            let (_, _, smax) = stats(window(rows, 0.3, 0.5).map(|r| r.smoothed));
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
        10.0,
        onsets(120.0, 0.5, 10.0),
        |_, _| 0.8,
        false,
        |out, ks, rows| {
            // Startup: how long until the ceiling is within 10 % of the
            // kicks it is normalizing.
            let target = 0.9 * ks[ks.len() - 1].peak_smoothed;
            let caught = rows.iter().find(|r| r.ceiling >= target).map(|r| r.t);
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
        10.0,
        ons,
        |_, _| 0.8,
        true,
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
        10.0,
        ons,
        move |i, _| amps[i],
        true,
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
    // not phase-locked to any decimation grid.
    cases.push(golden_kick_case("w04e_onset_jitter_120", |out, ks, _| {
        for b in 0..4 {
            let peaks: Vec<f32> = ks.iter().map(|k| k.bands_peak[b]).collect();
            let (mean, min, max) = stats(peaks.iter().copied());
            let _ = writeln!(
                out,
                "    band{b} kick peaks: mean {mean:.3} min {min:.3} max {max:.3} CV {:.3}",
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
        10.0,
        onsets(120.0, 0.5, 10.0),
        |_, _| 2.0,
        false,
        |_, _, _| {},
    ));
    // W13: 512-frame buffers.
    cases.push(kick_case(
        "w13_kicks_512frames",
        RunConfig {
            sample_rate: 48000,
            frames: 512,
        },
        10.0,
        onsets(120.0, 0.5, 10.0),
        |_, _| 0.8,
        false,
        |_, _, _| {},
    ));
    // W14: 44.1 kHz.
    let cfg441 = RunConfig {
        sample_rate: 44100,
        frames: 64,
    };
    cases.push(kick_case(
        "w14_kicks_44k1",
        cfg441,
        10.0,
        onsets(120.0, 0.5, 10.0),
        |_, _| 0.8,
        false,
        |_, _, _| {},
    ));

    // W15: quiet kick under louder 1 kHz content — the full-band peak sets
    // the trim, the sub-bass envelope is small relative to it.
    cases.push(golden_kick_case("w15_kick_under_1khz", |out, _, rows| {
        report_band_peaks(out, "kick under 1kHz", rows, 15.0, 20.0)
    }));
    // W15b: same kick alone, for comparison.
    cases.push(kick_case(
        "w15b_quiet_kick_alone",
        PROD,
        20.0,
        onsets(120.0, 0.5, 20.0),
        |_, _| 0.3,
        false,
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

    cases
}

// -------------------------------------------------------------------- music

fn load_clip(path: &str) -> Signal {
    let bytes = fs::read(path).expect("read clip");
    let clip = clip::decode(&bytes).expect("decode clip");
    assert_eq!(clip.sample_rate, PROD.sample_rate, "clip sample rate");
    assert_eq!(clip.channels, 2, "clip channels");
    clip.samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[l, r]| [l as f32 / 32768.0, r as f32 / 32768.0])
        .collect()
}

/// Per-band peak within 150 ms after each band-0 onset. An onset is a buffer
/// where band 0 rises at least 0.2 above the minimum of the preceding 50 ms.
fn music_kicks(rows: &[Row]) -> Vec<(f32, [f32; NUM_OUTPUT_BANDS])> {
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
        kicks.push((r.t, peaks));
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

fn run_music(out_dir: &Path, path: &str, loops: usize) {
    let clip = load_clip(path);
    let loop_len = clip.len();
    let mut signal = Vec::with_capacity(loop_len * loops);
    for _ in 0..loops {
        signal.extend_from_slice(&clip);
    }
    let rows = run(PROD, &signal);
    write_csv(&out_dir.join("music_loops.csv"), &rows);

    let buffers_per_loop = loop_len / PROD.frames;
    let mut report = String::new();
    let _ = writeln!(
        report,
        "=== music {path}: {loop_len} frames/loop ({:.2}s), {loops} loops, frames mod 128 = {} ===",
        loop_len as f32 / PROD.sample_rate as f32,
        loop_len % 128
    );
    let _ = writeln!(
        report,
        "  loop   trim    floor   ceil   |  dtrim    dfloor   dceil   | b0 dist | b0 min  b0 max  kicks"
    );
    let mut prev: Option<&[Row]> = None;
    let mut prev_end: Option<&Row> = None;
    for l in 0..loops {
        let chunk = &rows[l * buffers_per_loop..((l + 1) * buffers_per_loop).min(rows.len())];
        let end = chunk.last().expect("rows in loop");
        let (_, bmin, bmax) = stats(chunk.iter().map(|r| r.bands[0]));
        let kicks = music_kicks(chunk).len();
        let deltas = prev_end.map(|p| {
            (
                end.trim - p.trim,
                end.floor - p.floor,
                end.ceiling - p.ceiling,
            )
        });
        let dist = prev.map(|p| rms_distance(p, chunk));
        let _ = writeln!(
            report,
            "  {l:>4}   {:.4}  {:.4}  {:.4} | {:>8} {:>8} {:>8} | {:>7} | {:.3}   {:.3}   {kicks}",
            end.trim,
            end.floor,
            end.ceiling,
            deltas.map_or("-".into(), |d| format!("{:+.4}", d.0)),
            deltas.map_or("-".into(), |d| format!("{:+.4}", d.1)),
            deltas.map_or("-".into(), |d| format!("{:+.4}", d.2)),
            dist.map_or("-".into(), |d| format!("{d:.4}")),
            bmin,
            bmax
        );
        prev = Some(chunk);
        prev_end = Some(end);
    }

    // Shift experiment: the same clip with SHIFT samples of silence in front,
    // compared kick by kick against the unshifted run's first loop.
    const SHIFT: usize = 37;
    let mut shifted = vec![[0.0, 0.0]; SHIFT];
    shifted.extend_from_slice(&clip);
    let shifted_rows = run(PROD, &shifted);
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
    for (t, a) in &base_kicks {
        let Some((_, b)) = shift_kicks
            .iter()
            .find(|(ts, _)| (ts - shift_secs - t).abs() < 0.01)
        else {
            continue;
        };
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out_dir = args
        .get(1)
        .expect("usage: envelope_suite <out_dir> [--music <clip> --loops N]");
    let out_dir = Path::new(out_dir);
    fs::create_dir_all(out_dir).expect("create out dir");
    FLOOR_LIMIT.store(args.iter().any(|a| a == "--floor-limit"), Ordering::Relaxed);

    if let Some(i) = args.iter().position(|a| a == "--music") {
        let path = args.get(i + 1).expect("--music <clip>");
        let loops = args
            .iter()
            .position(|a| a == "--loops")
            .and_then(|j| args.get(j + 1))
            .map_or(6, |n| n.parse().expect("--loops N"));
        run_music(out_dir, path, loops);
        return;
    }

    let mut report = String::new();
    for case in suite() {
        let rows = run(case.cfg, &case.signal);
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
