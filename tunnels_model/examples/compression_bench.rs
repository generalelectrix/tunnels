//! What lz4 and zstd cost, and save, on serialized show frames.
//!
//! The question this answers is whether a frame fits in one Ethernet datagram,
//! which is what a move from TCP to UDP multicast would turn from a cosmetic
//! property into a hard one.
//!
//! Dictionaries are trained on frames that are not then measured. A dictionary
//! that has seen the frame it is scoring reports the size of its own memory
//! rather than the size of the traffic.

use std::time::{Duration, Instant};

use tunnels_lib::number::UnipolarFloat;
use tunnels_model::beam::Beam;
use tunnels_model::show_frame::{ShowFrame, fixture};
use tunnels_model::tunnel::fixture::configure_all_noise;

/// One show tick, the interval the console publishes at.
const TICK: Duration = Duration::from_micros(25_300);

/// What a single 1500-byte-MTU datagram leaves for a compressed frame:
/// 1500 less the IP and UDP headers, our length prefix and our magic+version.
const MTU_BUDGET: usize = 1500 - 20 - 8 - 4 - 4;

/// How many frames of each lineage the dictionary is trained on.
const TRAIN_STEPS: usize = 400;

/// How many ticks separate the last trained frame from the first measured one.
///
/// Every integrated angle, smoother and clock phase in a frame has moved on by
/// then, so a dictionary scoring a measured frame is recalling the *shape* of a
/// frame rather than the bytes of one it was shown.
const GAP_STEPS: usize = 800;

/// How many held-out frames each lineage is measured over.
const TEST_STEPS: usize = 20;

/// How many times each frame is compressed, for the timing.
const REPS: usize = 30;

/// The dictionary sizes trained, in bytes. A dictionary ships in both binaries,
/// so its size is part of what it costs.
const DICT_SIZES: [usize; 2] = [4 * 1024, 16 * 1024];

/// A show advancing under its own state updates, sampled a frame at a time.
struct Lineage {
    name: &'static str,
    frame: ShowFrame,
    step: usize,
}

impl Lineage {
    fn new(name: &'static str, frame: ShowFrame) -> Self {
        Self {
            name,
            frame,
            step: 0,
        }
    }

    /// Advance one tick under an audio drive that depends on `wobble`, so that
    /// two runs of the same lineage do not walk the same path.
    fn advance(&mut self, wobble: f64) {
        self.step += 1;
        let envelope = UnipolarFloat::new(0.5 + 0.49 * (self.step as f64 * wobble).sin());
        self.frame.mixer.update_state(TICK, envelope);
        self.frame.audio_envelope = envelope;
        self.frame.frame_number += 1;
    }

    /// The frame as it currently stands, serialized the way the wire does.
    fn payload(&self) -> Vec<u8> {
        postcard::to_allocvec(&self.frame).expect("a frame serializes")
    }
}

fn lineages() -> Vec<Lineage> {
    let mut noise = fixture::max_variation_frame();
    for channel in noise.mixer.channels() {
        if let Beam::Tunnel(tunnel) = &mut channel.beam {
            configure_all_noise(tunnel);
        }
    }
    vec![
        Lineage::new("default", fixture::default_frame()),
        Lineage::new("max variation", fixture::max_variation_frame()),
        Lineage::new("noise", noise),
        Lineage::new("nested looks", fixture::nested_look_frame()),
    ]
}

/// The training corpus and the held-out frames, one bucket per lineage.
struct Split {
    names: Vec<&'static str>,
    train: Vec<Vec<Vec<u8>>>,
    test: Vec<Vec<Vec<u8>>>,
}

fn split() -> Split {
    let mut names = Vec::new();
    let mut train = Vec::new();
    let mut test = Vec::new();
    for mut lineage in lineages() {
        names.push(lineage.name);
        let mut trained = Vec::with_capacity(TRAIN_STEPS);
        for _ in 0..TRAIN_STEPS {
            lineage.advance(0.137);
            trained.push(lineage.payload());
        }
        for _ in 0..GAP_STEPS {
            lineage.advance(0.211);
        }
        let mut held = Vec::with_capacity(TEST_STEPS);
        for _ in 0..TEST_STEPS {
            lineage.advance(0.211);
            held.push(lineage.payload());
        }
        train.push(trained);
        test.push(held);
    }
    Split { names, train, test }
}

/// What one codec did to one bucket of frames.
struct Result {
    raw: f64,
    compressed: f64,
    worst: usize,
    under_budget: usize,
    total: usize,
    compress_us: f64,
    decompress_us: f64,
}

impl Result {
    fn row(&self, label: &str) {
        println!(
            "  {:<26} {:>7.0} {:>8.0} {:>7.2} {:>8} {:>9.1} {:>10.1}  {}",
            label,
            self.raw,
            self.compressed,
            self.raw / self.compressed,
            self.worst,
            self.compress_us,
            self.decompress_us,
            if self.under_budget == self.total {
                "all fit".to_string()
            } else if self.under_budget == 0 {
                "none fit".to_string()
            } else {
                format!("{}/{} fit", self.under_budget, self.total)
            },
        );
    }
}

trait Codec {
    fn compress(&mut self, plain: &[u8], out: &mut Vec<u8>);
    fn decompress(&mut self, compressed: &[u8], plain_len: usize, out: &mut Vec<u8>);
}

/// The incumbent, driven exactly the way `FrameEncoder` drives it.
struct Lz4;

impl Codec for Lz4 {
    fn compress(&mut self, plain: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.resize(lz4_flex::block::get_maximum_output_size(plain.len()), 0);
        let n = lz4_flex::block::compress_into(plain, out).expect("the buffer is large enough");
        out.truncate(n);
    }

    fn decompress(&mut self, compressed: &[u8], plain_len: usize, out: &mut Vec<u8>) {
        *out = lz4_flex::decompress(compressed, plain_len).expect("round trip");
    }
}

struct Zstd {
    compressor: zstd::bulk::Compressor<'static>,
    decompressor: zstd::bulk::Decompressor<'static>,
}

impl Zstd {
    fn new(level: i32, dictionary: Option<&[u8]>) -> Self {
        let (compressor, decompressor) = match dictionary {
            None => (
                zstd::bulk::Compressor::new(level).unwrap(),
                zstd::bulk::Decompressor::new().unwrap(),
            ),
            Some(dict) => (
                zstd::bulk::Compressor::with_dictionary(level, dict).unwrap(),
                zstd::bulk::Decompressor::with_dictionary(dict).unwrap(),
            ),
        };
        Self {
            compressor,
            decompressor,
        }
    }
}

impl Codec for Zstd {
    fn compress(&mut self, plain: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.reserve(zstd::zstd_safe::compress_bound(plain.len()));
        self.compressor
            .compress_to_buffer(plain, out)
            .expect("compression succeeds");
    }

    fn decompress(&mut self, compressed: &[u8], plain_len: usize, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(plain_len);
        self.decompressor
            .decompress_to_buffer(compressed, out)
            .expect("round trip");
    }
}

fn measure(codec: &mut dyn Codec, frames: &[Vec<u8>]) -> Result {
    let mut out = Vec::new();
    let mut back = Vec::new();

    let mut raw = 0usize;
    let mut compressed = 0usize;
    let mut worst = 0usize;
    let mut under_budget = 0usize;
    for frame in frames {
        codec.compress(frame, &mut out);
        codec.decompress(&out, frame.len(), &mut back);
        assert_eq!(&back, frame, "codec did not round trip");
        raw += frame.len();
        compressed += out.len();
        worst = worst.max(out.len());
        if out.len() <= MTU_BUDGET {
            under_budget += 1;
        }
    }

    let start = Instant::now();
    for _ in 0..REPS {
        for frame in frames {
            codec.compress(frame, &mut out);
            std::hint::black_box(&out);
        }
    }
    let compress_us = start.elapsed().as_secs_f64() * 1e6 / (REPS * frames.len()) as f64;

    let squeezed: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| {
            codec.compress(f, &mut out);
            out.clone()
        })
        .collect();
    let start = Instant::now();
    for _ in 0..REPS {
        for (small, frame) in squeezed.iter().zip(frames) {
            codec.decompress(small, frame.len(), &mut back);
            std::hint::black_box(&back);
        }
    }
    let decompress_us = start.elapsed().as_secs_f64() * 1e6 / (REPS * frames.len()) as f64;

    Result {
        raw: raw as f64 / frames.len() as f64,
        compressed: compressed as f64 / frames.len() as f64,
        worst,
        under_budget,
        total: frames.len(),
        compress_us,
        decompress_us,
    }
}

fn train(samples: &[&[u8]], size: usize) -> Vec<u8> {
    zstd::dict::from_samples(samples, size).expect("dictionary trains")
}

fn header(title: &str) {
    println!("\n{title}");
    println!(
        "  {:<26} {:>7} {:>8} {:>7} {:>8} {:>9} {:>10}  {}",
        "codec", "raw B", "comp B", "ratio", "worst B", "comp us", "decomp us", "MTU"
    );
}

fn main() {
    println!("MTU budget for a compressed frame: {MTU_BUDGET} bytes");

    let Split { names, train: corpus, test } = split();

    println!("\nfixtures, pristine (no dictionary involved)");
    for named in fixture::all() {
        let plain = postcard::to_allocvec(&named.frame).unwrap();
        let wire = named.frame.encode().unwrap();
        println!(
            "  {:<16} postcard {:>6} B, lz4 wire {:>6} B",
            named.name,
            plain.len(),
            wire.len()
        );
    }

    // The dictionary the whole corpus trains, held out only in time: every
    // lineage contributed frames, none of them the ones being scored.
    let all: Vec<&[u8]> = corpus.iter().flatten().map(|v| v.as_slice()).collect();
    let train_bytes: usize = all.iter().map(|s| s.len()).sum();
    println!(
        "\ncorpus: {} frames, {} bytes, drawn from steps 1..={} of {} lineages",
        all.len(),
        train_bytes,
        TRAIN_STEPS,
        names.len()
    );
    println!(
        "held out: steps {}..{} of each lineage, under a different audio drive",
        TRAIN_STEPS + GAP_STEPS + 1,
        TRAIN_STEPS + GAP_STEPS + TEST_STEPS
    );

    let time_dicts: Vec<(usize, Vec<u8>)> = DICT_SIZES
        .iter()
        .map(|&size| {
            let dict = train(&all, size);
            println!("  time-held-out dictionary: asked {size} B, got {} B", dict.len());
            (size, dict)
        })
        .collect();

    // One dictionary per lineage, trained on every lineage but that one, to
    // bound how much of the win is the dictionary having memorized a structure
    // rather than having learned the model's vocabulary.
    let loo_dicts: Vec<Vec<u8>> = (0..names.len())
        .map(|skip| {
            let samples: Vec<&[u8]> = corpus
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .flat_map(|(_, frames)| frames.iter().map(|v| v.as_slice()))
                .collect();
            let dict = train(&samples, DICT_SIZES[1]);
            println!(
                "  leave-out-{:<14} dictionary: {} B",
                names[skip],
                dict.len()
            );
            dict
        })
        .collect();

    for (i, name) in names.iter().enumerate() {
        header(&format!("{name} — {} held-out frames", test[i].len()));
        let frames = &test[i];
        measure(&mut Lz4, frames).row("lz4_flex block");
        for level in [-5, 1, 3] {
            measure(&mut Zstd::new(level, None), frames).row(&format!("zstd {level}"));
        }
        for (size, dict) in &time_dicts {
            for level in [-5, 1, 3] {
                measure(&mut Zstd::new(level, Some(dict)), frames)
                    .row(&format!("zstd {level} + dict {}K", size / 1024));
            }
        }
        for level in [1, 3] {
            measure(&mut Zstd::new(level, Some(&loo_dicts[i])), frames)
                .row(&format!("zstd {level} + dict 16K (LOO)"));
        }
    }
}
