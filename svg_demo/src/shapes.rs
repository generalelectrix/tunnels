//! Loading SVG files and turning them into triangles piston can draw.
//!
//! Shapes are normalised into a unit box centred on the origin so that a
//! layer's scale knobs mean the same thing regardless of the source artwork's
//! coordinate system.

use anyhow::{Context, Result, anyhow};
use lyon_path::Path as LyonPath;
use lyon_path::math::point;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFillRule, FillTessellator, FillVertex,
    LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use std::path::Path as FsPath;

/// How finely curves are flattened, in normalised shape units. Shapes are two
/// units across, so this is roughly a thousandth of the figure's width — below
/// what a projector can resolve.
const TOLERANCE: f32 = 0.002;

/// A closed figure ready to draw, normalised into a unit box at the origin.
pub struct ShapeMesh {
    pub name: String,
    /// Flat triangle list of the shape's interior.
    pub fill: Vec<[f32; 2]>,
    /// The normalised outline, kept so strokes can be re-tessellated when the
    /// width knob moves.
    contours: Vec<(LyonPath, LyonFillRule)>,
}

impl ShapeMesh {
    /// Triangles tracing the shape's outline at the given width.
    pub fn stroke(&self, width: f32) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        let opts = StrokeOptions::tolerance(TOLERANCE)
            .with_line_width(width.max(1e-4))
            .with_line_join(LineJoin::Round)
            .with_line_cap(LineCap::Round);
        for (path, _) in &self.contours {
            let mut buf: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
            let mut builder =
                BuffersBuilder::new(&mut buf, |v: StrokeVertex| v.position().to_array());
            if StrokeTessellator::new()
                .tessellate_path(path, &opts, &mut builder)
                .is_ok()
            {
                expand(&buf, &mut out);
            }
        }
        out
    }

    pub fn triangle_count(&self) -> usize {
        self.fill.len() / 3
    }
}

/// Flatten indexed vertices into the flat triangle list piston's `tri_list`
/// wants, appending to `out`.
fn expand(buf: &VertexBuffers<[f32; 2], u32>, out: &mut Vec<[f32; 2]>) {
    out.extend(buf.indices.iter().map(|&i| buf.vertices[i as usize]));
}

/// Read every `.svg` in a directory, sorted by filename.
pub fn load_dir(dir: &FsPath) -> Result<Vec<ShapeMesh>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading shape directory {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .collect();
    files.sort();

    let mut shapes = Vec::new();
    for file in files {
        let name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        match load_file(&file) {
            Ok(mesh) => shapes.push(mesh),
            // A shape that will not tessellate is dropped rather than fatal —
            // the library is a grab bag and one bad file should not stop a show.
            Err(e) => eprintln!("skipping {name}: {e:#}"),
        }
    }
    if shapes.is_empty() {
        return Err(anyhow!("no usable shapes in {}", dir.display()));
    }
    Ok(shapes)
}

fn load_file(file: &FsPath) -> Result<ShapeMesh> {
    let data = std::fs::read(file)?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|e| anyhow!("parse: {e}"))?;

    let mut raw = Vec::new();
    collect(tree.root(), &mut raw);
    if raw.is_empty() {
        return Err(anyhow!("no filled paths"));
    }

    // Normalise: centre the union of all contours and scale its longest side to
    // fill [-1, 1]. Aspect ratio is preserved so the source art is not skewed
    // before the user's own scale knobs get to it.
    let (min, max) = bounds(&raw);
    let extent = ((max.0 - min.0).max(max.1 - min.1)).max(1e-6);
    let scale = 2.0 / extent;
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let norm = |x: f32, y: f32| point((x - cx) * scale, (y - cy) * scale);

    let mut contours = Vec::new();
    let mut fill = Vec::new();
    for (pts, rule) in raw {
        let mut builder = LyonPath::builder();
        let mut open = false;
        for cmd in &pts {
            match *cmd {
                Cmd::Move(x, y) => {
                    if open {
                        builder.end(false);
                    }
                    builder.begin(norm(x, y));
                    open = true;
                }
                Cmd::Line(x, y) => {
                    if open {
                        builder.line_to(norm(x, y));
                    }
                }
                Cmd::Quad(cx1, cy1, x, y) => {
                    if open {
                        builder.quadratic_bezier_to(norm(cx1, cy1), norm(x, y));
                    }
                }
                Cmd::Cubic(a, b, c, d, x, y) => {
                    if open {
                        builder.cubic_bezier_to(norm(a, b), norm(c, d), norm(x, y));
                    }
                }
                Cmd::Close => {
                    if open {
                        builder.end(true);
                        open = false;
                    }
                }
            }
        }
        if open {
            builder.end(true);
        }
        let path = builder.build();

        let opts = FillOptions::tolerance(TOLERANCE).with_fill_rule(rule);
        let mut buf: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
        let mut builder = BuffersBuilder::new(&mut buf, |v: FillVertex| v.position().to_array());
        if FillTessellator::new()
            .tessellate_path(&path, &opts, &mut builder)
            .is_ok()
        {
            expand(&buf, &mut fill);
        }
        contours.push((path, rule));
    }

    if fill.is_empty() {
        return Err(anyhow!("tessellated to nothing"));
    }
    Ok(ShapeMesh {
        name: file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string(),
        fill,
        contours,
    })
}

/// A path command in absolute SVG user units.
enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

fn bounds(paths: &[(Vec<Cmd>, LyonFillRule)]) -> ((f32, f32), (f32, f32)) {
    let (mut min, mut max) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    let mut see = |x: f32, y: f32| {
        min.0 = min.0.min(x);
        min.1 = min.1.min(y);
        max.0 = max.0.max(x);
        max.1 = max.1.max(y);
    };
    for (cmds, _) in paths {
        for cmd in cmds {
            match *cmd {
                Cmd::Move(x, y) | Cmd::Line(x, y) => see(x, y),
                Cmd::Quad(a, b, x, y) => {
                    see(a, b);
                    see(x, y);
                }
                Cmd::Cubic(a, b, c, d, x, y) => {
                    see(a, b);
                    see(c, d);
                    see(x, y);
                }
                Cmd::Close => {}
            }
        }
    }
    (min, max)
}

/// Walk the usvg tree, flattening every filled path into absolute coordinates.
fn collect(group: &usvg::Group, out: &mut Vec<(Vec<Cmd>, LyonFillRule)>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out),
            usvg::Node::Path(p) => {
                let Some(fill) = p.fill() else { continue };
                let rule = match fill.rule() {
                    usvg::FillRule::NonZero => LyonFillRule::NonZero,
                    usvg::FillRule::EvenOdd => LyonFillRule::EvenOdd,
                };
                let t = p.abs_transform();
                let map = |pt: usvg::tiny_skia_path::Point| {
                    (t.sx * pt.x + t.kx * pt.y + t.tx, t.ky * pt.x + t.sy * pt.y + t.ty)
                };
                let mut cmds = Vec::new();
                for seg in p.data().segments() {
                    match seg {
                        usvg::tiny_skia_path::PathSegment::MoveTo(a) => {
                            let (x, y) = map(a);
                            cmds.push(Cmd::Move(x, y));
                        }
                        usvg::tiny_skia_path::PathSegment::LineTo(a) => {
                            let (x, y) = map(a);
                            cmds.push(Cmd::Line(x, y));
                        }
                        usvg::tiny_skia_path::PathSegment::QuadTo(a, b) => {
                            let (ax, ay) = map(a);
                            let (bx, by) = map(b);
                            cmds.push(Cmd::Quad(ax, ay, bx, by));
                        }
                        usvg::tiny_skia_path::PathSegment::CubicTo(a, b, c) => {
                            let (ax, ay) = map(a);
                            let (bx, by) = map(b);
                            let (cx, cy) = map(c);
                            cmds.push(Cmd::Cubic(ax, ay, bx, by, cx, cy));
                        }
                        usvg::tiny_skia_path::PathSegment::Close => cmds.push(Cmd::Close),
                    }
                }
                if !cmds.is_empty() {
                    out.push((cmds, rule));
                }
            }
            _ => {}
        }
    }
}
