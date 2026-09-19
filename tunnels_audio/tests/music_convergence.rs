//! Long-term behaviour on real music: loop a packed clip through one
//! processor and require the adaptive parameters (auto-trim gain, normalizer
//! floor and ceiling) to converge without oscillating, and the lowpass band's
//! output to become loop-periodic.

mod common;

use common::clip;
use std::path::Path;
use tunnels_audio::processor::{
    BandStages, ENVELOPE_HISTORY_CAPACITY, NUM_OUTPUT_BANDS, Processor, ProcessorSettings,
};
use tunnels_audio::ring_buffer::{EnvelopeProducer, envelope_ring_buffer};

const FRAMES_PER_BUFFER: usize = 64;

/// Adaptive state at the end of one pass through the clip, plus the lowpass
/// band's output over that pass.
struct LoopSummary {
    trim: f32,
    stages: BandStages,
    band0: Vec<f32>,
}

fn run_loops(clip: &clip::Clip, loops: usize) -> Vec<LoopSummary> {
    let channels = clip.channels as usize;
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
    let mut processor = Processor::new(settings.clone(), clip.sample_rate, channels, producers);

    // One continuous stream, as a device would deliver it: the buffer grid
    // does not restart at the loop seam.
    let mut samples = Vec::with_capacity(clip.samples.len() * loops);
    for _ in 0..loops {
        samples.extend(clip.samples.iter().map(|&s| s as f32 / 32768.0));
    }
    let buffers_per_loop = clip.frames() / FRAMES_PER_BUFFER;
    let mut drained = Vec::new();
    let mut summaries = Vec::with_capacity(loops);
    let mut band0 = Vec::with_capacity(buffers_per_loop);
    for buffer in samples.chunks(FRAMES_PER_BUFFER * channels) {
        processor.process(buffer);
        for stream in &mut streams {
            drained.clear();
            stream.drain_into(&mut drained);
        }
        band0.push(settings.envelope.get());
        if band0.len() == buffers_per_loop {
            summaries.push(LoopSummary {
                trim: settings.auto_trim_gain.get(),
                stages: processor.band_stages(0).expect("band 0"),
                band0: std::mem::take(&mut band0),
            });
        }
    }
    summaries
}

fn rms_distance(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len()) as f32;
    (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / n).sqrt()
}

/// One parameter has converged when its deltas between consecutive loops
/// have settled: each of the last three is below `tol` (no oscillation
/// larger than that) and so is their sum (no drift). It must also have
/// left its cold-start value `initial` by more than `tol`, or the test is
/// not exercising anything.
fn assert_converged(name: &str, initial: f32, values: &[f32], tol: f32) {
    let deltas: Vec<f32> = values.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        (values[0] - initial).abs() > tol,
        "{name} barely moved from {initial}: {values:?}"
    );
    let tail = &deltas[deltas.len() - 3..];
    for d in tail {
        assert!(d.abs() < tol, "{name} still moving by {d}: {deltas:?}");
    }
    let drift: f32 = tail.iter().sum();
    assert!(drift.abs() < tol, "{name} drifts by {drift}: {deltas:?}");
}

#[test]
fn nightlife_8_bars_converges_without_oscillating() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/nightlife_8bars.clip");
    let clip = clip::decode(&std::fs::read(path).expect("read clip")).expect("decode clip");
    assert_eq!(clip.sample_rate, 48000);
    assert_eq!(clip.channels, 2);

    let loops = run_loops(&clip, 6);

    let trim: Vec<f32> = loops.iter().map(|l| l.trim).collect();
    let floor: Vec<f32> = loops.iter().map(|l| l.stages.floor).collect();
    let ceiling: Vec<f32> = loops.iter().map(|l| l.stages.ceiling).collect();
    assert_converged("trim", 1.0, &trim, 0.02);
    assert_converged("floor", 0.0, &floor, 0.01);
    assert_converged("ceiling", 0.0, &ceiling, 0.01);

    let distances: Vec<f32> = loops
        .windows(2)
        .map(|w| rms_distance(&w[0].band0, &w[1].band0))
        .collect();
    let last = distances[distances.len() - 1];
    assert!(
        last < 0.02,
        "band 0 output not loop-periodic: distances {distances:?}"
    );
    assert!(
        distances[0] > last,
        "band 0 output did not settle: distances {distances:?}"
    );

    let final_band0 = &loops[loops.len() - 1].band0;
    let max = final_band0.iter().copied().fold(0.0, f32::max);
    let min = final_band0.iter().copied().fold(1.0, f32::min);
    assert!(max >= 0.95, "band 0 never reached full scale: max {max}");
    assert!(min <= 0.05, "band 0 never returned to zero: min {min}");
}
