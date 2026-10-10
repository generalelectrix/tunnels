//! The spectrum on whole signals: pink noise reads level but for the
//! corpus's curve, a beating tone's fast ripple is rejected while its
//! rhythm is kept, and the responses to a music clip and to a set of
//! transients are pinned by goldens (see `common/golden.rs`).
//!
//! `UPDATE_GOLDENS=1 cargo test -p tunnels_audio --test spectrum` rewrites
//! the goldens from the current output.

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

/// A tone at `freq` from `start` for `len` seconds, faded in and out over a
/// millisecond so that its edges stay within its band.
fn burst(sig: &mut signals::Signal, start: f32, len: f32, freq: f32, amp: f32) {
    const FADE: f32 = 0.001;
    signals::add_mono(sig, signals::SAMPLE_RATE, start, |t| {
        (t < len).then(|| {
            let edge = (t / FADE).min((len - t) / FADE).min(1.0);
            let gain = 0.5 - 0.5 * (std::f32::consts::PI * edge).cos();
            amp * gain * (2.0 * std::f32::consts::PI * freq * t).sin()
        })
    });
}

/// The band holding the steady reference tone in the transient and ripple
/// signals, louder than anything else, so the ceiling sits on it and the
/// bands under test read inside the window rather than at its top.
const REFERENCE_BAND: usize = 4;

/// The reference tone's amplitude.
const REFERENCE_AMP: f32 = 0.2;

/// The amplitude the bands under test are played at: about 14 dB under the
/// reference, so they read near 0.8.
const TEST_AMP: f32 = 0.04;

/// Two equal tones `beat_hz` apart, centred in `band`, from `start` for
/// `len` seconds: a tone whose envelope swings from full to nothing
/// `beat_hz` times a second.
fn beating(sig: &mut signals::Signal, band: usize, start: f32, len: f32, beat_hz: f32) {
    let f = bank::centre(band);
    for freq in [f - beat_hz / 2.0, f + beat_hz / 2.0] {
        burst(sig, start, len, freq, TEST_AMP);
    }
}

/// A signal of `secs` seconds holding only the reference tone.
fn reference(secs: f32) -> signals::Signal {
    let mut signal = signals::silence(signals::SAMPLE_RATE, secs);
    burst(
        &mut signal,
        0.0,
        secs,
        bank::centre(REFERENCE_BAND),
        REFERENCE_AMP,
    );
    signal
}

/// The bands the transient signal plays in: the lowest, one in the middle,
/// and one high in the treble.
const TRANSIENT_BANDS: [usize; 3] = [0, 12, 22];

/// The transient signal: the reference tone throughout, and in each of
/// [`TRANSIENT_BANDS`] in turn, a 4 s section of
/// - at 0 s, a single 20 ms burst;
/// - at 0.3 s, a 200 ms tone;
/// - at 0.8 s, six 8 ms hits 25 ms apart (closer than the peak hold);
/// - at 1.2 s, six hits 40 ms apart (just past it);
/// - at 1.6 s, six hits 60 ms apart;
/// - at 2 s, a second of a tone beating at 8 Hz;
/// - at 3 s, a second of a tone beating at 45 Hz.
///
/// The first section starts after a second of the reference alone.
fn transient_signal() -> signals::Signal {
    const LEAD: f32 = 1.0;
    const SECTION: f32 = 4.0;
    let mut signal = reference(LEAD + SECTION * TRANSIENT_BANDS.len() as f32);
    for (i, &band) in TRANSIENT_BANDS.iter().enumerate() {
        let at = LEAD + SECTION * i as f32;
        let freq = bank::centre(band);
        burst(&mut signal, at, 0.020, freq, TEST_AMP);
        burst(&mut signal, at + 0.3, 0.200, freq, TEST_AMP);
        for (from, spacing) in [(0.8, 0.025), (1.2, 0.040), (1.6, 0.060)] {
            for hit in 0..6 {
                burst(
                    &mut signal,
                    at + from + spacing * hit as f32,
                    0.008,
                    freq,
                    TEST_AMP,
                );
            }
        }
        beating(&mut signal, band, at + 2.0, 1.0, 8.0);
        beating(&mut signal, band, at + 3.0, 1.0, 45.0);
    }
    signal
}

/// The spectrum's response to isolated bursts, a held tone, rolls of hits at
/// spacings either side of the peak hold, and tones beating at 8 Hz and
/// 45 Hz, every buffer, pinned by a golden: how fast a band rises, how long
/// it holds, how it falls, when a roll's hits merge, and how much of a
/// beating envelope reaches the output.
#[test]
fn spectrum_transient_golden() {
    let signal = transient_signal();
    let mut actual = Golden::new(
        signals::SAMPLE_RATE as f32 / FRAMES_PER_BUFFER as f32,
        1,
        NUM_BANDS,
    );
    offline::run_stereo(
        signals::SAMPLE_RATE,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        &signal,
        |_, _, frame| actual.push(frame.spectrum().map(|v| v.val() as f32)),
    );
    if let Some(report) = golden::check("transients", "spec", &actual, |b| {
        format!("{:.0} Hz", bank::centre(b))
    }) {
        panic!("spectrum transient golden differs:\n{report}");
    }
}

/// How far a band's output swings while a tone in it beats at `beat_hz`: the
/// spread between its 5th and 95th percentiles over the last two of four
/// seconds, under the reference tone.
fn beat_swing(band: usize, beat_hz: f32) -> f32 {
    const SECS: f32 = 4.0;
    const FROM_SECS: f32 = 2.0;
    let mut signal = reference(SECS);
    beating(&mut signal, band, 0.0, SECS, beat_hz);
    let from = (FROM_SECS * signals::SAMPLE_RATE as f32) as usize / FRAMES_PER_BUFFER;
    let mut out = Vec::new();
    offline::run_stereo(
        signals::SAMPLE_RATE,
        FRAMES_PER_BUFFER,
        ProcessorSettings::default(),
        &signal,
        |i, _, frame| {
            if i >= from {
                out.push(frame.spectrum()[band].val() as f32);
            }
        },
    );
    out.sort_by(f32::total_cmp);
    let at = |q: f32| out[((out.len() - 1) as f32 * q).round() as usize];
    at(0.95) - at(0.05)
}

/// A band whose envelope fluctuates faster than a 60 fps display can show
/// reads nearly steady, while one fluctuating at a rhythmic rate keeps its
/// motion: a tone beating at 45 Hz swings the output by less than
/// `RIPPLE_LIMIT`, and one beating at 8 Hz by more than `RHYTHM_FLOOR`.
#[test]
fn fast_ripple_is_rejected_and_rhythm_kept() {
    /// The most a 45 Hz beat may swing the output: 0.75 dB of the window.
    const RIPPLE_LIMIT: f32 = 0.025;
    /// The least an 8 Hz beat must swing it: 3 dB of the window.
    const RHYTHM_FLOOR: f32 = 0.10;
    const BAND: usize = 12;
    let ripple = beat_swing(BAND, 45.0);
    assert!(
        ripple < RIPPLE_LIMIT,
        "a 45 Hz beat swings the {:.0} Hz band by {ripple:.3}",
        bank::centre(BAND)
    );
    let rhythm = beat_swing(BAND, 8.0);
    assert!(
        rhythm > RHYTHM_FLOOR,
        "an 8 Hz beat swings the {:.0} Hz band by only {rhythm:.3}",
        bank::centre(BAND)
    );
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
