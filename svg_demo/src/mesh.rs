//! Refining a triangle list to a uniform resolution.
//!
//! Mesh density is chosen from how large the shape is on screen and nothing
//! else. Keeping it independent of color is what lets the refined mesh be
//! cached across frames while the color on it changes freely — including
//! waveforms driven by a clock, which change every frame.

use std::collections::HashMap;

/// Default for how many pixels a refined triangle's longest edge should span.
///
/// The mesh carries phase, not color — color is resolved per fragment against
/// the ramp texture. Phase is smooth, so this only has to be fine enough that a
/// straight line approximates `atan2` or a square root over one triangle, which
/// is a far weaker requirement than approximating a hue sweep was. Triangle
/// count goes as the inverse square of this, so it is the strongest lever there
/// is on per-frame cost.
pub const DEFAULT_TARGET_PX: f64 = 14.0;

/// How much finer triangles get as they approach the origin.
///
/// The mesh carries phase, and angular phase varies without bound at the
/// centre: one triangle spanning the origin covers every angle there is, and
/// interpolating across it is meaningless. Tightening the limit near the centre
/// keeps that error bounded. This is a property of the coordinate, not of any
/// color setting, so it costs the mesh no dependence on one.
const CENTRE_REFINEMENT: f32 = 12.0;

/// Radius, in shape space, inside which the extra refinement applies.
const CENTRE_RADIUS: f32 = 0.35;

/// Guard against a degenerate transform asking for an unbounded mesh.
const MAX_DEPTH: u32 = 24;

/// A refined shape, with vertices shared between the triangles that use them.
///
/// Sharing matters because color is evaluated per vertex every frame: a flat
/// triangle list repeats each shared vertex about six times, and each repeat
/// would be another transcendental evaluated for an answer already known.
pub struct RefinedMesh {
    pub verts: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl RefinedMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Coarsest mesh worth keeping: two units across is the whole shape, so a
/// quarter-unit edge already gives a few hundred triangles.
const COARSEST_LEVEL: i8 = -2;

/// Finest mesh worth keeping.
///
/// Output resolution is bounded — a 1080-line projector with a shape filling
/// the frame puts one shape-space unit at 540 pixels — so past this the
/// triangles are smaller than a pixel and the extra ones buy nothing.
const FINEST_LEVEL: i8 = -7;

/// A mesh density, as the power of two giving its target edge length.
///
/// Bucketing is what makes the meshes reusable: a shape scaled anywhere within
/// a bucket draws from the same mesh, so a scale slider steps between a handful
/// of prebuilt levels rather than rebuilding continuously.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Level(pub i8);

impl Level {
    /// The bucket whose triangles land nearest `target_px` on screen.
    pub fn for_scale(scale: f64, critical: f64, target_px: f64) -> Self {
        // A shape spans two units, so one unit covers `scale * critical / 2`
        // pixels.
        let px_per_unit = (scale.abs() * critical / 2.0).max(1.0);
        let raw = target_px / px_per_unit;
        let exp = raw.log2().round() as i8;
        Level(exp.clamp(FINEST_LEVEL, COARSEST_LEVEL))
    }

    /// Target edge length in shape space.
    pub fn target_edge(self) -> f32 {
        2f32.powi(i32::from(self.0))
    }

    /// Every level, coarsest first.
    pub fn all() -> impl Iterator<Item = Level> {
        (FINEST_LEVEL..=COARSEST_LEVEL).rev().map(Level)
    }
}

/// Meshes built so far, keyed by shape, what is being meshed, and density.
///
/// Never evicts. A show touches a handful of shapes at a handful of sizes, so
/// the set converges quickly and nothing rebuilds mid-show; building every
/// level of every shape up front would instead cost millions of triangles for
/// meshes that are never drawn.
#[derive(Default)]
pub struct MeshLibrary {
    built: HashMap<MeshId, RefinedMesh>,
}

/// Identifies one cached mesh.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct MeshId {
    pub shape: usize,
    /// `None` for the shape's fill, or the quantised stroke width for its
    /// outline.
    pub stroke_width: Option<u32>,
    pub level: Level,
}

impl MeshLibrary {
    /// The mesh for this id, building it on first use.
    pub fn get(&mut self, id: MeshId, source: &[[f32; 2]]) -> &RefinedMesh {
        self.built
            .entry(id)
            .or_insert_with(|| refine(source, id.level.target_edge()))
    }

    pub fn len(&self) -> usize {
        self.built.len()
    }

    /// Total triangles held, for reporting memory pressure.
    pub fn triangles(&self) -> usize {
        self.built.values().map(RefinedMesh::triangle_count).sum()
    }
}

/// Split a triangle list until no edge is longer than `target`, sharing
/// vertices between the results.
///
/// Nothing here consults a color setting. The mesh exists to carry phase, which
/// is a function of position alone, so it is built once per shape and density
/// and then holds for every color a layer can take — including one changing
/// every frame.
pub fn refine(tris: &[[f32; 2]], target: f32) -> RefinedMesh {
    let mut flat = Vec::new();
    for tri in tris.chunks(3) {
        if let [a, b, c] = tri {
            bisect([*a, *b, *c], target, 0, &mut flat);
        }
    }

    // Midpoints are computed as (a + b) / 2 from both triangles sharing an
    // edge, and float addition is commutative, so the two agree bit for bit and
    // this dedup finds them.
    let mut seen: HashMap<(u32, u32), u32> = HashMap::new();
    let mut verts: Vec<[f32; 2]> = Vec::new();
    let mut indices = Vec::with_capacity(flat.len());
    for v in flat {
        let key = (v[0].to_bits(), v[1].to_bits());
        let idx = *seen.entry(key).or_insert_with(|| {
            verts.push(v);
            (verts.len() - 1) as u32
        });
        indices.push(idx);
    }
    RefinedMesh { verts, indices }
}

/// Bisect the longest edge until every edge is short enough.
///
/// Splitting only the longest edge refines a long thin triangle along its
/// length rather than shattering it in both directions, which matters because a
/// fill tessellator emits plenty of them.
fn bisect(tri: [[f32; 2]; 3], target: f32, depth: u32, out: &mut Vec<[f32; 2]>) {
    let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let lengths = [
        dist(tri[0], tri[1]),
        dist(tri[1], tri[2]),
        dist(tri[2], tri[0]),
    ];
    let cut = (0..3).fold(0, |best, i| if lengths[i] > lengths[best] { i } else { best });

    // Triangles near the origin get a tighter limit, since angular phase varies
    // fastest there.
    let nearest = tri
        .iter()
        .map(|v| (v[0] * v[0] + v[1] * v[1]).sqrt())
        .fold(f32::MAX, f32::min);
    let limit = if nearest < CENTRE_RADIUS {
        let t = (nearest / CENTRE_RADIUS).clamp(0.0, 1.0);
        target * (1.0 / CENTRE_REFINEMENT).mul_add(1.0 - t, t)
    } else {
        target
    };

    if depth >= MAX_DEPTH || lengths[cut] <= limit {
        out.extend_from_slice(&tri);
        return;
    }

    let (i, j, k) = match cut {
        0 => (0, 1, 2),
        1 => (1, 2, 0),
        _ => (2, 0, 1),
    };
    let mid = [
        (tri[i][0] + tri[j][0]) / 2.0,
        (tri[i][1] + tri[j][1]) / 2.0,
    ];
    bisect([tri[i], mid, tri[k]], target, depth + 1, out);
    bisect([mid, tri[j], tri[k]], target, depth + 1, out);
}
