//! Pinned envelope responses. Each case runs a signal through the processor
//! and compares every role's output against a checked-in golden (see
//! `common/golden.rs`).
//!
//! `UPDATE_GOLDENS=1 cargo test -p tunnels_audio --test envelope_golden`
//! rewrites the goldens from the current output.

mod common;

use common::golden::{self, Golden};
use common::{clip, offline, signals};
use std::path::Path;
use tunnels_audio::processor::ProcessorSettings;
use tunnels_audio::roles::{NUM_ROLES, Role};

const FRAMES_PER_BUFFER: usize = 64;
/// Every `STRIDE`th buffer is kept: 187 Hz against an 8 ms output smoother.
const STRIDE: usize = 4;
/// Loops of the music clip; startup and the converged state both count.
const MUSIC_LOOPS: usize = 3;

fn run(signal: &[[f32; 2]]) -> Golden {
    let mut golden = Golden::new(
        signals::SAMPLE_RATE as f32 / FRAMES_PER_BUFFER as f32,
        STRIDE,
        NUM_ROLES,
    );
    offline::run_stereo(
        signals::SAMPLE_RATE,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        signal,
        |i, _, frame| {
            if i % STRIDE == 0 {
                golden.push(frame.roles().map(f32::from));
            }
        },
    );
    golden
}

fn check(name: &str, actual: &Golden) -> Option<String> {
    golden::check(name, "env", actual, |r| Role::ALL[r].label().to_string())
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

    let clip_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/its_not_a_toy_8bars.clip");
    let clip = clip::decode(&std::fs::read(clip_path).expect("read clip")).expect("decode clip");
    assert_eq!(clip.sample_rate, signals::SAMPLE_RATE);
    let one_loop = clip.stereo_frames();
    let mut signal = Vec::with_capacity(one_loop.len() * MUSIC_LOOPS);
    for _ in 0..MUSIC_LOOPS {
        signal.extend_from_slice(&one_loop);
    }
    if let Some(report) = check("its_not_a_toy_8bars_x3", &run(&signal)) {
        failures.push(report);
    }

    assert!(
        failures.is_empty(),
        "envelope goldens differ:\n{}",
        failures.join("\n")
    );
}
