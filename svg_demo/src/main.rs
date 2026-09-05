//! Demo for judging whether a library of SVG shapes is worth building into
//! tunnels for texture projection.
//!
//! `control` opens an egui window of knobs and spawns `render`, which draws the
//! shapes through the same piston/OpenGL stack the real client uses so what you
//! see is what the client would produce. `sheet` writes PNGs headlessly.

mod draw;
mod params;
mod shapes;
mod sheet;
mod stats;
mod software;

#[cfg(feature = "windows")]
mod control;
#[cfg(feature = "windows")]
mod render;

use anyhow::{Result, anyhow};
use std::path::PathBuf;

/// Where the shape library lives, relative to the crate root.
fn shape_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shapes")
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "control".to_string());
    match cmd.as_str() {
        "sheet" => {
            let shapes = shapes::load_dir(&shape_dir())?;
            let out = args.next().unwrap_or_else(|| "/tmp/shapes.png".into());
            println!("{} shapes loaded", shapes.len());
            for (i, s) in shapes.iter().enumerate() {
                println!("  {i:>3}  {:<52} {} tris", s.name, s.triangle_count());
            }
            sheet::contact_sheet(&shapes, &PathBuf::from(&out), 150, 10)?;
            println!("wrote {out}");
            Ok(())
        }
        "features" => {
            let shapes = shapes::load_dir(&shape_dir())?;
            let out = args.next().unwrap_or_else(|| "/tmp/features.png".into());
            let picks: Vec<usize> = args
                .filter_map(|a| a.parse().ok())
                .filter(|i| *i < shapes.len())
                .collect();
            let picks = if picks.is_empty() {
                (0..shapes.len().min(6)).collect()
            } else {
                picks
            };
            sheet::feature_sheet(&shapes, &picks, &PathBuf::from(&out), 240)?;
            println!("wrote {out}");
            Ok(())
        }
        "stats" => {
            let shapes = shapes::load_dir(&shape_dir())?;
            stats::report(&shapes);
            Ok(())
        }
        "zoom" => {
            let shapes = shapes::load_dir(&shape_dir())?;
            let idx: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
            let out = args.next().unwrap_or_else(|| "/tmp/zoom.png".into());
            let phase = match args.next().as_deref() {
                Some("radius") => params::ColorPhase::Radius,
                Some("x") => params::ColorPhase::LinearX,
                Some("y") => params::ColorPhase::LinearY,
                _ => params::ColorPhase::Angle,
            };
            sheet::zoom(&shapes[idx.min(shapes.len() - 1)], &PathBuf::from(&out), 900, phase)?;
            println!("wrote {out}");
            Ok(())
        }
        #[cfg(feature = "windows")]
        "render" => render::run(&shape_dir()),
        #[cfg(feature = "windows")]
        "control" => control::run(&shape_dir()),
        other => Err(anyhow!(
            "unknown command {other:?}; expected control, render, sheet or features"
        )),
    }
}
