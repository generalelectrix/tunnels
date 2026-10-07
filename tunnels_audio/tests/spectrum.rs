//! The spectrum on whole signals: pink noise reads level but for the
//! corpus's curve, and the response to a music clip is pinned by a golden
//! (see `common/golden.rs`).
//!
//! `UPDATE_GOLDENS=1 cargo test -p tunnels_audio --test spectrum` rewrites
//! the golden from the current output.

mod common;

use common::golden::{self, Golden};
use common::signals::{self, Lcg};
use common::{clip, offline};
use std::path::Path;
use tunnels_audio::bank::{self, NUM_BANDS};
use tunnels_audio::processor::ProcessorSettings;
use tunnels_audio::spectrum::CORPUS_LEVEL_DB;

const FRAMES_PER_BUFFER: usize = 64;

/// Pink noise from a fixed seed: equal power in every octave.
fn pink_noise(secs: f32, amplitude: f32) -> signals::Signal {
    let mut rng = Lcg(7);
    // Paul Kellet's filter: white noise through a sum of one-poles spaced
    // across the audio band, within 0.05 dB of -3 dB per octave.
    let mut b = [0.0_f32; 7];
    (0..(secs * signals::SAMPLE_RATE as f32) as usize)
        .map(|_| {
            let white = rng.next_f32();
            b[0] = 0.99886 * b[0] + white * 0.0555179;
            b[1] = 0.99332 * b[1] + white * 0.0750759;
            b[2] = 0.969 * b[2] + white * 0.153852;
            b[3] = 0.8665 * b[3] + white * 0.3104856;
            b[4] = 0.55 * b[4] + white * 0.5329522;
            b[5] = -0.7616 * b[5] - white * 0.0168980;
            let pink = b[..6].iter().sum::<f32>() + b[6] + white * 0.5362;
            b[6] = white * 0.115926;
            let v = amplitude * pink;
            [v, v]
        })
        .collect()
}

/// Least-squares fit of per-band values against octave position: the slope
/// per octave, and each band's residual about the line.
struct LineFit {
    slope: f32,
    residuals: [f32; NUM_BANDS],
}

impl LineFit {
    fn new(values: &[f32; NUM_BANDS]) -> Self {
        let octave: [f32; NUM_BANDS] = std::array::from_fn(|b| (bank::centre(b) / 1000.0).log2());
        let mean_x = octave.iter().sum::<f32>() / NUM_BANDS as f32;
        let mean_y = values.iter().sum::<f32>() / NUM_BANDS as f32;
        let (num, den) = (0..NUM_BANDS).fold((0.0, 0.0), |(num, den), b| {
            let x = octave[b] - mean_x;
            (num + x * (values[b] - mean_y), den + x * x)
        });
        let slope = num / den;
        Self {
            slope,
            residuals: std::array::from_fn(|b| values[b] - mean_y - slope * (octave[b] - mean_x)),
        }
    }
}

/// Pink noise has equal power in every octave, so once the tilt has taken
/// off the slope of its difference from the corpus, it reads level across
/// the bands but for the corpus's own curve about its slope, inverted.
#[test]
fn pink_noise_reads_flat_but_for_the_corpus_curve() {
    const SECS: f32 = 90.0;
    /// The averaging starts here, once the tilt has had two time constants.
    const FROM_SECS: f32 = 70.0;
    /// The window the outputs span, in dB.
    const WINDOW_DB: f32 = 30.0;
    let signal = pink_noise(SECS, 0.1);
    let from = (FROM_SECS * signals::SAMPLE_RATE as f32) as usize / FRAMES_PER_BUFFER;
    let mut sums = [0.0_f64; NUM_BANDS];
    let mut count = 0;
    offline::run_stereo(
        signals::SAMPLE_RATE,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        &signal,
        |i, _, frame| {
            if i >= from {
                for (sum, v) in sums.iter_mut().zip(frame.spectrum()) {
                    *sum += v.val();
                }
                count += 1;
            }
        },
    );
    let reading_db: [f32; NUM_BANDS] =
        std::array::from_fn(|b| WINDOW_DB * (sums[b] / count as f64) as f32);
    let reading = LineFit::new(&reading_db);
    let corpus = LineFit::new(&CORPUS_LEVEL_DB);
    assert!(
        reading.slope.abs() < 0.5,
        "pink noise reads with a slope of {:.2} dB/oct",
        reading.slope
    );
    for band in 0..NUM_BANDS {
        let expected = -corpus.residuals[band];
        let read = reading.residuals[band];
        assert!(
            (read - expected).abs() < 2.0,
            "the {:.0} Hz band reads {read:+.1} dB about the line, not {expected:+.1} dB",
            bank::centre(band)
        );
    }
}

/// Every `STRIDE`th buffer is kept.
const STRIDE: usize = 8;

#[test]
fn spectrum_golden() {
    let clip_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/its_not_a_toy_8bars.clip");
    let clip = clip::decode(&std::fs::read(clip_path).expect("read clip")).expect("decode clip");
    let one_loop = clip.stereo_frames();
    let signal = [one_loop.as_slice(), one_loop.as_slice()].concat();
    let mut actual = Golden::new(
        clip.sample_rate as f32 / FRAMES_PER_BUFFER as f32,
        STRIDE,
        NUM_BANDS,
    );
    offline::run_stereo(
        clip.sample_rate,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        &signal,
        |i, _, frame| {
            if i % STRIDE == 0 {
                actual.push(frame.spectrum().map(|v| v.val() as f32));
            }
        },
    );
    if let Some(report) = golden::check("its_not_a_toy_8bars_x2", "spec", &actual, |b| {
        format!("{:.0} Hz", bank::centre(b))
    }) {
        panic!("spectrum golden differs:\n{report}");
    }
}
