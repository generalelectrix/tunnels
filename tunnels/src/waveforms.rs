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
    /// This implements standing vs travelling wave behavior for all waveforms.
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
    let (amplitude, mut args) = args.spatial_params();
    // Fix bug where square pulse is 1 everywhere by compressing duty cycle.
    if args.pulse {
        args.duty_cycle = args.duty_cycle * UnipolarFloat::new(0.5);
    }
    amplitude * square_spatial(&args)
}

fn square_spatial(args: &WaveformArgsSpatial) -> f64 {
    if args.outside_duty_cycle() {
        return 0.0;
    }

    // internal smoothing scale is 0 to 0.25.
    let smoothing = args.smoothing * UnipolarFloat::new(0.25);

    let phase = args.duty_cycle_scaled_phase();
    if args.pulse {
        return square_spatial(&WaveformArgsSpatial {
            phase: phase * UnipolarFloat::new(0.5),
            smoothing,
            duty_cycle: UnipolarFloat::new(1.0),
            pulse: false,
        });
    }
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
    // internal smoothing scale is 0 to 0.25.
    let smoothing = args.smoothing * UnipolarFloat::new(0.25);
    let phase = args.duty_cycle_scaled_phase();

    if args.pulse {
        return sawtooth_spatial(&WaveformArgsSpatial {
            phase: phase * UnipolarFloat::new(0.5),
            smoothing,
            duty_cycle: UnipolarFloat::new(1.0),
            pulse: false,
        });
    }
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
#[allow(unused)]
mod test {
    use std::error::Error;

    use super::*;

    fn debug() -> Result<(), Box<dyn Error>> {
        use plotters::prelude::*;
        let points = generate_span(sawtooth_spatial, 0.1, 0.5, true);

        let root = BitMapBackend::new("waveform_test.png", (1600, 1200)).into_drawing_area();
        root.fill(&WHITE)?;
        let mut chart = ChartBuilder::on(&root)
            .margin(5)
            .x_label_area_size(30)
            .y_label_area_size(30)
            .build_cartesian_2d(-0.1f64..1.1f64, -1.5f64..1.5f64)?;

        chart.configure_mesh().draw()?;

        chart
            .draw_series(PointSeries::of_element(
                points.into_iter(),
                2,
                &RED,
                &|c, s, st| {
                    return EmptyElement::at(c)    // We want to construct a composed element on-the-fly
                + Circle::new((0,0),s,st.filled()) // At this point, the new pixel coordinate is established
               ;
                },
            ))?
            .label("waveform")
            .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &RED));

        chart
            .configure_series_labels()
            .background_style(&WHITE.mix(0.8))
            .border_style(&BLACK)
            .draw()?;
        Ok(())
    }

    fn generate_span(
        f: fn(&WaveformArgsSpatial) -> f64,
        smoothing: f64,
        duty_cycle: f64,
        pulse: bool,
    ) -> Vec<(f64, f64)> {
        (0..1000)
            .map(|i| i as f64 / 1000.)
            .map(Phase::new)
            .map(|angle| {
                (
                    angle.val(),
                    f(&WaveformArgsSpatial {
                        phase: angle,
                        smoothing: UnipolarFloat::new(smoothing),
                        duty_cycle: UnipolarFloat::new(duty_cycle),
                        pulse,
                    }),
                )
            })
            .collect()
    }
}
