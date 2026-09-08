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
    let root = FsPath::new(env!("CARGO_MANIFEST_DIR"));
    let assets = root.join("assets/shapes");
    let manifest = root.join("assets/library.txt");
    println!("cargo:rerun-if-changed={}", assets.display());
    println!("cargo:rerun-if-changed={}", manifest.display());

    let families = read_manifest(&manifest);
    check_against(&families, &assets);

    let mut names = Vec::new();
    let mut blob = Vec::new();
    let mut sprites = Vec::new();
    for family in &families {
        for name in &family.members {
            let file = assets.join(format!("{name}.svg"));
            let figures = load(&file);
            assert!(!figures.is_empty(), "{name} has no filled paths");
            names.push(name.clone());
            sprites.push(figures);
        }
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

    writeln!(
        table,
        "\n/// Every family, in the order the family control offers them.\n\
         pub const SPRITE_FAMILIES: [SpriteFamily; {}] = [",
        families.len()
    )
    .unwrap();
    let mut first = 0u16;
    for family in &families {
        let len = u16::try_from(family.members.len()).expect("a family of 65536 figures");
        writeln!(
            table,
            "    SpriteFamily {{ name: {:?}, first: {first}, len: {len} }},",
            family.name
        )
        .unwrap();
        first += len;
    }
    writeln!(table, "];").unwrap();
}

/// One block of the manifest: a family and the figures under it, in order.
struct FamilyEntry {
    name: String,
    members: Vec<String>,
}

/// Read the manifest that gives the library its order.
///
/// A bracketed line opens a family and the bare lines under it name its
/// figures; `#` comments and blank lines are ignored. Ids are handed out by
/// reading top to bottom, so a family is always one contiguous run of them.
fn read_manifest(file: &FsPath) -> Vec<FamilyEntry> {
    let text =
        fs::read_to_string(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    let mut families: Vec<FamilyEntry> = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            families.push(FamilyEntry {
                name: name.to_string(),
                members: Vec::new(),
            });
        } else {
            let family = families
                .last_mut()
                .unwrap_or_else(|| panic!("{line} is listed before any family"));
            family.members.push(line.to_string());
        }
    }
    assert!(!families.is_empty(), "{} names no family", file.display());
    for family in &families {
        assert!(
            !family.members.is_empty(),
            "the {} family is empty",
            family.name
        );
    }
    families
}

/// Fail unless the manifest and the shape directory hold the same figures.
///
/// Both directions matter. A figure listed but absent would bake nothing under
/// a name the console offers; a figure present but unlisted would be
/// undrawable, and silently so.
fn check_against(families: &[FamilyEntry], assets: &FsPath) {
    let mut listed: Vec<&str> = families
        .iter()
        .flat_map(|f| f.members.iter().map(String::as_str))
        .collect();
    let mut present: Vec<String> = fs::read_dir(assets)
        .unwrap_or_else(|e| panic!("reading {}: {e}", assets.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_else(|| panic!("un-nameable shape file {}", p.display()))
                .to_string()
        })
        .collect();
    listed.sort_unstable();
    present.sort_unstable();

    let duplicated: Vec<&&str> = listed
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| &w[0])
        .collect();
    assert!(
        duplicated.is_empty(),
        "listed more than once: {duplicated:?}"
    );
    let missing: Vec<&&str> = listed
        .iter()
        .filter(|n| !present.contains(&n.to_string()))
        .collect();
    assert!(
        missing.is_empty(),
        "listed but not in {}: {missing:?}",
        assets.display()
    );
    let unplaced: Vec<&String> = present
        .iter()
        .filter(|n| !listed.contains(&n.as_str()))
        .collect();
    assert!(
        unplaced.is_empty(),
        "in {} but placed in no family: {unplaced:?}",
        assets.display()
    );
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
    recentre(&mut figures);
    figures
}

/// How far off centre a figure has to be before moving it is worth a redraw.
///
/// A couple of flattening tolerances. Below that the offset is the artwork's
/// own imprecision rather than a placement error, and correcting it moves the
/// figure by well under a projected pixel.
const CENTRE_FLOOR: f64 = 0.005;
/// Harmonics of the outline examined when looking for a centre of rotation.
const HARMONICS: usize = 24;
/// The largest rotational order looked for.
const MAX_FOLD: usize = 12;
/// How much of the outline's angular energy a rotational order may leave
/// unexplained and still be believed.
///
/// The library separates cleanly either side of this: figures that really do
/// turn onto themselves leave under a tenth, and the nearest thing that does
/// not — a hand, which is mirrored rather than rotational — leaves a fifth.
const FOLD_TOLERANCE: f64 = 0.12;
/// Outline pieces are split to at most this length before their angle is
/// taken, so a long straight edge contributes to the wedges it crosses rather
/// than to the one holding its midpoint.
const OUTLINE_STEP: f64 = 0.02;

/// A piece of a figure's outline: where it is, and how much of it there is.
struct Piece {
    mid: [f64; 2],
    len: f64,
}

/// Sit a figure that turns onto itself on the centre it turns about.
///
/// Centring on the bounding box puts the centre of a figure's extent at the
/// origin, which for a figure with a centre of rotation is not that centre —
/// a five-pointed star's box hangs below the point it spins around, and the
/// figure wobbles as it turns. Only odd-order figures are ever wrong this way:
/// an even order contains the half turn, which maps the box onto itself and so
/// pins its centre to the centre of rotation.
///
/// A figure with no centre to find is left alone, and so is one sitting close
/// enough to its centre. Moving a figure can push it past the unit box, so
/// what moves is scaled back to just fit.
fn recentre(figures: &mut [Figure]) {
    let Some(centre) = centre_of_rotation(figures) else {
        return;
    };
    if (centre[0] * centre[0] + centre[1] * centre[1]).sqrt() < CENTRE_FLOOR {
        return;
    }
    let (centre, mut reach) = ([centre[0] as f32, centre[1] as f32], 0f32);
    for figure in figures.iter_mut() {
        for subpath in &mut figure.subpaths {
            for p in subpath {
                p[0] -= centre[0];
                p[1] -= centre[1];
                reach = reach.max(p[0].abs()).max(p[1].abs());
            }
        }
    }
    let scale = 1.0 / reach.max(1e-6);
    for figure in figures {
        for subpath in &mut figure.subpaths {
            for p in subpath {
                p[0] *= scale;
                p[1] *= scale;
            }
        }
    }
}

/// The point a figure turns about, if it turns onto itself at all.
///
/// The candidate is the centroid of the outline weighted by arc length. That
/// measure asks nothing of the winding direction or the fill rule, both of
/// which vary across a library assembled from separate artwork, and a figure
/// invariant under a rotation has an outline invariant under it too — so the
/// outline's centroid is that rotation's centre exactly, not nearly.
///
/// Whether the figure really is invariant is then a separate question, and the
/// answer has to be no for anything merely mirrored, or the centroid of a hand
/// would be taken for the centre of one. It is settled by the angular Fourier
/// content of the outline about the candidate: an order-n figure puts all of
/// its energy on harmonics that are multiples of n, so the smallest fraction
/// any order leaves elsewhere says how well a rotation explains the figure.
/// The content is measured twice, weighted by arc length and by arc length
/// times radius, and the worse reading is the one that counts — length alone
/// cannot tell a long thin lobe from a short fat one facing the other way.
fn centre_of_rotation(figures: &[Figure]) -> Option<[f64; 2]> {
    let pieces = outline(figures);
    let total: f64 = pieces.iter().map(|p| p.len).sum();
    if total < 1e-9 {
        return None;
    }
    let centre = [
        pieces.iter().map(|p| p.mid[0] * p.len).sum::<f64>() / total,
        pieces.iter().map(|p| p.mid[1] * p.len).sum::<f64>() / total,
    ];

    let plain = harmonics(&pieces, centre, false);
    let radial = harmonics(&pieces, centre, true);
    let explained = (2..=MAX_FOLD)
        .map(|fold| unexplained(&plain, fold).max(unexplained(&radial, fold)))
        .fold(f64::INFINITY, f64::min);
    (explained <= FOLD_TOLERANCE).then_some(centre)
}

/// A figure's outline as pieces short enough to stand at a single angle.
fn outline(figures: &[Figure]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for figure in figures {
        for subpath in &figure.subpaths {
            for i in 0..subpath.len() {
                let p = subpath[i].map(f64::from);
                let q = subpath[(i + 1) % subpath.len()].map(f64::from);
                let len = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
                let splits = (len / OUTLINE_STEP).ceil().max(1.0) as usize;
                for s in 0..splits {
                    let t = (s as f64 + 0.5) / splits as f64;
                    pieces.push(Piece {
                        mid: [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t],
                        len: len / splits as f64,
                    });
                }
            }
        }
    }
    pieces
}

/// How much of the outline sits at each angular harmonic about a point.
///
/// Normalised by the total weight, so the readings mean the same thing for a
/// figure drawn heavy as for one drawn light.
fn harmonics(pieces: &[Piece], centre: [f64; 2], radial: bool) -> [f64; HARMONICS + 1] {
    let weigh = |p: &Piece| {
        if radial {
            let (dx, dy) = (p.mid[0] - centre[0], p.mid[1] - centre[1]);
            p.len * (dx * dx + dy * dy).sqrt()
        } else {
            p.len
        }
    };
    let total: f64 = pieces.iter().map(weigh).sum();
    let mut out = [0.0; HARMONICS + 1];
    if total < 1e-12 {
        return out;
    }
    for (k, slot) in out.iter_mut().enumerate().skip(1) {
        let (mut re, mut im) = (0.0, 0.0);
        for piece in pieces {
            let angle = (piece.mid[1] - centre[1]).atan2(piece.mid[0] - centre[0]) * k as f64;
            let w = weigh(piece);
            re += w * angle.cos();
            im += w * angle.sin();
        }
        *slot = (re * re + im * im).sqrt() / total;
    }
    out
}

/// The share of the outline's angular energy a rotation of this order would
/// not preserve.
///
/// Zero for a figure that turns onto itself at that order; a figure with no
/// angular structure at all reads zero too, which is right — it is a ring, and
/// every rotation preserves it.
fn unexplained(content: &[f64; HARMONICS + 1], fold: usize) -> f64 {
    let total: f64 = (1..=HARMONICS).map(|k| content[k] * content[k]).sum();
    if total < 1e-12 {
        return 0.0;
    }
    let off: f64 = (1..=HARMONICS)
        .filter(|k| k % fold != 0)
        .map(|k| content[k] * content[k])
        .sum();
    off / total
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
