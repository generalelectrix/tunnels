//! Throughput of the envelope chain on real music, in release mode.
//!
//! Times `Processor::process` over the packed clip in production-sized
//! buffers, and the wavelet decomposition alone, reporting nanoseconds per
//! input frame and the fraction of real time consumed.
//!
//! Usage: `cargo run -p tunnels_audio --release --example bench -- <file.clip>`

// The shared codec is included by path; this binary only uses its decoder.
#[allow(dead_code)]
#[path = "../tests/common/clip.rs"]
mod clip;

use std::time::{Duration, Instant};

use tunnels_audio::processor::{Processor, ProcessorSettings, envelope_ring_buffers};
use tunnels_audio::wavelet::WaveletDecomposition;

const FRAMES_PER_BUFFER: usize = 64;
const PASSES: usize = 5;

/// Wall time of the fastest of `PASSES` runs of `f`.
fn fastest(mut f: impl FnMut()) -> Duration {
    (0..PASSES)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .expect("at least one pass")
}

fn report(label: &str, elapsed: Duration, frames: usize, sample_rate: u32) {
    let audio_secs = frames as f64 / sample_rate as f64;
    println!(
        "{label:<28} {:>7.1} ns/frame   {:>6.2}% of real time",
        elapsed.as_nanos() as f64 / frames as f64,
        100.0 * elapsed.as_secs_f64() / audio_secs
    );
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: bench <file.clip>");
    let clip = clip::decode(&std::fs::read(path).expect("read clip")).expect("decode clip");
    let channels = clip.channels as usize;
    let frames = clip.frames();
    let stereo = clip.stereo_frames();
    let samples: Vec<f32> = stereo.iter().flatten().copied().collect();
    let mono: Vec<f32> = stereo.iter().map(|[l, r]| (l + r) / 2.0).collect();
    println!(
        "{frames} frames at {} Hz x{channels}, {FRAMES_PER_BUFFER}-frame buffers, best of {PASSES}",
        clip.sample_rate
    );

    let buffers = envelope_ring_buffers();
    let mut streams = buffers.streams;
    let mut processor = Processor::new(
        ProcessorSettings::default(),
        clip.sample_rate,
        channels,
        buffers.producers,
    );
    let mut drained = Vec::new();
    let elapsed = fastest(|| {
        for buffer in samples.chunks(FRAMES_PER_BUFFER * channels) {
            processor.process(buffer);
            for stream in &mut streams {
                drained.clear();
                stream.drain_into(&mut drained);
            }
        }
    });
    report("Processor::process", elapsed, frames, clip.sample_rate);

    let mut wavelet = WaveletDecomposition::new();
    let mut sink = 0.0_f32;
    let elapsed = fastest(|| {
        for &s in &mono {
            wavelet.push(s, |_, v| sink += v);
        }
    });
    report(
        "WaveletDecomposition::push",
        elapsed,
        frames,
        clip.sample_rate,
    );
    std::hint::black_box(sink);
}
