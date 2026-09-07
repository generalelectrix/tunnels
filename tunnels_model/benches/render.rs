//! How long it takes to render one tunnel.
//!
//! The show loop renders every layer on every frame, so a tunnel's render cost
//! sets the ceiling on how many layers a show can carry. The stress
//! configuration is the interesting case: all four animations active, blacking
//! off so every one of the 126 segments draws.
//!
//! Reports nanoseconds per render. Run with `cargo bench -p tunnels_model`;
//! debug numbers are meaningless here, so a release profile is not optional.

use std::hint::black_box;
use std::time::{Duration, Instant};

use tunnels_lib::number::{BipolarFloat, UnipolarFloat};
use tunnels_model::clock_bank::ClockBank;
use tunnels_model::palette::ColorPalette;
use tunnels_model::position_bank::PositionBank;
use tunnels_model::render_context::RenderContext;
use tunnels_model::tunnel::{Tunnel, fixture::configure_stress};

/// Renders to run before timing, so the timed run measures warm code.
const WARMUP: usize = 200;

/// Renders per timed sample.
const BATCH: usize = 200;

/// Samples taken; the reported figure is the fastest, which is the one least
/// polluted by whatever else the machine was doing.
const SAMPLES: usize = 30;

fn bench(name: &str, tunnel: &Tunnel) {
    let clocks = ClockBank::default().as_static();
    let palette = ColorPalette::default();
    let positions = PositionBank::default();

    let render = || {
        black_box(tunnel.render(
            UnipolarFloat::ONE,
            false,
            RenderContext {
                clocks: &clocks,
                palette: &palette,
                positions: &positions,
                audio_envelope: UnipolarFloat::new(0.5),
            },
        ))
    };

    for _ in 0..WARMUP {
        render();
    }

    let mut best = Duration::MAX;
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..BATCH {
            render();
        }
        let elapsed = start.elapsed();
        best = best.min(elapsed);
    }

    let per_render = best.as_secs_f64() / BATCH as f64;
    println!("{name:<28} {:>10.0} ns/render", per_render * 1e9);
}

fn main() {
    let mut stress = Tunnel::default();
    configure_stress(&mut stress, BipolarFloat::new(-1.0));
    bench("stress (4 anims, 126 seg)", &stress);

    bench("default (no anims)", &Tunnel::default());
}
