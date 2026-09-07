//! Demo for judging whether a library of SVG shapes is worth building into
//! tunnels for texture projection.
//!
//! `control` opens an egui window of knobs and spawns `render`, which draws the
//! shapes through the same piston/OpenGL stack the real client uses so what you
//! see is what the client would produce. `sheet` writes PNGs headlessly.

mod anim;
mod bench;
mod draw;
mod fastmath;
mod mesh;
mod params;
mod ramp;
mod shapes;
mod sheet;
mod stats;
mod software;

#[cfg(feature = "windows")]
mod ab;
#[cfg(feature = "windows")]
mod compare;
#[cfg(feature = "windows")]
mod control;
#[cfg(feature = "windows")]
mod gpu;
#[cfg(feature = "windows")]
mod profile;
#[cfg(feature = "windows")]
mod render;

use anyhow::{Result, anyhow};
use std::path::PathBuf;

/// Where the shape library lives.
///
/// Checked in order so a binary can be copied to another machine — which is the
/// point, since the interesting hardware is not the machine that built it:
///
/// 1. `SVG_DEMO_SHAPES`, for an explicit location
/// 2. a `shapes` directory beside the executable
/// 3. the crate's own `shapes`, for `cargo run` during development
fn shape_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SVG_DEMO_SHAPES") {
        return PathBuf::from(dir);
    }
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("shapes")))
        .filter(|beside| beside.is_dir())
    {
        return beside;
    }
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
        "bench" => {
            let seconds: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(10.0);
            let layers: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(3);
            bench::run(&shape_dir(), seconds, layers)
        }
        "stats" => {
            let shapes = shapes::load_dir(&shape_dir())?;
            stats::report(&shapes);
            Ok(())
        }
        "anim" => {
            let shapes = shapes::load_dir(&shape_dir())?;
            let idx: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
            let out = args.next().unwrap_or_else(|| "/tmp/anim.png".into());
            sheet::anim_sheet(&shapes, idx.min(shapes.len() - 1), &PathBuf::from(&out), 260)?;
            println!("wrote {out}");
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
            // Default to no supersampling: the question a zoom answers is how
            // the mesh looks, and supersampling hides exactly that.
            let ss = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
            let target_px = args
                .next()
                .and_then(|a| a.parse().ok())
                .unwrap_or(mesh::DEFAULT_TARGET_PX);
            sheet::zoom(
                &shapes[idx.min(shapes.len() - 1)],
                &PathBuf::from(&out),
                900,
                phase,
                ss,
                target_px,
            )?;
            println!("wrote {out}");
            Ok(())
        }
        #[cfg(feature = "windows")]
        "profile" => {
            let mut opts = profile::Options::default();
            let rest: Vec<String> = args.collect();
            let mut i = 0;
            while i < rest.len() {
                let take = |i: usize| rest.get(i + 1).cloned().unwrap_or_default();
                match rest[i].as_str() {
                    "--layers" => {
                        opts.layers = take(i).parse().unwrap_or(opts.layers);
                        i += 2;
                    }
                    "--seconds" => {
                        opts.seconds = take(i).parse().unwrap_or(opts.seconds);
                        i += 2;
                    }
                    "--size" => {
                        if let Some((w, h)) = take(i).split_once('x') {
                            opts.width = w.parse().unwrap_or(opts.width);
                            opts.height = h.parse().unwrap_or(opts.height);
                        }
                        i += 2;
                    }
                    "--target-px" => {
                        opts.target_px = take(i).parse().unwrap_or(opts.target_px);
                        i += 2;
                    }
                    "--budget-hz" => {
                        opts.budget_hz = take(i).parse().unwrap_or(opts.budget_hz);
                        i += 2;
                    }
                    "--samples" => {
                        opts.samples = take(i).parse().unwrap_or(opts.samples);
                        i += 2;
                    }
                    "--shapes" => {
                        opts.shapes = take(i)
                            .split(',')
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                        i += 2;
                    }
                    "--gpu" => {
                        opts.gpu = true;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            if !opts.shapes.is_empty() {
                opts.layers = opts.shapes.len();
            }
            profile::run(&shape_dir(), opts)
        }
        #[cfg(feature = "windows")]
        "render" => {
            // Only the initial state: with a control window attached, its own
            // toggle takes over on the first packet.
            let gpu = args.any(|a| a == "--gpu");
            render::run(&shape_dir(), gpu)
        }
        #[cfg(feature = "windows")]
        "ab" => {
            let mut opts = ab::Options::default();
            let rest: Vec<String> = args.collect();
            let mut i = 0;
            while i < rest.len() {
                let take = |i: usize| rest.get(i + 1).cloned().unwrap_or_default();
                match rest[i].as_str() {
                    "--layers" => {
                        opts.layers = take(i)
                            .split(',')
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                        i += 2;
                    }
                    "--seconds" => {
                        opts.seconds = take(i).parse().unwrap_or(opts.seconds);
                        i += 2;
                    }
                    "--blocks" => {
                        opts.blocks = take(i).parse().unwrap_or(opts.blocks);
                        i += 2;
                    }
                    "--size" => {
                        if let Some((w, h)) = take(i).split_once('x') {
                            opts.width = w.parse().unwrap_or(opts.width);
                            opts.height = h.parse().unwrap_or(opts.height);
                        }
                        i += 2;
                    }
                    "--target-px" => {
                        opts.target_px = take(i).parse().unwrap_or(opts.target_px);
                        i += 2;
                    }
                    "--samples" => {
                        opts.samples = take(i).parse().unwrap_or(opts.samples);
                        i += 2;
                    }
                    "--budget-hz" => {
                        opts.budget_hz = take(i).parse().unwrap_or(opts.budget_hz);
                        i += 2;
                    }
                    "--shapes" => {
                        opts.shapes = take(i)
                            .split(',')
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            ab::run(&shape_dir(), opts)
        }
        #[cfg(feature = "windows")]
        "compare" => {
            let mut opts = compare::Options::default();
            let rest: Vec<String> = args.collect();
            let mut i = 0;
            while i < rest.len() {
                let take = |i: usize| rest.get(i + 1).cloned().unwrap_or_default();
                match rest[i].as_str() {
                    "--size" => {
                        if let Some((w, h)) = take(i).split_once('x') {
                            opts.width = w.parse().unwrap_or(opts.width);
                            opts.height = h.parse().unwrap_or(opts.height);
                        }
                        i += 2;
                    }
                    "--samples" => {
                        opts.samples = take(i).parse().unwrap_or(opts.samples);
                        i += 2;
                    }
                    "--tolerance" => {
                        opts.tolerance = take(i).parse().unwrap_or(opts.tolerance);
                        i += 2;
                    }
                    "--out" => {
                        opts.out_dir = PathBuf::from(take(i));
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            compare::run(&shape_dir(), opts)
        }
        #[cfg(feature = "windows")]
        "control" => control::run(&shape_dir()),
        other => Err(anyhow!(
            "unknown command {other:?}; expected control, render, profile, ab, compare, stats, sheet, features or zoom"
        )),
    }
}
