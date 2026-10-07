//! The roles do not depend on where the music falls relative to the buffer
//! grid: the same clip with a few samples of silence in front gives the same
//! hits.

mod common;

use common::{clip, offline};
use std::path::Path;
use tunnels_audio::processor::ProcessorSettings;
use tunnels_audio::roles::{NUM_ROLES, Role};

const FRAMES_PER_BUFFER: usize = 64;

/// Every role's output per buffer, for the clip looped `loops` times with
/// `offset` frames of silence in front.
fn run(clip: &clip::Clip, loops: usize, offset: usize) -> Vec<[f32; NUM_ROLES]> {
    let one_loop = clip.stereo_frames();
    let mut signal = vec![[0.0, 0.0]; offset];
    for _ in 0..loops {
        signal.extend_from_slice(&one_loop);
    }
    let mut out = Vec::new();
    offline::run_stereo(
        clip.sample_rate,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        &signal,
        |_, _, outputs| out.push(*outputs),
    );
    out
}

/// A role's hits after `from`: each local maximum above 0.3, with its time in
/// seconds of music.
fn hits(out: &[[f32; NUM_ROLES]], role: Role, rate: f32, from: f32) -> Vec<f32> {
    let v: Vec<f32> = out.iter().map(|o| o[role.index()]).collect();
    (1..v.len() - 1)
        .filter(|&i| v[i] > 0.3 && v[i] >= v[i - 1] && v[i] > v[i + 1])
        .map(|i| i as f32 / rate)
        .filter(|&t| t > from)
        .collect()
}

/// A role's highest output within `buffers` buffers of a moment in the music.
fn height(
    out: &[[f32; NUM_ROLES]],
    role: Role,
    rate: f32,
    offset_secs: f32,
    t: f32,
    buffers: f32,
) -> f32 {
    let centre = ((t + offset_secs) * rate).round() as isize;
    let reach = buffers.ceil() as isize + 1;
    (centre - reach..=centre + reach)
        .filter(|&i| i >= 0 && (i as usize) < out.len())
        .filter(|&i| ((i as f32 / rate - offset_secs) - t).abs() <= buffers / rate)
        .map(|i| out[i as usize][role.index()])
        .fold(0.0, f32::max)
}

/// Kick and Hats hit at the same height whether the clip starts on a buffer
/// boundary or a few samples after one.
///
/// A hit's height is read as the highest output within a few buffers of it.
/// The shifted run's buffers fall at different moments, so a window's edge
/// can take in a neighbouring peak in one run and not the other; the aligned
/// height is therefore checked to lie between the shifted run's heights over
/// a slightly narrower and a slightly wider window, which no edge can decide.
#[test]
fn hits_do_not_depend_on_buffer_alignment() {
    const TOLERANCE: f32 = 0.05;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/its_not_a_toy_8bars.clip");
    let clip = clip::decode(&std::fs::read(path).expect("read clip")).expect("decode clip");
    let rate = clip.sample_rate as f32 / FRAMES_PER_BUFFER as f32;
    // Skip the first loop, while the trim and the peaks are still settling.
    let from = clip.frames() as f32 / clip.sample_rate as f32;

    let base = run(&clip, 2, 0);
    for offset in [13, 37, 51] {
        let shifted = run(&clip, 2, offset);
        let offset_secs = offset as f32 / clip.sample_rate as f32;
        for role in [Role::Kick, Role::Hats] {
            let ts = hits(&base, role, rate, from);
            assert!(
                ts.len() > 10,
                "{}: only {} hits to compare",
                role.label(),
                ts.len()
            );
            for t in ts {
                let h = height(&base, role, rate, 0.0, t, 4.0);
                let narrow = height(&shifted, role, rate, offset_secs, t, 3.0);
                let wide = height(&shifted, role, rate, offset_secs, t, 5.0);
                assert!(
                    h > narrow - TOLERANCE && h < wide + TOLERANCE,
                    "{} with {offset} samples of offset: the hit at {t:.2} s reads {h:.3} aligned, \
                     and {narrow:.3} to {wide:.3} shifted",
                    role.label(),
                );
            }
        }
    }
}
