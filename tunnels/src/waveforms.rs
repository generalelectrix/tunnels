use std::f64::consts::PI;

use crate::numbers::{Phase, UnipolarFloat};

const TWO_PI: f64 = 2.0 * PI;
const HALF_PI: f64 = PI / 2.0;

pub fn sine(
    mut phase: Phase,
    _smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    if phase > duty_cycle || duty_cycle == 0.0 {
        return 0.0;
    }
    phase = phase / duty_cycle;
    if pulse {
        return ((TWO_PI * phase.val() - HALF_PI).sin() + 1.0) / 2.0;
    }
    (TWO_PI * phase.val()).sin()
}

pub fn triangle(
    mut phase: Phase,
    _smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    if phase > duty_cycle || duty_cycle == 0.0 {
        return 0.0;
    }
    phase = phase / duty_cycle;
    if pulse {
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

pub fn square(
    mut phase: Phase,
    mut smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    // internal smoothing scale is 0 to 0.25.
    smoothing = smoothing * UnipolarFloat::new(0.25);

    if phase > duty_cycle || duty_cycle == 0.0 {
        return 0.0;
    }
    phase = phase / duty_cycle;
    if pulse {
        return square(
            phase * UnipolarFloat::new(0.5),
            smoothing,
            UnipolarFloat::new(1.0),
            false,
        );
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

pub fn sawtooth(
    mut phase: Phase,
    mut smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    // internal smoothing scale is 0 to 0.25.
    smoothing = smoothing * UnipolarFloat::new(0.25);

    if phase > duty_cycle || duty_cycle == 0.0 {
        return 0.0;
    }
    phase = phase / duty_cycle;
    if pulse {
        return sawtooth(
            phase * UnipolarFloat::new(0.5),
            smoothing,
            UnipolarFloat::new(1.0),
            false,
        );
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
        let points = generate_span(sawtooth, 0.1, 0.5, true);

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
        f: fn(Phase, UnipolarFloat, UnipolarFloat, bool) -> f64,
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
                    f(
                        angle,
                        UnipolarFloat::new(smoothing),
                        UnipolarFloat::new(duty_cycle),
                        pulse,
                    ),
                )
            })
            .collect()
    }
}
