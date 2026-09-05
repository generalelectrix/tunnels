//! The control window: egui knobs, published to the render process over UDP.

use crate::params::{ColorPhase, DemoParams, DrawMode, LayerParams, PORT};
use crate::shapes::load_dir;
use anyhow::{Result, anyhow};
use eframe::egui;
use std::net::UdpSocket;
use std::path::Path;
use std::process::{Child, Command};

pub fn run(shape_dir: &Path) -> Result<()> {
    let names: Vec<String> = load_dir(shape_dir)?
        .into_iter()
        .map(|s| s.name)
        .collect();

    // Spawn the render window as a child, the way the console's admin panel
    // launches render clients. Both halves are still runnable on their own.
    let child = Command::new(std::env::current_exe()?)
        .arg("render")
        .spawn()
        .ok();

    let socket = UdpSocket::bind(("127.0.0.1", 0))?;
    socket.connect(("127.0.0.1", PORT))?;

    let app = ControlApp {
        params: DemoParams::default(),
        names,
        socket,
        child,
        selected: 0,
        filter: String::new(),
    };
    eframe::run_native(
        "svg_demo: control",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([460.0, 900.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(app))),
    )
    .map_err(|e| anyhow!("{e}"))
}

struct ControlApp {
    params: DemoParams,
    names: Vec<String>,
    socket: UdpSocket,
    child: Option<Child>,
    selected: usize,
    filter: String,
}

impl ControlApp {
    fn publish(&self) {
        if let Ok(bytes) = serde_json::to_vec(&self.params) {
            let _ = self.socket.send(&bytes);
        }
    }
}

impl eframe::App for ControlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for i in 0..self.params.layers.len() {
                    let on = self.params.layers[i].enabled;
                    let label = format!("layer {}{}", i + 1, if on { " *" } else { "" });
                    ui.selectable_value(&mut self.selected, i, label);
                }
            });
            ui.separator();

            let names = &self.names;
            let filter = self.filter.to_lowercase();
            let layer = &mut self.params.layers[self.selected];

            ui.checkbox(&mut layer.enabled, "enabled");
            ui.checkbox(&mut layer.mask, "mask (paint black — stacks like a gobo)");
            ui.separator();

            ui.label("shape");
            ui.text_edit_singleline(&mut self.filter);
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        if filter.is_empty() || name.to_lowercase().contains(&filter) {
                            ui.selectable_value(&mut layer.shape, i, name);
                        }
                    }
                });
            ui.separator();

            grid(ui, "geometry", |ui| {
                slider(ui, "scale x", &mut layer.scale_x, 0.0..=3.0);
                slider(ui, "scale y", &mut layer.scale_y, 0.0..=3.0);
                slider(ui, "rotation", &mut layer.rotation, 0.0..=1.0);
                slider(ui, "spin speed", &mut layer.spin_speed, -1.0..=1.0);
                slider(ui, "shear x", &mut layer.shear_x, -1.5..=1.5);
                slider(ui, "shear y", &mut layer.shear_y, -1.5..=1.5);
                slider(ui, "x", &mut layer.x, -1.0..=1.0);
                slider(ui, "y", &mut layer.y, -1.0..=1.0);
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("draw");
                for mode in DrawMode::ALL {
                    ui.selectable_value(&mut layer.draw_mode, mode, mode.label());
                }
            });
            if layer.draw_mode.draws_outline() {
                grid(ui, "stroke", |ui| {
                    slider(ui, "width", &mut layer.stroke_width, 0.002..=0.25);
                });
            }
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("color along");
                for phase in ColorPhase::ALL {
                    ui.selectable_value(&mut layer.color_phase, phase, phase.label());
                }
            });
            grid(ui, "color", |ui| {
                slider(ui, "col center", &mut layer.col_center, 0.0..=1.0);
                slider(ui, "col width", &mut layer.col_width, 0.0..=1.0);
                slider(ui, "col spread", &mut layer.col_spread, 0.0..=1.0);
                slider(ui, "col sat", &mut layer.col_sat, 0.0..=1.0);
                slider(ui, "level", &mut layer.level, 0.0..=1.0);
            });
            ui.separator();
            if ui.button("reset this layer").clicked() {
                let enabled = layer.enabled;
                *layer = LayerParams {
                    enabled,
                    ..Default::default()
                };
            }
        });

        // Cheap enough to send unconditionally, and it keeps the render window
        // in step with a freshly started control window.
        self.publish();
        ctx.request_repaint();
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
    }
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).show(ui, add);
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) {
    ui.label(label);
    ui.add(egui::Slider::new(value, range));
    ui.end_row();
}
