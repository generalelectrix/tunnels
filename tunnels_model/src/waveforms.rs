use std::f64::consts::PI;

use tunnels_lib::number::{Phase, UnipolarFloat};

const TWO_PI: f64 = 2.0 * PI;
const HALF_PI: f64 = PI / 2.0;

/// Common args passed to all waveform generating functions.
/// Spaital and temporal phases are equivalent for travelling waves and will be
/// summed.
/// For standing waves, the temporal phase is used to compute the overall
/// envelope modulation, while the spatial phase is used to determine the offset
/// into the waveform.
pub struct WaveformArgs {
    pub phase_spatial: Phase,
    pub phase_temporal: Phase,
    pub smoothing: UnipolarFloat,
    pub duty_cycle: UnipolarFloat,
    pub pulse: bool,
    pub standing: bool,
}

impl WaveformArgs {
    /// Return a temporal scaling factor and processed waveform args.
    /// This implements standing vs travelling wave behavior for any periodic waveform.
    fn spatial_params(&self) -> (f64, WaveformArgsSpatial) {
        if self.standing {
            let mut amplitude = (TWO_PI * self.phase_temporal.val()).cos();
            // In pulse mode, standing waves should still only take positive values.
            if self.pulse {
                amplitude = (amplitude + 1.0) / 2.0;
            }
            let spatial_args = WaveformArgsSpatial {
                phase: self.phase_spatial,
                smoothing: self.smoothing,
                duty_cycle: self.duty_cycle,
                pulse: self.pulse,
            };
            (amplitude, spatial_args)
        } else {
            (
                1.0,
                WaveformArgsSpatial {
                    phase: self.phase_spatial + self.phase_temporal,
                    smoothing: self.smoothing,
                    duty_cycle: self.duty_cycle,
                    pulse: self.pulse,
                },
            )
        }
    }
}

/// Common args passed to all spatial waveform generation functions.
struct WaveformArgsSpatial {
    pub phase: Phase,
    pub smoothing: UnipolarFloat,
    pub duty_cycle: UnipolarFloat,
    pub pulse: bool,
}

impl WaveformArgsSpatial {
    /// Return true if the value should be 0 due to set duty cycle.
    ///
    /// The window is open at its far end. A phase equal to the duty cycle
    /// scales to exactly one cycle, which wraps to the window's near end, so
    /// leaving that phase inside would read the close of the window as its
    /// opening.
    fn outside_duty_cycle(&self) -> bool {
        self.phase >= self.duty_cycle || self.duty_cycle == 0.0
    }

    /// Return the phase scaled to the duty cycle.
    fn duty_cycle_scaled_phase(&self) -> Phase {
        self.phase / self.duty_cycle
    }
}

pub fn sine(args: &WaveformArgs) -> f64 {
    let (amplitude, args) = args.spatial_params();
    amplitude * sine_spatial(&args)
}

fn sine_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }
    let phase = args.duty_cycle_scaled_phase();
    if args.pulse {
        return ((TWO_PI * phase.val() - HALF_PI).sin() + 1.0) / 2.0;
    }
    (TWO_PI * phase.val()).sin()
}

pub fn triangle(args: &WaveformArgs) -> f64 {
    let (amplitude, args) = args.spatial_params();
    amplitude * triangle_spatial(&args)
}

fn triangle_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }
    let phase = args.duty_cycle_scaled_phase();
    if args.pulse {
        return if phase < 0.5 {
            2.0 * phase.val()
        } else {
            2.0 * (1.0 - phase.val())
        };
    }

    if phase < 0.25 {
        4.0 * phase.val()
    } else if phase > 0.75 {
        4.0 * (phase.val() - 1.0)
    } else {
        2.0 - 4.0 * phase.val()
    }
}

pub fn square(args: &WaveformArgs) -> f64 {
    let (amplitude, args) = args.spatial_params();
    amplitude * square_spatial(&args)
}

fn square_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }

    let phase = args.duty_cycle_scaled_phase();
    if args.pulse {
        // Full for the first half of the window and at rest for the second,
        // built rather than rescaled from the bipolar wave. Rescaling would
        // carry the wave's own trough into the second half and lift the rest
        // between pulses off zero, which is a triangle *wave* at full
        // smoothing where a pulse wants a triangle and then nothing.
        //
        // Smoothing opens each edge of the pulse into a ramp, taking from the
        // dwell it bounds and never from the rest. At full smoothing the two
        // ramps meet in the middle and the pulse is a triangle in the first
        // half of its window — a triangle pulse at half the duty cycle.
        if phase >= 0.5 {
            return 0.0;
        }
        let ramp = 0.25 * args.smoothing.val();
        if ramp == 0.0 {
            return 1.0;
        }
        if phase.val() < ramp {
            return phase.val() / ramp;
        }
        if phase.val() > 0.5 - ramp {
            return (0.5 - phase.val()) / ramp;
        }
        return 1.0;
    }
    // internal smoothing scale is 0 to 0.25.
    let smoothing = args.smoothing * UnipolarFloat::new(0.25);

    if smoothing == 0.0 {
        return if phase < 0.5 { 1.0 } else { -1.0 };
    }

    if phase < smoothing {
        phase.val() / smoothing.val()
    } else if phase > (0.5 - smoothing.val()) && phase < (0.5 + smoothing.val()) {
        -(phase.val() - 0.5) / smoothing.val()
    } else if phase > (1.0 - smoothing.val()) {
        (phase.val() - 1.0) / smoothing.val()
    } else if phase >= smoothing && phase <= 0.5 - smoothing.val() {
        1.0
    } else {
        -1.0
    }
}

pub fn sawtooth(args: &WaveformArgs) -> f64 {
    let (amplitude, args) = args.spatial_params();
    amplitude * sawtooth_spatial(&args)
}

fn sawtooth_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }
    let phase = args.duty_cycle_scaled_phase();

    if args.pulse {
        return sawtooth_spatial(&WaveformArgsSpatial {
            phase: phase * UnipolarFloat::new(0.5),
            smoothing: args.smoothing,
            duty_cycle: UnipolarFloat::new(1.0),
            pulse: false,
        });
    }
    // internal smoothing scale is 0 to 0.25.
    let smoothing = args.smoothing * UnipolarFloat::new(0.25);
    if smoothing == 0.0 {
        return if phase < 0.5 {
            2.0 * phase.val()
        } else {
            2.0 * (phase.val() - 1.0)
        };
    }

    if phase < 0.5 - smoothing.val() {
        phase.val() / (0.5 - smoothing.val())
    } else if phase > 0.5 + smoothing.val() {
        (phase.val() - 1.0) / (0.5 - smoothing.val())
    } else {
        -(phase.val() - 0.5) / smoothing.val()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// How many points one period is sampled at.
    const SAMPLES: usize = 1024;

    /// One cycle of `waveform` in pulse mode at the given smoothing, sampled
    /// across the duty cycle window the cycle is compressed into.
    ///
    /// Each sample sits at the centre of the slice of the window it stands
    /// for, so a waveform with a step in it is never read exactly on the step,
    /// where which side the reading belongs to comes down to which comparison
    /// happens to be the strict one.
    fn pulse_period(
        waveform: fn(&WaveformArgs) -> f64,
        smoothing: f64,
        duty_cycle: f64,
    ) -> Vec<f64> {
        (0..SAMPLES)
            .map(|i| {
                waveform(&WaveformArgs {
                    phase_spatial: Phase::new(duty_cycle * (i as f64 + 0.5) / SAMPLES as f64),
                    phase_temporal: Phase::ZERO,
                    smoothing: UnipolarFloat::new(smoothing),
                    duty_cycle: UnipolarFloat::new(duty_cycle),
                    pulse: true,
                    standing: false,
                })
            })
            .collect()
    }

    /// The smoothings a waveform is sampled at: both ends of the range, and
    /// enough points inside it that a dwell's expected width has to be a model
    /// of where the samples sit rather than a rounding that agrees at a few.
    const SMOOTHINGS: [f64; 9] = [0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.9, 0.999, 1.0];

    /// The duty cycles a waveform is sampled at: the whole period and two
    /// windows the cycle has to compress into.
    const DUTY_CYCLES: [f64; 3] = [1.0, 0.5, 0.25];

    /// A sine and a triangle reach the unipolar range by being rescaled into
    /// it rather than clipped at zero, and both are symmetric about the middle
    /// of their period, so both stay symmetric once pulsed. A clipped waveform
    /// would not: clipping widens the trough by whatever the peak loses.
    ///
    /// A square pulse is built rather than rescaled and is asymmetric by
    /// design, so it is covered by its own tests instead.
    #[test]
    fn a_pulse_keeps_the_symmetry_of_its_waveform() {
        for (name, waveform) in [
            ("sine", sine as fn(&WaveformArgs) -> f64),
            ("triangle", triangle),
        ] {
            for smoothing in SMOOTHINGS {
                for duty_cycle in DUTY_CYCLES {
                    let samples = pulse_period(waveform, smoothing, duty_cycle);
                    for i in 0..SAMPLES / 2 {
                        let (rising, falling) = (samples[i], samples[SAMPLES - 1 - i]);
                        assert!(
                            (rising - falling).abs() < 1e-9,
                            "a {name} pulse at smoothing {smoothing} and duty cycle \
                             {duty_cycle} is lopsided: sample {i} reads {rising} \
                             while its mirror reads {falling}"
                        );
                    }
                }
            }
        }
    }

    /// A square pulse rests for the second half of its duty cycle window
    /// whatever the smoothing, and holds the top of its range for the first
    /// half less what the smoothing spends on the two edges bounding it.
    ///
    /// Smoothing takes from the dwell it opens and never from the rest. It
    /// reaches a quarter of a window at the top of its range and spends that on
    /// each of the pulse's two edges, so a fully smoothed pulse has no flat top
    /// left and the rest between pulses is untouched at half the window.
    #[test]
    fn a_pulsed_square_rests_for_half_its_window() {
        for smoothing in SMOOTHINGS {
            for duty_cycle in DUTY_CYCLES {
                let samples = pulse_period(square, smoothing, duty_cycle);
                let dwell = |level: f64| samples.iter().filter(|v| **v == level).count();
                let (high, low) = (dwell(1.0), dwell(0.0));
                // Counted from where the samples sit rather than from the
                // width of the dwell, because a sample never lands on a ramp's
                // boundary and rounding the width agrees with that only for
                // some smoothings.
                let scaled = |i: usize| (i as f64 + 0.5) / SAMPLES as f64;
                let ramp = 0.25 * smoothing;
                let expected_high = (0..SAMPLES)
                    .filter(|i| (ramp..=0.5 - ramp).contains(&scaled(*i)))
                    .count();
                let expected_low = (0..SAMPLES).filter(|i| scaled(*i) >= 0.5).count();
                assert_eq!(
                    (high, low),
                    (expected_high, expected_low),
                    "a square pulse at smoothing {smoothing} and duty cycle \
                     {duty_cycle} holds 1 for {high} of {SAMPLES} samples and 0 \
                     for {low}, where the edges leave room for {expected_high} at \
                     the top and the rest is {expected_low}"
                );
            }
        }
    }

    /// The duty cycle window is open at its far end, so the phase that lands
    /// exactly on it is outside the window and reads as rest.
    ///
    /// Scaling a phase into the window divides by the duty cycle, and a phase
    /// equal to it divides to exactly one — which wraps to zero, the window's
    /// near end. Were that phase still inside the window it would be read as
    /// the window's opening rather than its close, and a waveform that is full
    /// at the opening would paint full brightness at the instant it should be
    /// dark.
    #[test]
    fn the_phase_at_the_duty_cycle_is_outside_the_window() {
        for (name, waveform) in [
            ("sine", sine as fn(&WaveformArgs) -> f64),
            ("triangle", triangle),
            ("square", square),
            ("sawtooth", sawtooth),
        ] {
            for duty_cycle in DUTY_CYCLES.into_iter().filter(|d| *d < 1.0) {
                for smoothing in SMOOTHINGS {
                    for pulse in [false, true] {
                        let value = waveform(&WaveformArgs {
                            phase_spatial: Phase::new(duty_cycle),
                            phase_temporal: Phase::ZERO,
                            smoothing: UnipolarFloat::new(smoothing),
                            duty_cycle: UnipolarFloat::new(duty_cycle),
                            pulse,
                            standing: false,
                        });
                        assert_eq!(
                            value, 0.0,
                            "a {name} at smoothing {smoothing}, duty cycle \
                             {duty_cycle} and pulse {pulse} reads {value} at the \
                             phase that closes its window, where it rests"
                        );
                    }
                }
            }
        }
    }

    /// A square pulse rises at the start of its duty cycle window and falls
    /// halfway through it, so narrowing the window moves only the falling edge.
    ///
    /// A pulse centred in its window would move both of its edges as the window
    /// narrowed, which is a duty cycle changing where the pulse sits as well as
    /// how long it lasts. Holding the rise at the top of the window leaves the
    /// knob doing one thing: shortening the pulse.
    #[test]
    fn a_pulsed_square_rises_at_the_start_of_its_window() {
        for duty_cycle in DUTY_CYCLES {
            let samples = pulse_period(square, 0.0, duty_cycle);
            let (high, low) = samples.split_at(SAMPLES / 2);
            let not_high = high.iter().find(|v| **v != 1.0);
            assert!(
                not_high.is_none(),
                "a square pulse at duty cycle {duty_cycle} reads {} in the first \
                 half of its window, where it should hold 1",
                not_high.copied().unwrap_or_default()
            );
            let not_low = low.iter().find(|v| **v != 0.0);
            assert!(
                not_low.is_none(),
                "a square pulse at duty cycle {duty_cycle} reads {} in the second \
                 half of its window, where it should hold 0",
                not_low.copied().unwrap_or_default()
            );
        }
    }

    /// Smoothing a square pulse opens its edges into ramps without lifting the
    /// rest between pulses, so at full smoothing the pulse is a triangle
    /// occupying the first half of its window — which is a triangle pulse at
    /// half the duty cycle, sample for sample.
    ///
    /// The equivalence is what pins down what smoothing a pulse means. A
    /// rescaled wave would arrive at a triangle spanning the whole window and
    /// resting nowhere, which is a triangle *wave*, not a pulse.
    #[test]
    fn a_fully_smoothed_square_pulse_is_a_triangle_pulse_at_half_the_duty_cycle() {
        // Both are read at the same absolute phase, the windows they occupy
        // being what differs between them.
        let at = |waveform: fn(&WaveformArgs) -> f64, smoothing: f64, duty_cycle: f64, phase| {
            waveform(&WaveformArgs {
                phase_spatial: Phase::new(phase),
                phase_temporal: Phase::ZERO,
                smoothing: UnipolarFloat::new(smoothing),
                duty_cycle: UnipolarFloat::new(duty_cycle),
                pulse: true,
                standing: false,
            })
        };
        for duty_cycle in DUTY_CYCLES {
            for i in 0..SAMPLES {
                let phase = (i as f64 + 0.5) / SAMPLES as f64;
                let sq = at(square, 1.0, duty_cycle, phase);
                let tri = at(triangle, 0.0, duty_cycle / 2.0, phase);
                assert!(
                    (sq - tri).abs() < 1e-9,
                    "at duty cycle {duty_cycle} and phase {phase}, a fully \
                     smoothed square pulse reads {sq} where a triangle pulse at \
                     half the duty cycle reads {tri}"
                );
            }
        }
    }

    /// A pulse occupies the unipolar range, and every pulse that rises from
    /// rest begins at the bottom of it, so that the pulse reads as an amount of
    /// something rather than as a displacement either side of nothing.
    ///
    /// A square pulse begins at the top of the range instead, being a leading
    /// edge. The range itself is still asserted for it; where it rests is
    /// pinned by the test for its dwells.
    #[test]
    fn a_pulse_spans_the_unipolar_range_from_zero() {
        for (name, waveform, starts_at_rest) in [
            ("sine", sine as fn(&WaveformArgs) -> f64, true),
            ("triangle", triangle, true),
            ("square", square, false),
            ("sawtooth", sawtooth, true),
        ] {
            for smoothing in SMOOTHINGS {
                for duty_cycle in DUTY_CYCLES {
                    let samples = pulse_period(waveform, smoothing, duty_cycle);
                    let out_of_range = samples.iter().find(|v| !(0.0..=1.0).contains(*v));
                    assert!(
                        out_of_range.is_none(),
                        "a {name} pulse at smoothing {smoothing} and duty cycle \
                         {duty_cycle} left the unipolar range at {}",
                        out_of_range.unwrap()
                    );
                    let floor = samples.iter().copied().fold(f64::INFINITY, f64::min);
                    assert!(
                        !starts_at_rest || samples[0] <= floor + 1e-9,
                        "a {name} pulse at smoothing {smoothing} and duty cycle \
                         {duty_cycle} starts at {}, above the {floor} it reaches \
                         later in the cycle",
                        samples[0]
                    );
                }
            }
        }
    }

    /// A duty cycle compresses a whole pulse into the front of the period and
    /// leaves the rest of it silent, rather than cutting the pulse off wherever
    /// the window happens to end.
    #[test]
    fn a_pulse_compressed_by_a_duty_cycle_leaves_the_rest_of_the_period_silent() {
        for (name, waveform) in [
            ("sine", sine as fn(&WaveformArgs) -> f64),
            ("triangle", triangle),
            ("square", square),
            ("sawtooth", sawtooth),
        ] {
            for smoothing in SMOOTHINGS {
                for duty_cycle in DUTY_CYCLES.into_iter().filter(|d| *d < 1.0) {
                    // The pulse reaches the top of its range inside the window,
                    // so the window holds the whole cycle rather than the part
                    // of it that fitted.
                    let inside = pulse_period(waveform, smoothing, duty_cycle);
                    let ceiling = inside.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    assert!(
                        ceiling > 0.99,
                        "a {name} pulse at smoothing {smoothing} only reached \
                         {ceiling} inside a duty cycle of {duty_cycle}"
                    );
                    for i in 0..SAMPLES {
                        // Phases from the end of the window to the end of the
                        // period, which no part of the pulse belongs to.
                        let phase =
                            duty_cycle + (1.0 - duty_cycle) * (i as f64 + 0.5) / SAMPLES as f64;
                        let v = waveform(&WaveformArgs {
                            phase_spatial: Phase::new(phase),
                            phase_temporal: Phase::ZERO,
                            smoothing: UnipolarFloat::new(smoothing),
                            duty_cycle: UnipolarFloat::new(duty_cycle),
                            pulse: true,
                            standing: false,
                        });
                        assert_eq!(
                            v, 0.0,
                            "a {name} pulse at smoothing {smoothing} still read \
                             {v} at phase {phase}, past a duty cycle of {duty_cycle}"
                        );
                    }
                }
            }
        }
    }
}
