//! Measures the spectrum's corpus EQ: each band's typical level in dB per
//! octave over a corpus of music, as `spectrum::CORPUS_LEVEL_DB` holds it.
//!
//! Each track runs through a fresh processor at 48 kHz in 64-frame buffers.
//! A track's level per band is the median over the buffers where any band is
//! above `spectrum::SILENCE_DB`; the corpus level is each band's median over
//! the tracks.
//!
//! Usage:
//! `cargo run -p tunnels_audio --release --example spectrum_corpus -- <track>...`
//! where each track is raw little-endian mono `f32` at 48 kHz, optionally
//! followed by `:start:seconds` to take an excerpt. Prints each track's
//! levels and tilt against the corpus, then the corpus table.

use std::num::NonZeroUsize;
use tunnels_audio::bank::{self, NUM_BANDS};
use tunnels_audio::frame_buffer::frame_buffer;
use tunnels_audio::processor::{Processor, ProcessorSettings, envelope_ring_buffers};
use tunnels_audio::spectrum::SILENCE_DB;

const SAMPLE_RATE: u32 = 48_000;
const FRAMES_PER_BUFFER: usize = 64;

/// One track, or an excerpt of one.
struct Track {
    path: String,
    start_secs: f32,
    secs: Option<f32>,
}

impl Track {
    fn parse(arg: &str) -> Self {
        let mut parts = arg.split(':');
        let path = parts.next().expect("a path").to_string();
        let number = |s: Option<&str>| s.map(|s| s.parse::<f32>().expect("seconds"));
        Self {
            path,
            start_secs: number(parts.next()).unwrap_or(0.0),
            secs: number(parts.next()),
        }
    }

    fn samples(&self) -> Vec<f32> {
        let bytes = std::fs::read(&self.path).unwrap_or_else(|e| panic!("{}: {e}", self.path));
        let samples: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f32::from_le_bytes(b))
            .collect();
        let at = |secs: f32| ((secs * SAMPLE_RATE as f32) as usize).min(samples.len());
        let start = at(self.start_secs);
        let end = self
            .secs
            .map_or(samples.len(), |secs| at(self.start_secs + secs));
        samples[start..end].to_vec()
    }

    /// Each band's median level over the track's audible buffers, or nothing
    /// if no buffer was audible.
    fn levels(&self) -> Option<[f32; NUM_BANDS]> {
        let (producer, _) = frame_buffer();
        let mut processor = Processor::new(
            ProcessorSettings::default(),
            SAMPLE_RATE,
            NonZeroUsize::MIN,
            envelope_ring_buffers().producers,
            producer,
        );
        let mut audible: Vec<[f32; NUM_BANDS]> = Vec::new();
        for buffer in self.samples().as_chunks::<FRAMES_PER_BUFFER>().0 {
            processor.process(buffer);
            let levels = processor.spectrum_levels_db();
            if levels.iter().any(|&l| l > SILENCE_DB) {
                audible.push(levels);
            }
        }
        (!audible.is_empty())
            .then(|| std::array::from_fn(|band| median(audible.iter().map(|l| l[band]).collect())))
    }
}

fn median(mut values: Vec<f32>) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// The least-squares slope of per-band levels against octave position, in dB
/// per octave.
fn slope(levels: &[f32; NUM_BANDS]) -> f32 {
    let octave: Vec<f32> = (0..NUM_BANDS)
        .map(|b| (bank::centre(b) / 1000.0).log2())
        .collect();
    let mean = octave.iter().sum::<f32>() / NUM_BANDS as f32;
    let (num, den) = octave
        .iter()
        .zip(levels)
        .fold((0.0, 0.0), |(num, den), (x, l)| {
            (num + (x - mean) * l, den + (x - mean) * (x - mean))
        });
    num / den
}

fn main() {
    let tracks: Vec<Track> = std::env::args().skip(1).map(|a| Track::parse(&a)).collect();
    if tracks.is_empty() {
        eprintln!("usage: spectrum_corpus <mono.f32[:start:seconds]>...");
        return;
    }
    let per_track: Vec<(String, [f32; NUM_BANDS])> = tracks
        .iter()
        .filter_map(|track| {
            let levels = track.levels();
            if levels.is_none() {
                eprintln!("{}: never audible, skipped", track.path);
            }
            levels.map(|l| (track.path.clone(), l))
        })
        .collect();
    let corpus: [f32; NUM_BANDS] =
        std::array::from_fn(|band| median(per_track.iter().map(|(_, l)| l[band]).collect()));

    for (path, levels) in &per_track {
        let deviation: [f32; NUM_BANDS] = std::array::from_fn(|b| levels[b] - corpus[b]);
        let row: Vec<String> = levels.iter().map(|l| format!("{l:.2}")).collect();
        println!(
            "{path}\ttilt {:+.2} dB/oct\t{}",
            slope(&deviation),
            row.join(" ")
        );
    }
    let mut tilts: Vec<f32> = per_track
        .iter()
        .map(|(_, l)| slope(&std::array::from_fn(|b| l[b] - corpus[b])))
        .collect();
    tilts.sort_unstable_by(f32::total_cmp);
    let pct = |p: f32| tilts[((tilts.len() - 1) as f32 * p).round() as usize];
    println!(
        "{} tracks; tilt against the corpus: p10 {:+.2}, median {:+.2}, p90 {:+.2} dB/oct",
        per_track.len(),
        pct(0.1),
        pct(0.5),
        pct(0.9)
    );
    println!("corpus slope {:+.2} dB/oct", slope(&corpus));
    println!("pub const CORPUS_LEVEL_DB: [f32; NUM_BANDS] = [");
    for chunk in corpus.chunks(8) {
        let row: Vec<String> = chunk.iter().map(|l| format!("{l:.2}")).collect();
        println!("    {},", row.join(", "));
    }
    println!("];");
}
