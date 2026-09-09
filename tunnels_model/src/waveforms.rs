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
    fn outside_duty_cycle(&self) -> bool {
        self.phase > self.duty_cycle || self.duty_cycle == 0.0
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
        // Rescale the bipolar wave into the unipolar range, reading it three
        // quarters of a cycle ahead so the pulse starts at the bottom of that
        // range and peaks halfway through it, where a sine or triangle pulse
        // peaks.
        return (square_spatial(&WaveformArgsSpatial {
            phase: phase + 0.75,
            smoothing: args.smoothing,
            duty_cycle: UnipolarFloat::ONE,
            pulse: false,
        }) + 1.0)
            / 2.0;
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

    /// The smoothings a waveform is sampled at: both ends of the range and
    /// two points inside it.
    const SMOOTHINGS: [f64; 4] = [0.0, 0.25, 0.5, 1.0];

    /// The duty cycles a waveform is sampled at: the whole period and two
    /// windows the cycle has to compress into.
    const DUTY_CYCLES: [f64; 3] = [1.0, 0.5, 0.25];

    /// Pulse mode rescales a waveform into the unipolar range rather than
    /// clipping the half of it that falls below zero. A waveform symmetric
    /// about the middle of its period stays symmetric once rescaled, where a
    /// clipped one does not: clipping widens the trough by whatever the peak
    /// loses.
    #[test]
    fn a_pulse_keeps_the_symmetry_of_the_waveform_it_rescales() {
        for (name, waveform) in [
            ("sine", sine as fn(&WaveformArgs) -> f64),
            ("triangle", triangle),
            ("square", square),
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

    /// A square holds its two levels for equal parts of a cycle, and pulse
    /// mode rescales those levels rather than reshaping them, so a pulsed
    /// square holds the top of its range for exactly as much of its duty cycle
    /// window as it holds the bottom.
    ///
    /// Smoothing eats into both dwells from either side, never into one. It
    /// reaches a quarter of a cycle at the top of its range and spends that on
    /// each of the four edges either dwell has, so what is left of a dwell is
    /// half a cycle less twice the smoothing, down to nothing.
    #[test]
    fn a_pulsed_square_dwells_as_long_high_as_low() {
        for smoothing in SMOOTHINGS {
            for duty_cycle in DUTY_CYCLES {
                let samples = pulse_period(square, smoothing, duty_cycle);
                let dwell = |level: f64| samples.iter().filter(|v| **v == level).count();
                let (high, low) = (dwell(1.0), dwell(0.0));
                let expected = ((0.5 - 0.5 * smoothing) * SAMPLES as f64).round() as usize;
                assert_eq!(
                    (high, low),
                    (expected, expected),
                    "a square pulse at smoothing {smoothing} and duty cycle \
                     {duty_cycle} holds 1 for {high} of {SAMPLES} samples and 0 \
                     for {low}, where smoothing leaves room for {expected} of each"
                );
            }
        }
    }

    /// A pulse occupies the unipolar range and begins at the bottom of it, so
    /// that the waveform it is a rescaling of can be read as an amount of
    /// something rather than as a displacement either side of nothing.
    #[test]
    fn a_pulse_spans_the_unipolar_range_from_zero() {
        for (name, waveform) in [
            ("sine", sine as fn(&WaveformArgs) -> f64),
            ("triangle", triangle),
            ("square", square),
            ("sawtooth", sawtooth),
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
                        samples[0] <= floor + 1e-9,
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
