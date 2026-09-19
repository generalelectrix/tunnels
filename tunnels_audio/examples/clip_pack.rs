//! Pack raw interleaved 16-bit little-endian PCM (as produced by
//! `ffmpeg -f s16le`) into the test clip container.
//!
//! Usage: `clip_pack <in.pcm> <sample_rate> <channels> <out.clip>`

// The shared codec is included by path; this binary only uses its encoder.
#[allow(dead_code)]
#[path = "../tests/common/clip.rs"]
mod clip;

use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, rate, channels, output] = args.as_slice() else {
        eprintln!("usage: clip_pack <in.pcm> <sample_rate> <channels> <out.clip>");
        std::process::exit(2);
    };
    let sample_rate: u32 = rate.parse().expect("sample rate");
    let channels: u16 = channels.parse().expect("channel count");
    let raw = fs::read(input).expect("read input");
    let samples: Vec<i16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| i16::from_le_bytes(b))
        .collect();
    let clip = clip::Clip {
        sample_rate,
        channels,
        samples,
    };
    let packed = clip::encode(&clip);
    fs::write(output, &packed).expect("write output");
    println!(
        "{} frames at {} Hz x{}: {} raw bytes -> {} packed ({:.0}%)",
        clip.frames(),
        sample_rate,
        channels,
        raw.len(),
        packed.len(),
        100.0 * packed.len() as f64 / raw.len() as f64
    );
}
