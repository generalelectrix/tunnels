use eframe::egui::{self, Color32};
use egui_plot::{Line, Plot, PlotPoint, PlotPoints, Points};
use tunnels_lib::number::Phase;
use tunnels_model::spectrum::SpectrumTables;

use crate::animation::Animation;
use crate::clock_server::SharedClockData;

/// Snapshot of animation state for the visualizer panel.
#[derive(Default)]
pub struct AnimationSnapshot {
    pub animation: Animation,
    pub clocks: SharedClockData,
    /// How many places along the beam take a value from the animation, one
    /// per segment, evenly spaced across a cycle from its start.
    ///
    /// Each place reads the animation at its own phase, so its value lies on
    /// the scaled waveform, and the count marks points on that line without
    /// changing its shape.
    pub fixture_count: usize,
    /// Whether each of those places is also drawn as a point of its own.
    ///
    /// A run dense enough that its points merge into a line is already served
    /// by the line, so the points are for the case where there are few of them
    /// and each is worth picking out.
    pub show_fixture_values: bool,
}

#[derive(Default)]
pub struct VisualizerPanelState {
    preview: Vec<PlotPoint>,
    live: Vec<PlotPoint>,
    dots: Vec<PlotPoint>,
}

const NUM_WAVE_POINTS: usize = 1000;

impl VisualizerPanelState {
    /// Recompute plot data from the current animation snapshot.
    fn compute(&mut self, state: &AnimationSnapshot) {
        // The animation is the same for every point being plotted, so resolve
        // its frame-constant state once rather than a thousand times. The unit
        // waveform ignores the amplitude, so one preparation serves all three
        // plots.
        let spectrum = SpectrumTables::new(&state.clocks.audio.frame);
        let anim =
            state
                .animation
                .prepare(&state.clocks.clock_bank, &state.clocks.audio, &spectrum);

        // Unit waveform (amplitude always 1).
        self.preview.clear();
        self.preview.extend((0..NUM_WAVE_POINTS).map(|i| {
            let phase = i as f64 / NUM_WAVE_POINTS as f64;
            PlotPoint::new(phase, anim.unit_value(Phase::new(phase)))
        }));

        // Scaled waveform (applies audio envelope and animation scaling).
        self.live.clear();
        self.live.extend(
            self.preview
                .iter()
                .map(|point| PlotPoint::new(point.x, anim.scale_value(point.y))),
        );

        // Individual fixture dots, each at its own phase.
        self.dots.clear();
        if state.show_fixture_values {
            let phase_offset_per_fixture = 1.0 / state.fixture_count as f64;
            self.dots.extend((0..state.fixture_count).map(|i| {
                let phase = i as f64 * phase_offset_per_fixture;
                PlotPoint::new(phase, anim.value(Phase::new(phase)))
            }));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, state: &AnimationSnapshot) {
        self.compute(state);

        Plot::new("Animation")
            .default_x_bounds(0.0, 1.0)
            .default_y_bounds(-1.0, 1.0)
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Unit Waveform", PlotPoints::Borrowed(&self.preview))
                        .color(Color32::DARK_RED)
                        .width(2.0_f32),
                );
                plot_ui.line(
                    Line::new("Scaled Waveform", PlotPoints::Borrowed(&self.live))
                        .color(Color32::WHITE)
                        .width(2.0_f32),
                );
                // Named in the legend only when there is something to name, so
                // a plot that draws no points does not offer to hide them.
                if !self.dots.is_empty() {
                    plot_ui.points(
                        Points::new("Fixture Values", PlotPoints::Borrowed(&self.dots))
                            .color(Color32::CYAN)
                            .radius(5.0_f32),
                    );
                }
            });

        // Continuously repaint while the visualizer is active.
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tunnels_lib::audio::AudioState;

    #[test]
    fn compute_default_state() {
        let state = AnimationSnapshot::default();
        let mut panel = VisualizerPanelState::default();
        panel.compute(&state);

        assert_eq!(panel.preview.len(), NUM_WAVE_POINTS);
        assert_eq!(panel.live.len(), NUM_WAVE_POINTS);
        // Default fixture_count is 0, so no dots.
        assert_eq!(panel.dots.len(), 0);
    }

    #[test]
    fn compute_with_fixtures() {
        let state = AnimationSnapshot {
            fixture_count: 4,
            show_fixture_values: true,
            ..Default::default()
        };
        let mut panel = VisualizerPanelState::default();
        panel.compute(&state);

        assert_eq!(panel.preview.len(), NUM_WAVE_POINTS);
        assert_eq!(panel.live.len(), NUM_WAVE_POINTS);
        assert_eq!(panel.dots.len(), 4);

        // Dots should be evenly spaced across [0, 1).
        let phases: Vec<f64> = panel.dots.iter().map(|p| p.x).collect();
        assert!((phases[0] - 0.0).abs() < 1e-10);
        assert!((phases[1] - 0.25).abs() < 1e-10);
        assert!((phases[2] - 0.50).abs() < 1e-10);
        assert!((phases[3] - 0.75).abs() < 1e-10);
    }

    #[test]
    fn preview_x_values_span_unit_range() {
        let state = AnimationSnapshot::default();
        let mut panel = VisualizerPanelState::default();
        panel.compute(&state);

        assert!((panel.preview[0].x - 0.0).abs() < 1e-10);
        let last = panel.preview.last().unwrap();
        // Last point should be just under 1.0 (999/1000).
        assert!(last.x > 0.99 && last.x < 1.0);
    }

    #[test]
    fn live_has_same_x_as_preview() {
        let state = AnimationSnapshot {
            fixture_count: 2,
            ..Default::default()
        };
        let mut panel = VisualizerPanelState::default();
        panel.compute(&state);

        for (p, l) in panel.preview.iter().zip(panel.live.iter()) {
            assert!((p.x - l.x).abs() < 1e-10);
        }
    }

    #[test]
    fn recompute_clears_previous_data() {
        let mut panel = VisualizerPanelState::default();

        // First compute with 4 fixtures.
        panel.compute(&AnimationSnapshot {
            fixture_count: 4,
            show_fixture_values: true,
            ..Default::default()
        });
        assert_eq!(panel.dots.len(), 4);

        // Recompute with 2 fixtures -- dots should shrink.
        panel.compute(&AnimationSnapshot {
            fixture_count: 2,
            show_fixture_values: true,
            ..Default::default()
        });
        assert_eq!(panel.dots.len(), 2);
    }

    /// The plot is the function a beam renders: every point of the waveform
    /// is the animation's unit value at that phase, and every fixture's dot
    /// its scaled value at that fixture's phase. So neither how many places
    /// the beam has nor whether each is drawn changes the line, and noise
    /// comes out shaped by its smoothing at any count of places, one included.
    #[test]
    fn the_plot_is_the_animation_at_each_phase() {
        use crate::animation::{ControlMessage, EmitStateChange, StateChange, Waveform};
        use tunnels_lib::number::UnipolarFloat;

        struct Silent;
        impl EmitStateChange for Silent {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }

        let noise = |smoothing: f64| {
            let mut animation = Animation::default();
            for sc in [
                StateChange::Waveform(Waveform::Noise),
                StateChange::NPeriods(3),
                // Off full size, so a dot that drew the unit value instead of
                // the scaled one would be seen to.
                StateChange::Size(UnipolarFloat::new(0.5)),
                StateChange::Smoothing(UnipolarFloat::new(smoothing)),
            ] {
                animation.control(ControlMessage::Set(sc), &mut Silent);
            }
            // The smoothing control is reached over time rather than set.
            animation.update_state(std::time::Duration::from_secs(1), &AudioState::default());
            animation
        };
        let plot = |animation: &Animation, fixture_count: usize, show_fixture_values: bool| {
            let state = AnimationSnapshot {
                animation: animation.clone(),
                fixture_count,
                show_fixture_values,
                ..Default::default()
            };
            let mut panel = VisualizerPanelState::default();
            panel.compute(&state);
            let spectrum = SpectrumTables::new(&state.clocks.audio.frame);
            let rendered =
                state
                    .animation
                    .prepare(&state.clocks.clock_bank, &state.clocks.audio, &spectrum);
            for point in &panel.preview {
                assert_eq!(
                    point.y,
                    rendered.unit_value(Phase::new(point.x)),
                    "the waveform at {} is not what a beam renders there",
                    point.x
                );
            }
            for dot in &panel.dots {
                assert_eq!(
                    dot.y,
                    rendered.value(Phase::new(dot.x)),
                    "the dot at {} is not what a beam renders there",
                    dot.x
                );
            }
            panel
        };

        let rough = noise(0.0);
        let line = |panel: &VisualizerPanelState| -> Vec<f64> {
            panel.preview.iter().map(|p| p.y).collect()
        };
        let reference = line(&plot(&rough, 1, false));
        for (fixture_count, show_fixture_values) in
            [(1, true), (4, true), (126, true), (126, false)]
        {
            let panel = plot(&rough, fixture_count, show_fixture_values);
            assert_eq!(
                line(&panel),
                reference,
                "{fixture_count} places, drawn {show_fixture_values}, changed the waveform"
            );
            assert_eq!(
                panel.dots.len(),
                if show_fixture_values {
                    fixture_count
                } else {
                    0
                },
                "{fixture_count} places, drawn {show_fixture_values}, drew {} dots",
                panel.dots.len()
            );
        }

        let smooth = line(&plot(&noise(1.0), 1, false));
        let spread = reference
            .iter()
            .zip(&smooth)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            spread > 0.1,
            "turning the smoothing control moved the waveform by {spread}, which is \
             nothing a viewer would see"
        );
    }
}
