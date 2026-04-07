//! A multi-channel audio processor that derives a single envelope from its input.
//!
//! Processing chain per channel:
//!   lowpass (band isolation) → Hilbert |z(t)| (rectification) → fast envelope → slow envelope
//!
//! The Hilbert transform provides clean rectification without carrier ripple.
//! The two-stage envelope follower separates peak tracking (fast) from
//! smoothing (slow), giving better transient response than a single stage.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{AtomicF32, AudioContext, simple_processor::MonoAudioProcessor};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::debug;
use std::sync::Arc;
use std::time::Duration;

use crate::hilbert::HilbertTransform;
use crate::ring_buffer::SignalRingBuffer;

/// 16384 slots ≈ 16 seconds of history at ~1kHz buffer rate.
const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// Fast envelope follower: catches every peak within a cycle.
const FAST_ATTACK: Duration = Duration::from_millis(1);
const FAST_RELEASE: Duration = Duration::new(0, 4_000_000); // 4ms

pub struct ProcessorSettingsInner {
    /// Current envelope value for the show loop.
    pub envelope: AtomicF32,
    pub filter_cutoff: AtomicF32,    // Hz
    pub envelope_attack: AtomicF32,  // sec (slow stage)
    pub envelope_release: AtomicF32, // sec (slow stage)
    /// Input signal gain multiplier (linear scale).
    pub gain: AtomicF32,
    /// Symmetric output smoothing time constant (seconds). 0 = disabled.
    pub output_smoothing: AtomicF32,
    /// Whether the auto-trim is enabled.
    pub auto_trim_enabled: AtomicF32,
    /// Current auto-trim gain factor (read by GUI for display).
    pub auto_trim_gain: AtomicF32,
    /// True if any raw input sample hit >= 1.0 before our gain (pre-gain clipping).
    pub pre_gain_clipping: AtomicF32,

    // === Ring buffers for visualization ===
    /// Envelope after two-stage follower (before output smoothing).
    pub envelope_history: SignalRingBuffer,
    /// Per-buffer input peak amplitude.
    pub input_peak_history: SignalRingBuffer,
    /// Envelope after symmetric output smoothing.
    pub smoothed_history: SignalRingBuffer,
    /// Effective input gain over time.
    pub effective_gain_history: SignalRingBuffer,
    /// Target gain the AGC is slewing toward.
    pub target_gain_history: SignalRingBuffer,
}

impl ProcessorSettingsInner {
    const DEFAULT_FILTER_CUTOFF: f32 = 200.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;

    pub fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
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
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            auto_trim_enabled: AtomicF32::new(1.0), // enabled by default
            auto_trim_gain: AtomicF32::new(1.0),
            pre_gain_clipping: AtomicF32::new(0.0),
            envelope_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            input_peak_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            smoothed_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            effective_gain_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            target_gain_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// Input gain trim: slow automatic gain that compensates for gradual drift
/// in the feed level. NOT compression — just keeping the pipe full.
///
/// Slews in dB (log) space so that equal perceptual changes (+6 dB vs -6 dB)
/// take equal time at the same coefficient, independent of the current gain.
struct AutoTrim {
    /// Tracked peak level, decays slowly toward zero.
    peak_tracker: f32,
    /// Current trim gain in dB.
    gain_db: f32,
    /// Current trim gain as a linear multiplier (cached from gain_db).
    gain: f32,
    /// The clamped target gain the slew is heading toward (linear).
    desired_gain: f32,
}

impl AutoTrim {
    /// Target peak level. Unity — downstream is all floating point,
    /// so brief transient overshoot just means the envelope exceeds 1.0
    /// momentarily (clamped at final output). The 20:1 asymmetry between
    /// up (20s) and down (0.5s) time constants prevents oscillation.
    const TARGET: f32 = 1.0;
    /// Gain range in dB.
    const MIN_GAIN_DB: f32 = -10.0;
    const MAX_GAIN_DB: f32 = 10.0;
    /// Peak tracker release time constant (~10s at 1kHz buffer rate).
    const PEAK_RELEASE_COEFF: f32 = 0.9999;
    /// Gain adjustment rate upward: slow (~20s time constant at 1kHz).
    const GAIN_UP_COEFF: f32 = 0.99995;
    /// Gain adjustment rate downward: faster (~0.5s time constant at 1kHz).
    const GAIN_DOWN_COEFF: f32 = 0.998;

    fn new() -> Self {
        Self {
            peak_tracker: 0.0,
            gain_db: 0.0,
            gain: 1.0,
            desired_gain: 1.0,
        }
    }

    fn db_to_linear(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    fn linear_to_db(lin: f32) -> f32 {
        20.0 * lin.log10()
    }

    /// Update the trim based on the peak level observed in this buffer.
    /// Returns the current trim gain to apply.
    fn update(&mut self, buffer_peak: f32) -> f32 {
        // Track the peak: instant attack, slow release.
        if buffer_peak > self.peak_tracker {
            self.peak_tracker = buffer_peak;
        } else {
            self.peak_tracker =
                Self::PEAK_RELEASE_COEFF * self.peak_tracker
                    + (1.0 - Self::PEAK_RELEASE_COEFF) * buffer_peak;
        }

        // Compute desired gain in dB to bring tracked peak to target.
        if self.peak_tracker > 0.001 {
            let desired_linear = Self::TARGET / self.peak_tracker;
            let desired_db =
                Self::linear_to_db(desired_linear).clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
            self.desired_gain = Self::db_to_linear(desired_db);

            // Slew in dB space: slow up, fast down.
            let coeff = if desired_db > self.gain_db {
                Self::GAIN_UP_COEFF
            } else {
                Self::GAIN_DOWN_COEFF
            };
            self.gain_db = coeff * self.gain_db + (1.0 - coeff) * desired_db;
            self.gain = Self::db_to_linear(self.gain_db);
        }

        self.gain
    }
}

pub struct Processor {
    settings: ProcessorSettings,
    filter_cutoff: f32,
    envelope_attack: f32,
    envelope_release: f32,
    channel_count: usize,
    context: AudioContext,

    filters: Vec<FilterProcessor<f32>>,
    hilberts: Vec<HilbertTransform>,
    fast_envelopes: Vec<EnvelopeFollowerProcessor>,
    slow_envelopes: Vec<EnvelopeFollowerProcessor>,

    /// Symmetric one-pole smoother state (one value, not per-channel —
    /// applied after the per-channel envelopes are averaged).
    smoothed_value: f32,
    /// Cached smoother coefficient, recomputed when output_smoothing changes.
    smooth_coeff: f32,
    smooth_time: f32,

    /// Automatic input gain trim.
    auto_trim: AutoTrim,
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
        let slow_attack = Duration::from_secs_f32(envelope_attack);
        let slow_release = Duration::from_secs_f32(envelope_release);

        let mut filters = vec![];
        let mut hilberts = vec![];
        let mut fast_envelopes = vec![];
        let mut slow_envelopes = vec![];

        for _ in 0..n {
            let mut filter = FilterProcessor::new(FilterType::LowPass);
            filter.set_cutoff(filter_cutoff);
            filter.m_prepare(&mut context);
            filters.push(filter);

            hilberts.push(HilbertTransform::new());
            fast_envelopes.push(make_envelope(&mut context, FAST_ATTACK, FAST_RELEASE));
            slow_envelopes.push(make_envelope(&mut context, slow_attack, slow_release));
        }

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: n,
            context,
            filters,
            hilberts,
            fast_envelopes,
            slow_envelopes,
            smoothed_value: 0.0,
            smooth_coeff: 0.0,
            smooth_time: 0.0,
            auto_trim: AutoTrim::new(),
        }
    }

    /// Compute the one-pole coefficient from a time constant and the
    /// actual buffer update rate (sample_rate / frames_per_buffer).
    fn compute_smooth_coeff(time_secs: f32, update_rate: f32) -> f32 {
        if time_secs <= 0.0 || update_rate <= 0.0 {
            return 0.0; // disabled
        }
        (-1.0 / (time_secs * update_rate)).exp()
    }

    fn maybe_update_parameters(&mut self) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            debug!("Updating filter cutoff to {new_filter_cutoff}");
            self.filter_cutoff = new_filter_cutoff;
            for filter in &mut self.filters {
                filter.set_cutoff(new_filter_cutoff);
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
            for env in &mut self.slow_envelopes {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
        }

    }

    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        if interleaved_buffer.is_empty() {
            return;
        }

        self.maybe_update_parameters();

        // Recompute smoothing coefficient if the time constant changed,
        // using the actual buffer size to determine the update rate.
        let new_smooth = self.settings.output_smoothing.get();
        if new_smooth != self.smooth_time {
            self.smooth_time = new_smooth;
            let frames = interleaved_buffer.len() / self.channel_count.max(1);
            if frames > 0 {
                let update_rate = self.context.settings.sample_rate / frames as f32;
                self.smooth_coeff = Self::compute_smooth_coeff(new_smooth, update_rate);
            }
        }

        let mut raw_peak: f32 = 0.0;
        let mut input_peak: f32 = 0.0;
        let auto_trim_enabled = self.settings.auto_trim_enabled.get() > 0.5;

        // Either manual gain or auto-trim, never both.
        let effective_gain = if auto_trim_enabled {
            self.auto_trim.gain
        } else {
            self.settings.gain.get()
        };

        for frame in interleaved_buffer.chunks(self.channel_count) {
            for (ch, raw_sample) in frame.iter().enumerate() {
                raw_peak = raw_peak.max(raw_sample.abs());
                let sample = *raw_sample * effective_gain;
                input_peak = input_peak.max(sample.abs());

                // lowpass → Hilbert |z(t)| → fast envelope → slow envelope
                let filtered = self.filters[ch].m_process(&mut self.context, sample);
                let amplitude = self.hilberts[ch].envelope(filtered as f64) as f32;
                self.fast_envelopes[ch].m_process(&mut self.context, amplitude);
                let fast_val = self.fast_envelopes[ch].handle().state();
                self.slow_envelopes[ch].m_process(&mut self.context, fast_val);
            }
        }

        // Pre-gain clipping indicator: did any raw sample hit >= 1.0?
        self.settings
            .pre_gain_clipping
            .set(if raw_peak >= 1.0 { 1.0 } else { 0.0 });

        // Update auto-trim based on the pre-gain peak (raw signal level).
        // We feed raw_peak, not input_peak, to avoid a feedback loop where
        // the trim adjusts based on its own output.
        if auto_trim_enabled {
            self.auto_trim.update(raw_peak);
            self.settings.auto_trim_gain.set(self.auto_trim.gain);
        }

        let ch_count = self.channel_count as f32;
        let envelope = self
            .slow_envelopes
            .iter()
            .map(|e| e.handle().state())
            .sum::<f32>()
            / ch_count;

        // Symmetric one-pole smoothing: y[n] = coeff * y[n-1] + (1 - coeff) * x[n]
        let coeff = self.smooth_coeff;
        self.smoothed_value = coeff * self.smoothed_value + (1.0 - coeff) * envelope;

        self.settings.envelope.set(envelope);
        self.settings.envelope_history.push(envelope);
        self.settings.input_peak_history.push(input_peak);
        self.settings.smoothed_history.push(self.smoothed_value);
        self.settings.effective_gain_history.push(effective_gain);
        self.settings
            .target_gain_history
            .push(self.auto_trim.desired_gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_trim_boosts_quiet_signal() {
        let mut trim = AutoTrim::new();
        assert!((trim.gain - 1.0).abs() < 1e-6);

        // Feed a consistently quiet signal (0.2 peak) for many buffers.
        // The upward adjustment is very slow (~20s at 1kHz), so we need
        // ~30000 iterations to see significant movement.
        for _ in 0..30000 {
            trim.update(0.2);
        }
        assert!(
            trim.gain > 1.3,
            "Trim should boost quiet signal, got {:.3}",
            trim.gain
        );
        assert!(
            trim.gain <= AutoTrim::db_to_linear(AutoTrim::MAX_GAIN_DB) + 0.01,
            "Trim should not exceed MAX_GAIN, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn auto_trim_reduces_loud_signal() {
        let mut trim = AutoTrim::new();

        // Feed a consistently loud signal (1.5 peak) for many buffers.
        // Desired = 0.85 / 1.5 ≈ 0.567.
        for _ in 0..5000 {
            trim.update(1.5);
        }
        assert!(
            trim.gain < 0.7,
            "Trim should reduce loud signal, got {:.3}",
            trim.gain
        );
        assert!(
            trim.gain >= AutoTrim::db_to_linear(AutoTrim::MIN_GAIN_DB) - 0.01,
            "Trim should not go below MIN_GAIN, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn auto_trim_stays_near_unity_at_target() {
        let mut trim = AutoTrim::new();

        // Feed signal right at target level.
        for _ in 0..5000 {
            trim.update(AutoTrim::TARGET);
        }
        assert!(
            (trim.gain - 1.0).abs() < 0.05,
            "Trim should stay near 1.0 for target-level signal, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn auto_trim_downward_is_faster_than_upward() {
        // Start from unity, feed loud signal, measure convergence speed.
        let mut trim_down = AutoTrim::new();
        for _ in 0..500 {
            trim_down.update(1.5);
        }
        let down_deviation = (trim_down.gain - 1.0).abs();

        // Start from unity, feed quiet signal, measure convergence speed.
        let mut trim_up = AutoTrim::new();
        for _ in 0..500 {
            trim_up.update(0.2);
        }
        let up_deviation = (trim_up.gain - 1.0).abs();

        assert!(
            down_deviation > up_deviation,
            "Downward adjustment ({:.4}) should be faster than upward ({:.4})",
            down_deviation,
            up_deviation
        );
    }

    #[test]
    fn auto_trim_ignores_silence() {
        let mut trim = AutoTrim::new();

        // Feed silence — trim should stay at 1.0 (peak tracker stays near 0,
        // which is below the 0.001 threshold).
        for _ in 0..5000 {
            trim.update(0.0);
        }
        assert!(
            (trim.gain - 1.0).abs() < 0.01,
            "Trim should stay at 1.0 during silence, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn pre_gain_clipping_detected() {
        let settings = ProcessorSettings::default();
        let mut processor = Processor::new(settings.clone(), 48000, 1);

        // Feed a signal that clips at the input.
        let buffer: Vec<f32> = vec![1.0; 48];
        processor.process(&buffer);
        assert!(
            settings.pre_gain_clipping.get() > 0.5,
            "Pre-gain clipping should be detected"
        );

        // Feed a normal signal.
        let buffer: Vec<f32> = vec![0.5; 48];
        processor.process(&buffer);
        assert!(
            settings.pre_gain_clipping.get() < 0.5,
            "Pre-gain clipping should clear on normal signal"
        );
    }

    #[test]
    fn auto_trim_disabled_stays_at_unity() {
        let settings = ProcessorSettings::default();
        settings.auto_trim_enabled.set(0.0); // disabled
        let mut processor = Processor::new(settings.clone(), 48000, 1);

        // Feed quiet signal — without trim, gain should stay at 1.0.
        let buffer: Vec<f32> = vec![0.1; 48];
        for _ in 0..100 {
            processor.process(&buffer);
        }
        assert!(
            (settings.auto_trim_gain.get() - 1.0).abs() < 0.01,
            "Auto-trim gain should stay at 1.0 when disabled, got {:.3}",
            settings.auto_trim_gain.get()
        );
    }

    #[test]
    fn processor_produces_envelope_from_sine() {
        let settings = ProcessorSettings::default();
        settings.auto_trim_enabled.set(0.0); // disable trim for deterministic test
        let mut processor = Processor::new(settings.clone(), 48000, 1);

        // Feed a 100Hz sine for 1 second.
        let sample_rate = 48000.0_f32;
        let total_samples = 48000;
        let buffer_size = 48;
        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    (2.0 * std::f32::consts::PI * 100.0 * t).sin() * 0.7
                })
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        let envelope = settings.envelope.get();
        assert!(
            envelope > 0.3,
            "Envelope should be non-trivial after 1s of 100Hz sine, got {:.3}",
            envelope
        );
    }

    /// Helper: generate a mono sine buffer and feed it through a processor
    /// for the given duration. Returns the final input_peak from the history.
    fn run_processor_with_sine(
        amplitude: f32,
        freq_hz: f32,
        duration_secs: f32,
        settings: &ProcessorSettings,
    ) -> Vec<f32> {
        let sample_rate = 48000_u32;
        let buffer_size = 48;
        let total_samples = (duration_secs * sample_rate as f32) as usize;
        let mut processor = Processor::new(settings.clone(), sample_rate, 1);

        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| {
                    let t = i as f32 / sample_rate as f32;
                    (2.0 * std::f32::consts::PI * freq_hz * t).sin() * amplitude
                })
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        // Drain the input_peak_history to get the post-gain peaks over time.
        let mut peaks = Vec::new();
        let mut pos = 0;
        settings.input_peak_history.drain_into(&mut peaks, &mut pos);
        peaks
    }

    #[test]
    fn auto_trim_converges_quiet_signal_through_processor() {
        // Feed a quiet 100Hz sine (amplitude 0.1) through the full processor
        // with auto-trim enabled. Desired gain = 1.0/0.1 = +20 dB, clamped
        // to +10 dB (3.162x). With a 20s upward time constant, 30s gets us
        // ~78% of the way in dB space (0 dB toward +10 dB ≈ +7.8 dB ≈ 2.45x).
        let settings = ProcessorSettings::default();

        let peaks = run_processor_with_sine(0.1, 100.0, 30.0, &settings);

        let last_peaks: Vec<f32> = peaks.iter().rev().take(100).copied().collect();
        let avg_last = last_peaks.iter().sum::<f32>() / last_peaks.len() as f32;

        // Post-gain peak should be meaningfully above the raw 0.1.
        assert!(
            avg_last > 0.15,
            "Auto-trim should boost quiet signal peaks above raw level, got avg {:.3}",
            avg_last
        );

        let trim_gain = settings.auto_trim_gain.get();
        assert!(
            trim_gain > 1.5,
            "Auto-trim gain should be well above unity for quiet signal, got {:.3}",
            trim_gain
        );
    }

    #[test]
    fn auto_trim_converges_loud_signal_through_processor() {
        // Feed a loud 100Hz sine (amplitude 1.5, over unity) through the
        // full processor. The trim should reduce gain so post-gain peaks
        // approach the target (1.0).
        let settings = ProcessorSettings::default();

        let peaks = run_processor_with_sine(1.5, 100.0, 10.0, &settings);

        // desired gain = 1.0 / 1.5 ≈ 0.667
        let last_peaks: Vec<f32> = peaks.iter().rev().take(100).copied().collect();
        let avg_last = last_peaks.iter().sum::<f32>() / last_peaks.len() as f32;

        assert!(
            avg_last < 1.3,
            "Auto-trim should reduce loud signal peaks toward target, got avg {:.3}",
            avg_last
        );

        let trim_gain = settings.auto_trim_gain.get();
        assert!(
            trim_gain < 0.8,
            "Auto-trim gain should be well below unity for 1.5x signal, got {:.3}",
            trim_gain
        );
    }

    #[test]
    fn auto_trim_no_feedback_loop() {
        // This is the specific regression test for the feedback loop bug.
        // If the trim feeds back on its own output (post-gain peaks), it
        // will oscillate or converge to the wrong value. If it correctly
        // reads pre-gain peaks, the gain should converge to TARGET/amplitude.
        let settings = ProcessorSettings::default();
        let amplitude = 0.4_f32;

        let _peaks = run_processor_with_sine(amplitude, 100.0, 30.0, &settings);

        let trim_gain = settings.auto_trim_gain.get();
        // Expected: gain ≈ TARGET / amplitude = 0.85 / 0.4 = 2.125
        let expected = AutoTrim::TARGET / amplitude;

        // With a 20s upward time constant and 30s of signal, we reach ~78%
        // of the way from 1.0 to the target. Allow enough tolerance for that,
        // but catch the feedback loop bug (which converges near 1.0).
        let min_expected = 1.0 + (expected - 1.0) * 0.5; // at least halfway there
        assert!(
            trim_gain > min_expected,
            "Auto-trim gain {:.3} should be converging toward TARGET/amplitude = {:.3} \
             (expected at least {:.3} after 30s). \
             If gain is near 1.0, the trim is feeding back on its own output.",
            trim_gain,
            expected,
            min_expected,
        );

        // Also verify post-gain peaks are actually near target.
        let mut peaks = Vec::new();
        let mut pos = 0;
        settings.input_peak_history.drain_into(&mut peaks, &mut pos);
        // We already drained in run_processor_with_sine, so this may be empty.
        // Use the effective_gain_history instead to verify gain converged.
        let mut gains = Vec::new();
        pos = 0;
        settings
            .effective_gain_history
            .drain_into(&mut gains, &mut pos);
        let last_gains: Vec<f32> = gains.iter().rev().take(100).copied().collect();
        let avg_gain = last_gains.iter().sum::<f32>() / last_gains.len().max(1) as f32;

        assert!(
            avg_gain > min_expected,
            "Effective gain history avg {:.3} should be converging toward {:.3}",
            avg_gain,
            expected
        );
    }
}
