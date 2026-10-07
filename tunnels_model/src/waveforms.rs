use std::f64::consts::PI;

use tunnels_lib::number::{Phase, UnipolarFloat};

const TWO_PI: f64 = 2.0 * PI;

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

/// A square whose edges are cosine, from a hard edge at the bottom of the
/// smoothing range to a sine at the top.
pub fn sine_square(args: &WaveformArgs) -> f64 {
    let (amplitude, args) = args.spatial_params();
    amplitude * sine_square_spatial(&args)
}

fn sine_square_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }

    let phase = args.duty_cycle_scaled_phase();
    if args.pulse {
        // The pulse fills its window, rising into it and falling out of it
        // over an edge at either end. The window is the pulse's length, so a
        // rest inside it would be a second way of shortening the pulse and
        // the duty cycle would shorten it at a rate of its own.
        //
        // Built rather than rescaled from the bipolar wave. Rescaling would
        // carry the wave's own trough into the window and lift the rest
        // between pulses off zero.
        let edge = 0.5 * args.smoothing.val();
        let phase = phase.val();
        if edge == 0.0 {
            // A hard edge has nothing to pulse against across a window that
            // is the whole period. A shorter pulse comes from the duty cycle.
            return 1.0;
        }
        // The ends of the range are the waveforms they are equal to rather
        // than the arithmetic that approaches them, so that the equality is a
        // guarantee and not a coincidence of rounding.
        if args.smoothing == 1.0 {
            return (1.0 - (TWO_PI * phase).cos()) / 2.0;
        }
        // Each edge is half a period of cosine, taken from the flat top it
        // bounds, so it leaves and reaches that top with no slope at either
        // end and the two edges meet in the middle at the top of the range.
        let rising = |distance: f64| (1.0 - (PI * distance / edge).cos()) / 2.0;
        if phase < edge {
            return rising(phase);
        }
        if phase > 1.0 - edge {
            return rising(1.0 - phase);
        }
        return 1.0;
    }
    // How far an edge reaches either side of where it sits: a quarter of the
    // period at the top of the smoothing range, where the two edges take the
    // whole period between them and the dwells they bound vanish.
    let edge = 0.25 * args.smoothing.val();
    let phase = phase.val();

    if edge == 0.0 {
        return if phase < 0.5 { 1.0 } else { -1.0 };
    }
    // The ends of the range are the waveforms they are equal to rather than
    // the arithmetic that approaches them, so that the equality is a guarantee
    // and not a coincidence of rounding.
    if args.smoothing == 1.0 {
        return (TWO_PI * phase).sin();
    }

    // Each edge is half a period of cosine, which spends its whole width
    // crossing the range and leaves and arrives with no slope at either end.
    let crossing = |distance: f64| (PI * distance / (2.0 * edge)).cos();
    if phase < edge {
        // The rising edge sits on phase zero, so this is the half of it that
        // climbs away from the start of the period. Written as the sine it is
        // a quarter turn along from, which reads exactly zero where the cycle
        // starts where the cosine reads a rounding away from it.
        (PI * phase / (2.0 * edge)).sin()
    } else if (phase - 0.5).abs() <= edge {
        crossing(phase - (0.5 - edge))
    } else if phase > 1.0 - edge {
        // The half of the rising edge that approaches the start of the period,
        // reached at the end of the one before it.
        -crossing(phase - (1.0 - edge))
    } else if phase < 0.5 {
        1.0
    } else {
        -1.0
    }
}

/// A ramp whose turn opens with smoothing, from a sawtooth at the bottom of
/// the range to a triangle at the top.
pub fn tri_saw(args: &WaveformArgs) -> f64 {
    let (amplitude, args) = args.spatial_params();
    amplitude * tri_saw_spatial(&args)
}

fn tri_saw_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }
    let phase = args.duty_cycle_scaled_phase();

    if args.pulse {
        return tri_saw_spatial(&WaveformArgsSpatial {
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

    /// Where a sample sits in the window, as a fraction of it.
    ///
    /// Each sample sits at the centre of the slice of the window it stands
    /// for, so a waveform with a step in it is never read exactly on the step,
    /// where which side the reading belongs to comes down to which comparison
    /// happens to be the strict one.
    fn window_phase(i: usize) -> f64 {
        (i as f64 + 0.5) / SAMPLES as f64
    }

    /// How a waveform is read: the two knobs that shape it and whether it is
    /// asked for a pulse or for a bipolar wave.
    #[derive(Copy, Clone)]
    struct Sampling {
        smoothing: f64,
        duty_cycle: f64,
        pulse: bool,
    }

    /// One cycle of `waveform`, sampled across the duty cycle window the cycle
    /// is compressed into.
    fn period(waveform: fn(&WaveformArgs) -> f64, sampling: Sampling) -> Vec<f64> {
        (0..SAMPLES)
            .map(|i| {
                waveform(&WaveformArgs {
                    phase_spatial: Phase::new(sampling.duty_cycle * window_phase(i)),
                    phase_temporal: Phase::ZERO,
                    smoothing: UnipolarFloat::new(sampling.smoothing),
                    duty_cycle: UnipolarFloat::new(sampling.duty_cycle),
                    pulse: sampling.pulse,
                    standing: false,
                })
            })
            .collect()
    }

    /// One cycle of `waveform` in pulse mode at the given smoothing and duty
    /// cycle.
    fn pulse_period(
        waveform: fn(&WaveformArgs) -> f64,
        smoothing: f64,
        duty_cycle: f64,
    ) -> Vec<f64> {
        period(
            waveform,
            Sampling {
                smoothing,
                duty_cycle,
                pulse: true,
            },
        )
    }

    /// The smoothings a waveform is sampled at: both ends of the range, and
    /// enough points inside it that a dwell's expected width has to be a model
    /// of where the samples sit rather than a rounding that agrees at a few.
    const SMOOTHINGS: [f64; 9] = [0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.9, 0.999, 1.0];

    /// The duty cycles a waveform is sampled at: the whole period and two
    /// windows the cycle has to compress into.
    const DUTY_CYCLES: [f64; 3] = [1.0, 0.5, 0.25];

    /// A waveform symmetric about the middle of its period reaches the
    /// unipolar range by being rescaled into it or built inside it rather than
    /// clipped at zero, so it stays symmetric once pulsed. A clipped waveform
    /// would not: clipping widens the trough by whatever the peak loses.
    ///
    /// A sine square is symmetric across the whole of its smoothing range. A
    /// tri-saw is a ramp, and so asymmetric, everywhere but the top of its
    /// range, where it is a triangle.
    #[test]
    fn a_pulse_keeps_the_symmetry_of_its_waveform() {
        for (name, waveform, smoothings) in [
            (
                "sine square",
                sine_square as fn(&WaveformArgs) -> f64,
                &SMOOTHINGS[..],
            ),
            ("tri saw", tri_saw, &[1.0][..]),
        ] {
            for smoothing in smoothings.iter().copied() {
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
            ("sine square", sine_square as fn(&WaveformArgs) -> f64),
            ("tri saw", tri_saw),
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

    /// A pulse occupies the unipolar range and begins at the bottom of it, so
    /// that the pulse reads as an amount of something rather than as a
    /// displacement either side of nothing.
    #[test]
    fn a_pulse_spans_the_unipolar_range_from_zero() {
        for (name, waveform) in [
            ("sine square", sine_square as fn(&WaveformArgs) -> f64),
            ("tri saw", tri_saw),
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
            ("sine square", sine_square as fn(&WaveformArgs) -> f64),
            ("tri saw", tri_saw),
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

    /// `sin(2πp)`, the wave a sine square reaches at the top of its smoothing
    /// range.
    ///
    /// Written out as a closed form, so that a sine square compared against it
    /// is not being compared to another branch of itself.
    fn sine_wave(phase: f64) -> f64 {
        (TWO_PI * phase).sin()
    }

    /// `(1 - cos(2πp)) / 2`, the pulse a sine square reaches at the top of its
    /// smoothing range.
    ///
    /// Written out as a closed form, so that a sine square compared against it
    /// is not being compared to another branch of itself.
    fn sine_pulse(phase: f64) -> f64 {
        (1.0 - (TWO_PI * phase).cos()) / 2.0
    }

    /// A sine square pulse fills its duty cycle window, holding the top of its
    /// range across the whole of it but for the two edges smoothing opens at
    /// either end, which take half the window between them at the top of the
    /// range.
    ///
    /// A pulse that rested inside its own window would answer the duty cycle
    /// twice: once in how much of the period it occupies, and again in how
    /// much of that it spends at rest. The window is the pulse's length, and
    /// nothing inside it is a second way of saying so.
    #[test]
    fn a_pulsed_sine_square_fills_its_window() {
        for smoothing in SMOOTHINGS {
            let edge = 0.5 * smoothing;
            for duty_cycle in DUTY_CYCLES {
                let samples = pulse_period(sine_square, smoothing, duty_cycle);
                // Counted from where the samples sit rather than from the
                // width of the top, because a sample never lands on an edge's
                // boundary.
                let held = samples.iter().filter(|v| **v == 1.0).count();
                let expected = (0..SAMPLES)
                    .filter(|i| (edge..=1.0 - edge).contains(&window_phase(*i)))
                    .count();
                assert_eq!(
                    held, expected,
                    "a sine square pulse at smoothing {smoothing} and duty cycle \
                     {duty_cycle} holds the top of its range for {held} of \
                     {SAMPLES} samples, where edges {edge} wide leave room for \
                     {expected}"
                );
            }
        }
    }

    /// A sine square pulse's smoothing runs between two exact waveforms: a
    /// window held full for the whole of its length at the bottom of the
    /// range, and a sine pulse at the top.
    ///
    /// The flat window is the accepted cost of a hard edge having nothing to
    /// pulse against at a duty cycle of one. A shorter pulse comes from
    /// turning the duty cycle down, which is the knob that means length.
    #[test]
    fn a_sine_square_pulse_runs_from_a_flat_window_to_a_sine_pulse() {
        for duty_cycle in DUTY_CYCLES {
            let hard = pulse_period(sine_square, 0.0, duty_cycle);
            let smooth = pulse_period(sine_square, 1.0, duty_cycle);
            for i in 0..SAMPLES {
                let phase = window_phase(i);
                assert_eq!(
                    hard[i], 1.0,
                    "an unsmoothed sine square pulse reads {} at phase {phase} of a \
                     duty cycle of {duty_cycle}, where it fills its window",
                    hard[i]
                );
                let sine = sine_pulse(phase);
                assert_eq!(
                    smooth[i], sine,
                    "a fully smoothed sine square pulse reads {} at phase {phase} of \
                     a duty cycle of {duty_cycle}, where a sine pulse reads {sine}",
                    smooth[i]
                );
            }
        }
    }

    /// A sine square's smoothing runs between two exact waveforms: a hard
    /// square at the bottom of the range and a sine at the top.
    ///
    /// The sine is what makes the knob a control over the whole of its range.
    /// A square whose edges are linear ramps arrives at a triangle instead,
    /// which is a shape another waveform already reaches, so the top of the
    /// range stops being a place worth putting the knob.
    #[test]
    fn a_sine_square_runs_from_a_hard_edge_to_a_sine() {
        for duty_cycle in DUTY_CYCLES {
            let sampling = |smoothing| Sampling {
                smoothing,
                duty_cycle,
                pulse: false,
            };
            let hard = period(sine_square, sampling(0.0));
            let smooth = period(sine_square, sampling(1.0));
            for i in 0..SAMPLES {
                let phase = window_phase(i);
                let edged = if phase < 0.5 { 1.0 } else { -1.0 };
                assert_eq!(
                    hard[i], edged,
                    "an unsmoothed sine square reads {} at phase {phase} of a duty \
                     cycle of {duty_cycle}, where a hard square reads {edged}",
                    hard[i]
                );
                let sine = sine_wave(phase);
                assert_eq!(
                    smooth[i], sine,
                    "a fully smoothed sine square reads {} at phase {phase} of a duty \
                     cycle of {duty_cycle}, where a sine reads {sine}",
                    smooth[i]
                );
            }
        }
    }

    /// A sine square reads exactly zero where its cycle starts, at every
    /// smoothing.
    ///
    /// The model reads an animation at the start of its cycle wherever a shape
    /// has no coordinate to read it along, and folds the answer into the
    /// parameter it drives. An answer a rounding away from zero displaces that
    /// parameter where the animation was meant to leave it alone.
    #[test]
    fn a_sine_square_crosses_zero_where_its_cycle_starts() {
        for smoothing in SMOOTHINGS {
            for duty_cycle in DUTY_CYCLES {
                let value = sine_square(&WaveformArgs {
                    phase_spatial: Phase::ZERO,
                    phase_temporal: Phase::ZERO,
                    smoothing: UnipolarFloat::new(smoothing),
                    duty_cycle: UnipolarFloat::new(duty_cycle),
                    pulse: false,
                    standing: false,
                });
                // A hard square has no edge to cross zero on and is at the top
                // of its range from the first instant of the cycle.
                let expected = if smoothing == 0.0 { 1.0 } else { 0.0 };
                assert_eq!(
                    value, expected,
                    "a sine square at smoothing {smoothing} and duty cycle \
                     {duty_cycle} reads {value} where its cycle starts, not \
                     {expected}"
                );
            }
        }
    }

    /// A sine square's edges are cosine, so each one reaches a quarter of the
    /// period either side of where it sits at the top of the smoothing range,
    /// and arrives at the dwell in front of it with no slope left.
    ///
    /// The corner is what the shape is for. An edge that arrives at its dwell
    /// still climbing turns the brightest and darkest moments of the cycle into
    /// instants rather than into arrivals, which is the difference between a
    /// wave that breathes and one that snaps.
    #[test]
    fn a_smoothed_sine_square_reaches_its_dwells_without_a_corner() {
        // Both ends of the smoothing range are closed forms of their own, and
        // at the top of it the dwells have no width left to measure.
        for smoothing in SMOOTHINGS.into_iter().filter(|s| *s > 0.0 && *s < 0.95) {
            let edge = 0.25 * smoothing;
            let samples = period(
                sine_square,
                Sampling {
                    smoothing,
                    duty_cycle: 1.0,
                    pulse: false,
                },
            );

            // What the wave holds the ends of its range for, counted from
            // where the samples sit rather than from the width of a dwell,
            // because a sample never lands on an edge's boundary.
            let held = |level: f64| (0..SAMPLES).filter(|i| samples[*i] == level).count();
            let spanning = |range: std::ops::RangeInclusive<f64>| {
                (0..SAMPLES)
                    .filter(|i| range.contains(&window_phase(*i)))
                    .count()
            };
            assert_eq!(
                (held(1.0), held(-1.0)),
                (
                    spanning(edge..=0.5 - edge),
                    spanning(0.5 + edge..=1.0 - edge)
                ),
                "a sine square at smoothing {smoothing} holds the top of its range for \
                 {} of {SAMPLES} samples and the bottom for {}, where edges {edge} \
                 wide leave room for {} and {}",
                held(1.0),
                held(-1.0),
                spanning(edge..=0.5 - edge),
                spanning(0.5 + edge..=1.0 - edge)
            );

            // The falling edge, read as the steps between its samples. A
            // cosine's steepest step is at its centre and its last step is a
            // small fraction of that; a linear ramp's steps are all the same
            // size right up to the corner.
            let falling: Vec<f64> = (1..SAMPLES)
                .filter(|i| ((0.5 - edge)..=(0.5 + edge)).contains(&window_phase(*i)))
                .map(|i| samples[i - 1] - samples[i])
                .collect();
            let steepest = falling.iter().copied().fold(0.0, f64::max);
            let arriving = *falling.last().expect("an edge spans at least one step");
            assert!(
                arriving < steepest / 10.0,
                "a sine square at smoothing {smoothing} arrives at the bottom of its \
                 range on a step of {arriving}, against the {steepest} of its \
                 steepest, so the edge still has slope where it meets the dwell"
            );
        }
    }
}
