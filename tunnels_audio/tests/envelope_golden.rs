//! Pinned envelope responses. Each case runs a signal the chain was tuned
//! on through the processor and compares the eight normalized bands against
//! a checked-in golden. A behaviour change shows up as a per-band report of
//! how far, where, and how often the output moved, and the actual output is
//! written beside the golden for plotting.
//!
//! To accept a change: `UPDATE_GOLDENS=1 cargo test -p tunnels_audio --test
//! envelope_golden`, then include a before/after plot with the new bytes.

mod common;

use common::{clip, signals};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use tunnels_audio::processor::{
    ENVELOPE_HISTORY_CAPACITY, NUM_OUTPUT_BANDS, Processor, ProcessorSettings,
};
use tunnels_audio::ring_buffer::{EnvelopeProducer, envelope_ring_buffer};

const FRAMES_PER_BUFFER: usize = 64;
/// Every `STRIDE`th buffer is kept: 187 Hz against an 8 ms output smoother.
const STRIDE: usize = 4;
/// Largest step allowed between golden and actual, in 8-bit envelope units.
/// Two steps is 0.8 % of full scale: room for libm differences between
/// platforms, far below any behavioural change.
const TOLERANCE: u8 = 2;
/// Loops of the music clip; startup and the converged state both count.
const MUSIC_LOOPS: usize = 3;

/// One case's eight normalized bands, quantized to 8 bits, one row per
/// `STRIDE` buffers.
#[derive(Serialize, Deserialize)]
struct Golden {
    update_rate: f32,
    stride: usize,
    /// `bands[band][row]`.
    bands: Vec<Vec<u8>>,
}

fn quantize(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn run(signal: &[[f32; 2]]) -> Golden {
    let settings = ProcessorSettings::default();
    let mut producers = Vec::with_capacity(NUM_OUTPUT_BANDS);
    let mut streams = Vec::with_capacity(NUM_OUTPUT_BANDS);
    for _ in 0..NUM_OUTPUT_BANDS {
        let (p, c) = envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY);
        producers.push(p);
        streams.push(c);
    }
    let producers: [EnvelopeProducer; NUM_OUTPUT_BANDS] =
        producers.try_into().ok().expect("correct count");
    let mut processor = Processor::new(settings, signals::SAMPLE_RATE, 2, producers);

    let mut bands = vec![Vec::new(); NUM_OUTPUT_BANDS];
    let mut interleaved = Vec::with_capacity(FRAMES_PER_BUFFER * 2);
    let mut drained = Vec::new();
    for (i, chunk) in signal.chunks(FRAMES_PER_BUFFER).enumerate() {
        interleaved.clear();
        for frame in chunk {
            interleaved.extend_from_slice(frame);
        }
        processor.process(&interleaved);
        for (band, stream) in streams.iter_mut().enumerate() {
            drained.clear();
            stream.drain_into(&mut drained);
            if i % STRIDE == 0 {
                bands[band].push(quantize(*drained.last().expect("one value per buffer")));
            }
        }
    }
    Golden {
        update_rate: signals::SAMPLE_RATE as f32 / FRAMES_PER_BUFFER as f32,
        stride: STRIDE,
        bands,
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/golden")
}

/// Compare one case against its golden, returning a report if it differs
/// beyond tolerance. Writes the actual output beside the golden when it does.
fn check(name: &str, actual: &Golden) -> Option<String> {
    let path = golden_dir().join(format!("{name}.env"));
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&path, postcard::to_allocvec(actual).expect("serialize"))
            .expect("write golden");
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}; run with UPDATE_GOLDENS=1",
            path.display()
        )
    });
    let golden: Golden = postcard::from_bytes(&bytes).expect("decode golden");
    assert_eq!(golden.stride, actual.stride, "{name}: stride changed");
    assert_eq!(
        golden.update_rate, actual.update_rate,
        "{name}: update rate changed"
    );

    let secs_per_row = actual.stride as f32 / actual.update_rate;
    let mut report = String::new();
    for (band, (g, a)) in golden.bands.iter().zip(&actual.bands).enumerate() {
        if g.len() != a.len() {
            let _ = writeln!(
                report,
                "  band {band}: {} rows in golden, {} actual",
                g.len(),
                a.len()
            );
            continue;
        }
        let mut worst = 0u8;
        let mut worst_at = 0usize;
        let mut over = 0usize;
        for (i, (x, y)) in g.iter().zip(a).enumerate() {
            let d = x.abs_diff(*y);
            if d > worst {
                worst = d;
                worst_at = i;
            }
            if d > TOLERANCE {
                over += 1;
            }
        }
        if over > 0 {
            let _ = writeln!(
                report,
                "  band {band}: {over} of {} rows differ by more than {TOLERANCE}/255; worst {worst}/255 at {:.2}s (golden {}, actual {})",
                g.len(),
                worst_at as f32 * secs_per_row,
                g[worst_at],
                a[worst_at]
            );
        }
    }
    if report.is_empty() {
        return None;
    }
    let actual_path = golden_dir().join(format!("{name}.actual.env"));
    std::fs::write(
        &actual_path,
        postcard::to_allocvec(actual).expect("serialize"),
    )
    .expect("write actual");
    Some(format!(
        "{name} (actual written to {}):\n{report}",
        actual_path.display()
    ))
}

#[test]
fn envelope_goldens() {
    let mut failures = Vec::new();

    for name in signals::GOLDEN_CASES {
        let case = signals::golden_case(name).expect("known case");
        if let Some(report) = check(name, &run(&case.signal)) {
            failures.push(report);
        }
    }

    let clip_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/nightlife_8bars.clip");
    let clip = clip::decode(&std::fs::read(clip_path).expect("read clip")).expect("decode clip");
    assert_eq!(clip.sample_rate, signals::SAMPLE_RATE);
    assert_eq!(clip.channels, 2);
    let one_loop: Vec<[f32; 2]> = clip
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[l, r]| [l as f32 / 32768.0, r as f32 / 32768.0])
        .collect();
    let mut signal = Vec::with_capacity(one_loop.len() * MUSIC_LOOPS);
    for _ in 0..MUSIC_LOOPS {
        signal.extend_from_slice(&one_loop);
    }
    if let Some(report) = check("nightlife_8bars_x3", &run(&signal)) {
        failures.push(report);
    }

    assert!(
        failures.is_empty(),
        "envelope goldens differ:\n{}",
        failures.join("\n")
    );
}
