use eframe::egui::{self, Color32};
use egui_plot::{Line, Plot, PlotPoint, PlotPoints, Points};
use tunnels_lib::number::{Phase, UnipolarFloat};
use tunnels_model::animation::OffsetSpan;

use crate::animation::Animation;
use crate::clock_server::SharedClockData;

/// Snapshot of animation state for the visualizer panel.
#[derive(Default)]
pub struct AnimationSnapshot {
    pub animation: Animation,
    pub clocks: SharedClockData,
    /// How far the beam's offset axis runs, and how it is divided.
    ///
    /// Which place along the beam is asking is part of the question a waveform
    /// is asked — noise reads it directly, and the controls that spread an
    /// animation across a beam have nothing to act on without it — and the two
    /// kinds of beam measure it in different units. So this decides the shape
    /// of the plot, not just how finely it is sampled.
    pub spread: OffsetSpan,
    /// How many places along the beam are drawn as points of their own.
    pub fixture_count: usize,
    /// Whether those places are drawn as points at all.
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
        // its frame-constant state once rather than a thousand times. Prepared
        // against the beam's own spread, so what the plot draws is what the
        // beam is drawn with and not an approximation of it. The unit waveform
        // ignores the amplitude, so one preparation serves all three plots.
        let anim = state.animation.prepare(
            &state.clocks.clock_bank,
            state.clocks.audio_envelope,
            state.spread,
        );

        // The plot's own axis runs across the beam, so a sweep of it is a sweep
        // of the offset axis as well: the two are the same traversal, read in
        // the beam's units by the spread.
        let fraction = |i: usize, count: usize| UnipolarFloat::new(i as f64 / count as f64);

        // Unit waveform (amplitude always 1).
        self.preview.clear();
        self.preview.extend((0..NUM_WAVE_POINTS).map(|i| {
            let along = fraction(i, NUM_WAVE_POINTS);
            PlotPoint::new(
                along.val(),
                anim.unit_value(Phase::new(along.val()), state.spread.at(along)),
            )
        }));

        // Scaled waveform (applies audio envelope and animation scaling).
        self.live.clear();
        self.live.extend(
            self.preview
                .iter()
                .map(|point| PlotPoint::new(point.x, anim.scale_value(point.y))),
        );

        // Individual fixture dots.
        self.dots.clear();
        if state.show_fixture_values {
            self.dots.extend((0..state.fixture_count).map(|i| {
                let along = fraction(i, state.fixture_count);
                PlotPoint::new(
                    along.val(),
                    anim.value(Phase::new(along.val()), state.spread.at(along)),
                )
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
            spread: OffsetSpan::Segments(4),
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

    /// The count of places along the beam and the drawing of a point at each
    /// of them are separate questions: a dense run still has to be resolved at
    /// every one of its places for the waveform to come out right, while
    /// drawing all of them would bury the line they lie on.
    #[test]
    fn the_places_are_resolved_whether_or_not_each_is_drawn() {
        let plot = |show_fixture_values| {
            let mut panel = VisualizerPanelState::default();
            panel.compute(&AnimationSnapshot {
                spread: OffsetSpan::Segments(126),
                fixture_count: 126,
                show_fixture_values,
                ..Default::default()
            });
            (
                panel.preview.iter().map(|p| p.y).collect::<Vec<_>>(),
                panel.dots.len(),
            )
        };
        let (drawn, drawn_dots) = plot(true);
        let (undrawn, undrawn_dots) = plot(false);

        assert_eq!(drawn_dots, 126, "a plot that draws its places drew none");
        assert_eq!(
            undrawn_dots, 0,
            "a plot that draws no places drew {undrawn_dots}"
        );
        assert_eq!(
            drawn, undrawn,
            "the waveform changed shape according to whether its places were drawn"
        );
    }

    /// An animation that reads where along the beam it is being asked about —
    /// noise, through its cross-correlation — only answers differently at
    /// different places, so a plot that resolves every place at the first one
    /// silently drops every control that shapes the animation across the beam.
    #[test]
    fn a_waveform_that_reads_its_place_on_the_beam_is_shaped_by_the_controls() {
        use crate::animation::{ControlMessage, EmitStateChange, StateChange, Waveform};
        use tunnels_lib::number::UnipolarFloat;

        struct Silent;
        impl EmitStateChange for Silent {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }

        let plot = |smoothing: f64, spread: OffsetSpan| {
            let mut animation = Animation::default();
            for sc in [
                StateChange::Waveform(Waveform::Noise),
                StateChange::NPeriods(3),
                // No size: a plot draws the waveform an animation would make
                // whatever its amplitude, and an animation still being set up
                // is exactly the one an operator is watching the plot for.
                StateChange::Smoothing(UnipolarFloat::new(smoothing)),
            ] {
                animation.control(ControlMessage::Set(sc), &mut Silent);
            }
            // The smoothing control is reached over time rather than set.
            animation.update_state(std::time::Duration::from_secs(1), UnipolarFloat::ZERO);
            let mut panel = VisualizerPanelState::default();
            panel.compute(&AnimationSnapshot {
                animation,
                spread,
                ..Default::default()
            });
            panel.preview.iter().map(|p| p.y).collect::<Vec<_>>()
        };

        let reach = |spread| {
            plot(0.0, spread)
                .iter()
                .zip(plot(1.0, spread).iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max)
        };

        // Both kinds of beam, because the units the axis is measured in are the
        // one thing that differs between them and a plot that knew only one
        // would draw the other flat.
        for beam in [OffsetSpan::Segments(126), OffsetSpan::Figure] {
            assert!(
                reach(beam) > 0.1,
                "turning the control across {beam:?} moved the waveform by {}, \
                 which is nothing a viewer would see",
                reach(beam)
            );
        }

        // A beam with nowhere to spread is the degenerate case the bug wore:
        // every sample resolves at the same place, so there is no second place
        // to differ from.
        assert_eq!(
            reach(OffsetSpan::Segments(1)),
            0.0,
            "a run of one place somehow varies along itself"
        );
    }
}
