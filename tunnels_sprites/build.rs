//! Turns the SVG library in `assets/shapes` into a blob of flattened contours.
//!
//! SVG parsing is a build-time concern and stays one: a render client is
//! pushed to a machine mid-show and loads no files, so the figures have to be
//! in the binary by the time it starts.
//!
//! What comes out is contours rather than triangles. Every figure here is
//! drawn by an even-odd fill, where a ring is an outer loop and an inner loop
//! with the parity between them subtracting one from the other. Triangulate
//! the two loops without that rule and the ring becomes a disc — a different
//! figure, not a coarser one. Contours keep the rule available to whoever
//! fills them, and they are also what a stroke needs, so outline mode falls
//! out of the same data.

use lyon_path::Path;
use lyon_path::PathEvent;
use lyon_path::iterator::PathIterator;
use lyon_path::math::point;
use std::fs;
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};

/// How finely curves are flattened, in normalised figure units.
///
/// A figure is two units across, so this is about a thousandth of its width —
/// under a pixel for a figure filling a 1080-line projector, which is the only
/// place the difference could show.
const TOLERANCE: f32 = 0.002;

/// Winding rule tags, matching the discriminants `sprites.rs` reads back.
const RULE_NONZERO: u8 = 0;
const RULE_EVENODD: u8 = 1;

fn main() {
    let assets = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("assets/shapes");
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut files: Vec<PathBuf> = fs::read_dir(&assets)
        .unwrap_or_else(|e| panic!("reading {}: {e}", assets.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .collect();
    // Sorted by filename, because the sort order is the sprite id: the console
    // and the client agree on which figure a number means by both deriving it
    // from this directory in this order.
    files.sort();

    let mut names = Vec::with_capacity(files.len());
    let mut blob = Vec::new();
    let mut sprites = Vec::new();
    for file in &files {
        let name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_else(|| panic!("un-nameable shape file {}", file.display()));
        let figures = load(file);
        assert!(!figures.is_empty(), "{name} has no filled paths");
        names.push(name.to_string());
        sprites.push(figures);
    }

    write_u32(&mut blob, sprites.len());
    for figures in &sprites {
        write_u32(&mut blob, figures.len());
        for figure in figures {
            blob.push(figure.rule);
            write_u32(&mut blob, figure.subpaths.len());
            for subpath in &figure.subpaths {
                write_u32(&mut blob, subpath.len());
                for p in subpath {
                    blob.extend_from_slice(&p[0].to_le_bytes());
                    blob.extend_from_slice(&p[1].to_le_bytes());
                }
            }
        }
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out.join("sprites.bin"), &blob).expect("writing the contour blob");

    let mut table = fs::File::create(out.join("sprite_names.rs")).expect("writing the name table");
    writeln!(
        table,
        "/// Every baked figure's name, indexed by sprite id.\npub const SPRITE_NAMES: [&str; {}] = [",
        names.len()
    )
    .unwrap();
    for name in &names {
        writeln!(table, "    {name:?},").unwrap();
    }
    writeln!(table, "];").unwrap();
}

fn write_u32(out: &mut Vec<u8>, n: usize) {
    let n = u32::try_from(n).expect("a figure with more than 4 billion of anything");
    out.extend_from_slice(&n.to_le_bytes());
}

/// One `<path>` element's subpaths, which its fill rule applies across.
struct Figure {
    rule: u8,
    subpaths: Vec<Vec<[f32; 2]>>,
}

/// One `<path>` element before flattening, in the figure's own coordinates.
struct Outline {
    rule: u8,
    path: Path,
}

/// Read one SVG into figures normalised into a unit box at the origin.
///
/// Normalising is what makes a beam's size knob mean the same thing whatever
/// coordinate system the source artwork used: the union of every contour is
/// centred and its longest side scaled to fill [-1, 1], with aspect ratio
/// preserved so the artwork is not skewed before the knobs get to it.
fn load(file: &FsPath) -> Vec<Figure> {
    let data = fs::read(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .unwrap_or_else(|e| panic!("parsing {}: {e}", file.display()));

    let mut outlines = Vec::new();
    collect(tree.root(), &mut outlines);

    // Flattening tolerance is fixed in *normalised* units, so it has to be
    // converted into the source's before anything is flattened. The control
    // hull overestimates the figure's extent, which makes the converted
    // tolerance conservative rather than coarse.
    let hull = hull_extent(&outlines);
    let mut figures: Vec<Figure> = outlines
        .into_iter()
        .map(|o| Figure {
            rule: o.rule,
            subpaths: flatten(&o.path, TOLERANCE * hull / 2.0),
        })
        .filter(|f| !f.subpaths.is_empty())
        .collect();

    let (min, max) = bounds(&figures);
    let extent = ((max[0] - min[0]).max(max[1] - min[1])).max(1e-6);
    let scale = 2.0 / extent;
    let centre = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    for figure in &mut figures {
        for subpath in &mut figure.subpaths {
            for p in subpath {
                p[0] = (p[0] - centre[0]) * scale;
                p[1] = (p[1] - centre[1]) * scale;
            }
        }
    }
    figures
}

/// The longest side of the box holding every control point.
fn hull_extent(outlines: &[Outline]) -> f32 {
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for outline in outlines {
        for event in outline.path.iter() {
            for p in event_points(event) {
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
        }
    }
    ((max[0] - min[0]).max(max[1] - min[1])).max(1e-6)
}

/// Every point a path event names, control points included.
fn event_points(event: PathEvent) -> Vec<[f32; 2]> {
    match event {
        PathEvent::Begin { at } => vec![at.to_array()],
        PathEvent::Line { to, .. } => vec![to.to_array()],
        PathEvent::Quadratic { ctrl, to, .. } => vec![ctrl.to_array(), to.to_array()],
        PathEvent::Cubic {
            ctrl1, ctrl2, to, ..
        } => vec![ctrl1.to_array(), ctrl2.to_array(), to.to_array()],
        PathEvent::End { .. } => Vec::new(),
    }
}

fn bounds(figures: &[Figure]) -> ([f32; 2], [f32; 2]) {
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for figure in figures {
        for subpath in &figure.subpaths {
            for p in subpath {
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
        }
    }
    (min, max)
}

/// Walk the usvg tree, gathering every filled path in absolute coordinates.
fn collect(group: &usvg::Group, out: &mut Vec<Outline>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out),
            usvg::Node::Path(p) => {
                let Some(fill) = p.fill() else { continue };
                let rule = match fill.rule() {
                    usvg::FillRule::NonZero => RULE_NONZERO,
                    usvg::FillRule::EvenOdd => RULE_EVENODD,
                };
                out.push(Outline {
                    rule,
                    path: build(p),
                });
            }
            _ => {}
        }
    }
}

/// One usvg path as a lyon path, with its transform baked in.
fn build(path: &usvg::Path) -> Path {
    let t = path.abs_transform();
    let map = |pt: usvg::tiny_skia_path::Point| {
        point(
            t.sx * pt.x + t.kx * pt.y + t.tx,
            t.ky * pt.x + t.sy * pt.y + t.ty,
        )
    };

    let mut builder = Path::builder();
    let mut open = false;
    for seg in path.data().segments() {
        use usvg::tiny_skia_path::PathSegment::*;
        match seg {
            MoveTo(a) => {
                if open {
                    builder.end(true);
                }
                builder.begin(map(a));
                open = true;
            }
            LineTo(a) if open => {
                builder.line_to(map(a));
            }
            QuadTo(a, b) if open => {
                builder.quadratic_bezier_to(map(a), map(b));
            }
            CubicTo(a, b, c) if open => {
                builder.cubic_bezier_to(map(a), map(b), map(c));
            }
            Close if open => {
                builder.end(true);
                open = false;
            }
            // A segment before any move-to has no start point to draw from.
            _ => {}
        }
    }
    if open {
        // Every contour closes, whether or not the source said so: a fill is
        // an area, and an open one is only an area by implication anyway.
        builder.end(true);
    }
    builder.build()
}

/// A path's subpaths as polylines, at the given tolerance.
fn flatten(path: &Path, tolerance: f32) -> Vec<Vec<[f32; 2]>> {
    let mut subpaths: Vec<Vec<[f32; 2]>> = Vec::new();
    for event in path.iter().flattened(tolerance) {
        match event {
            PathEvent::Begin { at } => subpaths.push(vec![at.to_array()]),
            PathEvent::Line { to, .. } => {
                if let Some(current) = subpaths.last_mut() {
                    current.push(to.to_array());
                }
            }
            PathEvent::End { .. } => {}
            // `flattened` emits no curves.
            PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => {}
        }
    }
    // A subpath of fewer than three points encloses nothing.
    subpaths.retain(|s| s.len() > 2);
    subpaths
}
