//! Long-term behaviour on real music: loop a packed clip through one
//! processor and require the adaptive parameters (input trim, normalizer
//! floor and ceiling) to converge without oscillating, and the Bass role's
//! output to become loop-periodic.

mod common;

use common::{clip, offline};
use std::path::Path;
use tunnels_audio::processor::{BandStages, ProcessorSettings};
use tunnels_audio::roles::Role;

const FRAMES_PER_BUFFER: usize = 64;

/// Adaptive state at the end of one pass through the clip, plus the Bass
/// role's output over that pass.
struct LoopSummary {
    stages: BandStages,
    trim_gain: f32,
    bass: Vec<f32>,
}

fn run_loops(clip: &clip::Clip, loops: usize) -> Vec<LoopSummary> {
    // One continuous stream, as a device would deliver it: the buffer grid
    // does not restart at the loop seam.
    let one_loop = clip.stereo_frames();
    let mut signal = Vec::with_capacity(one_loop.len() * loops);
    for _ in 0..loops {
        signal.extend_from_slice(&one_loop);
    }
    let buffers_per_loop = clip.frames() / FRAMES_PER_BUFFER;

    let mut summaries = Vec::with_capacity(loops);
    let mut bass = Vec::with_capacity(buffers_per_loop);
    offline::run_stereo(
        clip.sample_rate,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        &signal,
        |_, processor, outputs| {
            bass.push(outputs[Role::Bass.index()]);
            if bass.len() == buffers_per_loop {
                let meter = processor.input_meter().upgrade();
                let meter = meter.expect("a live processor's meter upgrades");
                summaries.push(LoopSummary {
                    stages: processor.stages(Role::Bass).expect("Bass is a level role"),
                    trim_gain: 10f32.powf(meter.trim_db() / 20.0),
                    bass: std::mem::take(&mut bass),
                });
            }
        },
    );
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
fn music_clip_converges_without_oscillating() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/its_not_a_toy_8bars.clip");
    let clip = clip::decode(&std::fs::read(path).expect("read clip")).expect("decode clip");
    assert_eq!(clip.sample_rate, 48000);
    assert_eq!(clip.channels, 2);

    // About two minutes of music: a dozen floor half-lives, enough for the
    // slowest adaptive stage to settle whatever the clip's length.
    let loops = run_loops(
        &clip,
        (120.0 * clip.sample_rate as f32 / clip.frames() as f32).ceil() as usize,
    );

    let trim: Vec<f32> = loops.iter().map(|l| l.trim_gain).collect();
    assert_converged("trim gain", 1.0, &trim, 0.01);
    let floor: Vec<f32> = loops.iter().map(|l| l.stages.floor).collect();
    let ceiling: Vec<f32> = loops.iter().map(|l| l.stages.ceiling).collect();
    assert_converged("floor", 0.0, &floor, 0.01);
    assert_converged("ceiling", 0.0, &ceiling, 0.01);

    let distances: Vec<f32> = loops
        .windows(2)
        .map(|w| rms_distance(&w[0].bass, &w[1].bass))
        .collect();
    let last = distances[distances.len() - 1];
    assert!(
        last < 0.02,
        "Bass output not loop-periodic: distances {distances:?}"
    );
    assert!(
        distances[0] > last,
        "Bass output did not settle: distances {distances:?}"
    );

    let final_bass = &loops[loops.len() - 1].bass;
    let max = final_bass.iter().copied().fold(0.0, f32::max);
    let min = final_bass.iter().copied().fold(1.0, f32::min);
    assert!(max >= 0.95, "Bass never reached full scale: max {max}");
    assert!(min <= 0.05, "Bass never returned to zero: min {min}");
}
