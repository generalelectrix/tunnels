//! A multi-channel audio processor that derives a single envelope from its input.
//! Provides lowpass filtering and configurable amplitude envelope.
//!
//! Runs a matrix of experimental envelope extraction chains in parallel
//! for visualization and comparison in the audio_vis tool.
//!
//! Every chain follows the same structure:
//!   lowpass (band isolation) → rectifier (sample-wise) → reducer (buffer → scalar)
//!
//! Rectifiers: abs(), Hilbert |z(t)|
//! Reducers: envelope follower, two-stage follower, RMS, median
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{AtomicF32, AudioContext, simple_processor::MonoAudioProcessor};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::debug;
use std::sync::Arc;
use std::time::Duration;

use super::hilbert::HilbertTransform;
use super::ring_buffer::SignalRingBuffer;

/// 16384 slots ≈ 16 seconds of history at ~1kHz buffer rate.
const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// Number of chains in our comparison matrix.
/// 2 rectifiers × 4 reducers = 8, plus input peak = 9 ring buffers.
const NUM_CHAINS: usize = 8;

pub struct ProcessorSettingsInner {
    /// The "production" envelope value (currently: abs + envelope follower).
    pub envelope: AtomicF32,
    pub filter_cutoff: AtomicF32,    // Hz
    pub envelope_attack: AtomicF32,  // sec
    pub envelope_release: AtomicF32, // sec
    /// Input signal gain multiplier (linear scale).
    pub gain: AtomicF32,
    /// Fast pre-envelope attack time (seconds).
    pub fast_attack: AtomicF32,
    /// Fast pre-envelope release time (seconds).
    pub fast_release: AtomicF32,
    /// Sliding window size for RMS and median reducers (seconds).
    pub reducer_window: AtomicF32,

    // === Ring buffers ===
    /// Per-buffer input peak amplitude.
    pub input_peak_history: SignalRingBuffer,
    /// 2×4 matrix of chains, indexed by ChainIdx.
    pub chain_history: [SignalRingBuffer; NUM_CHAINS],
}

/// Index into the chain_history array.
/// Naming: Rectifier_Reducer
pub struct ChainIdx;
impl ChainIdx {
    pub const ABS_ENV: usize = 0;
    pub const ABS_TWO_STAGE: usize = 1;
    pub const ABS_RMS: usize = 2;
    pub const ABS_MEDIAN: usize = 3;
    pub const HILBERT_ENV: usize = 4;
    pub const HILBERT_TWO_STAGE: usize = 5;
    pub const HILBERT_RMS: usize = 6;
    pub const HILBERT_MEDIAN: usize = 7;
}

impl ProcessorSettingsInner {
    const DEFAULT_FILTER_CUTOFF: f32 = 200.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.01;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.1;
    const DEFAULT_FAST_ATTACK: f32 = 0.001;
    const DEFAULT_FAST_RELEASE: f32 = 0.003;
    /// Default sliding window: 10ms.
    const DEFAULT_REDUCER_WINDOW: f32 = 0.010;

    pub fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.fast_attack.set(Self::DEFAULT_FAST_ATTACK);
        self.fast_release.set(Self::DEFAULT_FAST_RELEASE);
        self.reducer_window.set(Self::DEFAULT_REDUCER_WINDOW);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            filter_cutoff: AtomicF32::new(Self::DEFAULT_FILTER_CUTOFF),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
            gain: AtomicF32::new(1.0),
            fast_attack: AtomicF32::new(Self::DEFAULT_FAST_ATTACK),
            fast_release: AtomicF32::new(Self::DEFAULT_FAST_RELEASE),
            reducer_window: AtomicF32::new(Self::DEFAULT_REDUCER_WINDOW),
            input_peak_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            chain_history: std::array::from_fn(|_| {
                SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY)
            }),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// Per-channel state for one rectifier+filter pair.
/// Each chain needs its own lowpass filter instance (independent state).
struct FilteredChannel {
    filter: FilterProcessor<f32>,
}

/// Per-channel state for the Hilbert rectifier path.
struct HilbertChannel {
    filter: FilterProcessor<f32>,
    hilbert: HilbertTransform,
}

/// State for a single-stage envelope follower reducer (one per channel).
struct EnvReducer {
    channels: Vec<EnvelopeFollowerProcessor>,
}

/// State for a two-stage envelope follower reducer.
struct TwoStageReducer {
    fast: Vec<EnvelopeFollowerProcessor>,
    slow: Vec<EnvelopeFollowerProcessor>,
}

/// Maximum window size in samples (200ms at 48kHz).
/// Pre-allocated to this size so window changes don't require reallocation.
const MAX_WINDOW_SAMPLES: usize = 9600;

/// Sliding-window RMS reducer.
///
/// Maintains a ring buffer of squared values and a running sum.
/// Output = sqrt(sum / window_size) each time `output()` is called.
/// Window can be resized at runtime up to MAX_WINDOW_SAMPLES.
struct SlidingRms {
    ring: Vec<f64>,
    head: usize,
    sum_sq: f64,
    len: usize,
    window: usize,
}

impl SlidingRms {
    fn new(window_samples: usize) -> Self {
        let window = window_samples.min(MAX_WINDOW_SAMPLES).max(1);
        Self {
            ring: vec![0.0; MAX_WINDOW_SAMPLES],
            head: 0,
            sum_sq: 0.0,
            len: 0,
            window,
        }
    }

    fn set_window(&mut self, window_samples: usize) {
        let new_window = window_samples.min(MAX_WINDOW_SAMPLES).max(1);
        if new_window != self.window {
            // Reset state — simplest correct approach for window resize.
            self.head = 0;
            self.sum_sq = 0.0;
            self.len = 0;
            self.window = new_window;
        }
    }

    #[inline]
    fn push(&mut self, value: f32) {
        let sq = (value as f64) * (value as f64);
        if self.len == self.window {
            self.sum_sq -= self.ring[self.head];
        } else {
            self.len += 1;
        }
        self.ring[self.head] = sq;
        self.sum_sq += sq;
        self.head = (self.head + 1) % self.window;
    }

    fn output(&self) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        ((self.sum_sq / self.len as f64).max(0.0)).sqrt() as f32
    }
}

/// Sliding-window median reducer.
///
/// Maintains a ring buffer of recent samples and computes the median
/// by sorting a copy each time `output()` is called.
/// Window can be resized at runtime up to MAX_WINDOW_SAMPLES.
struct SlidingMedian {
    ring: Vec<f32>,
    head: usize,
    len: usize,
    window: usize,
    scratch: Vec<f32>,
}

impl SlidingMedian {
    fn new(window_samples: usize) -> Self {
        let window = window_samples.min(MAX_WINDOW_SAMPLES).max(1);
        Self {
            ring: vec![0.0; MAX_WINDOW_SAMPLES],
            head: 0,
            len: 0,
            window,
            scratch: Vec::with_capacity(MAX_WINDOW_SAMPLES),
        }
    }

    fn set_window(&mut self, window_samples: usize) {
        let new_window = window_samples.min(MAX_WINDOW_SAMPLES).max(1);
        if new_window != self.window {
            self.head = 0;
            self.len = 0;
            self.window = new_window;
        }
    }

    #[inline]
    fn push(&mut self, value: f32) {
        self.ring[self.head] = value;
        self.head = (self.head + 1) % self.window;
        if self.len < self.window {
            self.len += 1;
        }
    }

    fn output(&mut self) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        self.scratch.clear();
        // Copy the valid portion of the ring into scratch for sorting.
        if self.len == self.window {
            self.scratch.extend_from_slice(&self.ring[..self.window]);
        } else {
            self.scratch.extend_from_slice(&self.ring[..self.len]);
        }
        self.scratch
            .sort_unstable_by(|a, b| a.total_cmp(b));
        let mid = self.scratch.len() / 2;
        if self.scratch.len() % 2 == 0 {
            (self.scratch[mid - 1] + self.scratch[mid]) / 2.0
        } else {
            self.scratch[mid]
        }
    }
}

pub struct Processor {
    settings: ProcessorSettings,
    filter_cutoff: f32,
    envelope_attack: f32,
    envelope_release: f32,
    fast_attack: f32,
    fast_release: f32,
    channel_count: usize,
    context: AudioContext,

    // === Rectifier paths (one set of per-channel filters each) ===
    // abs() chains share filter instances across reducers that use abs.
    abs_filters: [Vec<FilteredChannel>; 4], // one per abs reducer
    hilbert_filters: [Vec<HilbertChannel>; 4], // one per hilbert reducer

    // === Reducers ===
    // abs() rectifier reducers
    abs_env: EnvReducer,
    abs_two_stage: TwoStageReducer,
    abs_rms: SlidingRms,
    abs_median: SlidingMedian,

    // Hilbert rectifier reducers
    hilbert_env: EnvReducer,
    hilbert_two_stage: TwoStageReducer,
    hilbert_rms: SlidingRms,
    hilbert_median: SlidingMedian,
}

fn make_envelope(
    context: &mut AudioContext,
    attack: Duration,
    release: Duration,
) -> EnvelopeFollowerProcessor {
    let mut env = EnvelopeFollowerProcessor::new(attack, release);
    env.m_prepare(context);
    env
}

fn make_lowpass(context: &mut AudioContext, cutoff: f32) -> FilterProcessor<f32> {
    let mut filter = FilterProcessor::new(FilterType::LowPass);
    filter.set_cutoff(cutoff);
    filter.m_prepare(context);
    filter
}

fn make_filtered_channels(
    context: &mut AudioContext,
    cutoff: f32,
    n: usize,
) -> Vec<FilteredChannel> {
    (0..n)
        .map(|_| FilteredChannel {
            filter: make_lowpass(context, cutoff),
        })
        .collect()
}

fn make_hilbert_channels(
    context: &mut AudioContext,
    cutoff: f32,
    n: usize,
) -> Vec<HilbertChannel> {
    (0..n)
        .map(|_| HilbertChannel {
            filter: make_lowpass(context, cutoff),
            hilbert: HilbertTransform::new(),
        })
        .collect()
}

fn make_env_reducer(
    context: &mut AudioContext,
    attack: Duration,
    release: Duration,
    n: usize,
) -> EnvReducer {
    EnvReducer {
        channels: (0..n)
            .map(|_| make_envelope(context, attack, release))
            .collect(),
    }
}

fn make_two_stage_reducer(
    context: &mut AudioContext,
    fast_attack: Duration,
    fast_release: Duration,
    slow_attack: Duration,
    slow_release: Duration,
    n: usize,
) -> TwoStageReducer {
    TwoStageReducer {
        fast: (0..n)
            .map(|_| make_envelope(context, fast_attack, fast_release))
            .collect(),
        slow: (0..n)
            .map(|_| make_envelope(context, slow_attack, slow_release))
            .collect(),
    }
}

impl Processor {
    pub fn new(handle: ProcessorSettings, sample_rate: u32, channel_count: usize) -> Self {
        let mut context: AudioContext = AudioProcessorSettings {
            sample_rate: sample_rate as f32,
            input_channels: channel_count,
            output_channels: channel_count,
            ..Default::default()
        }
        .into();
        let n = context.settings.input_channels;

        let filter_cutoff = handle.filter_cutoff.get();
        let envelope_attack = handle.envelope_attack.get();
        let envelope_release = handle.envelope_release.get();
        let fast_attack_s = handle.fast_attack.get();
        let fast_release_s = handle.fast_release.get();
        let attack = Duration::from_secs_f32(envelope_attack);
        let release = Duration::from_secs_f32(envelope_release);
        let fast_attack = Duration::from_secs_f32(fast_attack_s);
        let fast_release = Duration::from_secs_f32(fast_release_s);

        let abs_filters = [
            make_filtered_channels(&mut context, filter_cutoff, n),
            make_filtered_channels(&mut context, filter_cutoff, n),
            make_filtered_channels(&mut context, filter_cutoff, n),
            make_filtered_channels(&mut context, filter_cutoff, n),
        ];
        let hilbert_filters = [
            make_hilbert_channels(&mut context, filter_cutoff, n),
            make_hilbert_channels(&mut context, filter_cutoff, n),
            make_hilbert_channels(&mut context, filter_cutoff, n),
            make_hilbert_channels(&mut context, filter_cutoff, n),
        ];
        let abs_env = make_env_reducer(&mut context, attack, release, n);
        let abs_two_stage = make_two_stage_reducer(
            &mut context, fast_attack, fast_release, attack, release, n,
        );
        let hilbert_env = make_env_reducer(&mut context, attack, release, n);
        let hilbert_two_stage = make_two_stage_reducer(
            &mut context, fast_attack, fast_release, attack, release, n,
        );

        let window_secs = handle.reducer_window.get();
        let window_samples = (window_secs * sample_rate as f32) as usize;
        let window_samples = window_samples.max(1);

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            fast_attack: fast_attack_s,
            fast_release: fast_release_s,
            settings: handle,
            channel_count: n,
            context,
            abs_filters,
            hilbert_filters,
            abs_env,
            abs_two_stage,
            abs_rms: SlidingRms::new(window_samples),
            abs_median: SlidingMedian::new(window_samples),
            hilbert_env,
            hilbert_two_stage,
            hilbert_rms: SlidingRms::new(window_samples),
            hilbert_median: SlidingMedian::new(window_samples),
        }
    }

    fn maybe_update_parameters(&mut self) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            debug!("Updating filter cutoff to {new_filter_cutoff}");
            self.filter_cutoff = new_filter_cutoff;
            for filter_set in &mut self.abs_filters {
                for fc in filter_set.iter_mut() {
                    fc.filter.set_cutoff(new_filter_cutoff);
                }
            }
            for filter_set in &mut self.hilbert_filters {
                for hc in filter_set.iter_mut() {
                    hc.filter.set_cutoff(new_filter_cutoff);
                }
            }
        }

        let new_attack = self.settings.envelope_attack.get();
        let new_release = self.settings.envelope_release.get();
        if new_attack != self.envelope_attack || new_release != self.envelope_release {
            debug!("Updating envelope parameters to {new_attack}, {new_release}");
            self.envelope_attack = new_attack;
            self.envelope_release = new_release;
            let attack = Duration::from_secs_f32(new_attack);
            let release = Duration::from_secs_f32(new_release);
            for env in &mut self.abs_env.channels {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
            for env in &mut self.abs_two_stage.slow {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
            for env in &mut self.hilbert_env.channels {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
            for env in &mut self.hilbert_two_stage.slow {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
        }

        let new_fast_attack = self.settings.fast_attack.get();
        let new_fast_release = self.settings.fast_release.get();
        if new_fast_attack != self.fast_attack || new_fast_release != self.fast_release {
            debug!("Updating fast parameters to {new_fast_attack}, {new_fast_release}");
            self.fast_attack = new_fast_attack;
            self.fast_release = new_fast_release;
            let attack = Duration::from_secs_f32(new_fast_attack);
            let release = Duration::from_secs_f32(new_fast_release);
            for env in &mut self.abs_two_stage.fast {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
            for env in &mut self.hilbert_two_stage.fast {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
        }

        // Update sliding window size for RMS/median reducers.
        // sample_rate is baked into the context at construction time.
        let new_window_secs = self.settings.reducer_window.get();
        let sample_rate = self.context.settings.sample_rate;
        let new_window_samples = (new_window_secs * sample_rate) as usize;
        self.abs_rms.set_window(new_window_samples);
        self.abs_median.set_window(new_window_samples);
        self.hilbert_rms.set_window(new_window_samples);
        self.hilbert_median.set_window(new_window_samples);
    }

    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        self.maybe_update_parameters();

        let mut input_peak: f32 = 0.0;
        let gain = self.settings.gain.get();

        for frame in interleaved_buffer.chunks(self.channel_count) {
            for (ch, raw_sample) in frame.iter().enumerate() {
                let sample = *raw_sample * gain;
                input_peak = input_peak.max(sample.abs());

                // --- abs() rectifier path ---
                let abs_vals: [f32; 4] = std::array::from_fn(|i| {
                    self.abs_filters[i][ch]
                        .filter
                        .m_process(&mut self.context, sample)
                        .abs()
                });

                // abs + env
                self.abs_env.channels[ch].m_process(&mut self.context, abs_vals[0]);

                // abs + two-stage
                self.abs_two_stage.fast[ch].m_process(&mut self.context, abs_vals[1]);
                let fast = self.abs_two_stage.fast[ch].handle().state();
                self.abs_two_stage.slow[ch].m_process(&mut self.context, fast);

                // abs + RMS (sliding window)
                self.abs_rms.push(abs_vals[2]);

                // abs + median (sliding window)
                self.abs_median.push(abs_vals[3]);

                // --- Hilbert rectifier path ---
                let hilbert_vals: [f32; 4] = std::array::from_fn(|i| {
                    let filtered =
                        self.hilbert_filters[i][ch]
                            .filter
                            .m_process(&mut self.context, sample);
                    self.hilbert_filters[i][ch]
                        .hilbert
                        .envelope(filtered as f64) as f32
                });

                // hilbert + env
                self.hilbert_env.channels[ch].m_process(&mut self.context, hilbert_vals[0]);

                // hilbert + two-stage
                self.hilbert_two_stage.fast[ch].m_process(&mut self.context, hilbert_vals[1]);
                let fast = self.hilbert_two_stage.fast[ch].handle().state();
                self.hilbert_two_stage.slow[ch].m_process(&mut self.context, fast);

                // hilbert + RMS (sliding window)
                self.hilbert_rms.push(hilbert_vals[2]);

                // hilbert + median (sliding window)
                self.hilbert_median.push(hilbert_vals[3]);
            }
        }

        let ch_count = self.channel_count as f32;

        let abs_env_val = self
            .abs_env
            .channels
            .iter()
            .map(|e| e.handle().state())
            .sum::<f32>()
            / ch_count;

        let abs_two_stage_val = self
            .abs_two_stage
            .slow
            .iter()
            .map(|e| e.handle().state())
            .sum::<f32>()
            / ch_count;

        let hilbert_env_val = self
            .hilbert_env
            .channels
            .iter()
            .map(|e| e.handle().state())
            .sum::<f32>()
            / ch_count;

        let hilbert_two_stage_val = self
            .hilbert_two_stage
            .slow
            .iter()
            .map(|e| e.handle().state())
            .sum::<f32>()
            / ch_count;

        // Write outputs.
        self.settings.envelope.set(abs_env_val);
        self.settings.input_peak_history.push(input_peak);
        self.settings.chain_history[ChainIdx::ABS_ENV].push(abs_env_val);
        self.settings.chain_history[ChainIdx::ABS_TWO_STAGE].push(abs_two_stage_val);
        self.settings.chain_history[ChainIdx::ABS_RMS].push(self.abs_rms.output());
        self.settings.chain_history[ChainIdx::ABS_MEDIAN].push(self.abs_median.output());
        self.settings.chain_history[ChainIdx::HILBERT_ENV].push(hilbert_env_val);
        self.settings.chain_history[ChainIdx::HILBERT_TWO_STAGE].push(hilbert_two_stage_val);
        self.settings.chain_history[ChainIdx::HILBERT_RMS].push(self.hilbert_rms.output());
        self.settings.chain_history[ChainIdx::HILBERT_MEDIAN].push(self.hilbert_median.output());
    }
}
