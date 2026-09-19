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

    let samples: Vec<f32> = clip.samples.iter().map(|&s| s as f32 / 32768.0).collect();
    let mut drained = Vec::new();
    (0..loops)
        .map(|_| {
            let mut band0 = Vec::new();
            for buffer in samples.chunks(FRAMES_PER_BUFFER * channels) {
                processor.process(buffer);
                for stream in &mut streams {
                    drained.clear();
                    stream.drain_into(&mut drained);
                }
                band0.push(settings.envelope.get());
            }
            LoopSummary {
                trim: settings.auto_trim_gain.get(),
                stages: processor.band_stages(0),
                band0,
            }
        })
        .collect()
}

fn rms_distance(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len()) as f32;
    (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / n).sqrt()
}

/// Deltas of one parameter between consecutive loops must shrink in magnitude
/// and keep their sign (no oscillation), ending below `tol`.
fn assert_converged(name: &str, values: &[f32], tol: f32) {
    let deltas: Vec<f32> = values.windows(2).map(|w| w[1] - w[0]).collect();
    let tail = &deltas[deltas.len() - 3..];
    for pair in tail.windows(2) {
        assert!(
            pair[1].abs() <= pair[0].abs() + 1e-6,
            "{name} deltas grow: {deltas:?}"
        );
        assert!(
            pair[0] * pair[1] >= 0.0 || pair[1].abs() < tol,
            "{name} oscillates: {deltas:?}"
        );
    }
    assert!(
        tail[2].abs() < tol,
        "{name} final delta {} not below {tol}: {deltas:?}",
        tail[2]
    );
}

#[test]
fn nightlife_8_bars_converges_without_oscillating() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/nightlife_8bars.clip");
    let clip = clip::decode(&std::fs::read(path).expect("read clip")).expect("decode clip");
    assert_eq!(clip.sample_rate, 48000);
    assert_eq!(clip.channels, 2);

    let loops = run_loops(&clip, 5);

    let trim: Vec<f32> = loops.iter().map(|l| l.trim).collect();
    let floor: Vec<f32> = loops.iter().map(|l| l.stages.floor).collect();
    let ceiling: Vec<f32> = loops.iter().map(|l| l.stages.ceiling).collect();
    assert_converged("trim", &trim, 1e-3);
    assert_converged("floor", &floor, 1e-3);
    assert_converged("ceiling", &ceiling, 1e-3);

    let distances: Vec<f32> = loops
        .windows(2)
        .map(|w| rms_distance(&w[0].band0, &w[1].band0))
        .collect();
    let last = distances[distances.len() - 1];
    assert!(
        last < 0.01,
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
