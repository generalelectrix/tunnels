//! Refining a triangle list to a uniform on-screen resolution.
//!
//! Mesh density is chosen from how large a figure is on screen and nothing
//! else. Keeping it independent of colour is what lets a refined mesh be
//! cached across frames while the colour on it changes freely — including
//! waveforms driven by a clock, which change every frame.

use super::geom::{IndexBatch, Triangle, TriangleList};
use std::collections::HashMap;
use tunnels_model::layer::SpriteId;
use tunnels_sprites::Point;

/// How much finer triangles get as they approach the origin.
///
/// The mesh carries phase, and angular phase varies without bound at the
/// centre: one triangle spanning the origin covers every angle there is, and
/// interpolating across it is meaningless. Tightening the limit near the centre
/// keeps that error bounded. This is a property of the coordinate, not of any
/// colour setting, so it costs the mesh no dependence on one.
const CENTRE_REFINEMENT: f32 = 12.0;

/// Radius, in figure space, inside which the extra refinement applies.
const CENTRE_RADIUS: f32 = 0.35;

/// Guard against a degenerate transform asking for an unbounded mesh.
const MAX_DEPTH: u32 = 24;

/// Coarsest mesh worth keeping: two units across is the whole figure, so a
/// quarter-unit edge already gives a few hundred triangles.
const COARSEST_LEVEL: i8 = -2;

/// Finest density built before the show starts.
///
/// This is where a figure at the default size knob lands on a 1080-line
/// projector, so it covers the common case and everything smaller. The two
/// finer ones are reachable — a figure's extent runs to twice the size knob,
/// crossing into the next at about 0.59 against a default of 0.5 — but they
/// are built on first use rather than up front, because together they are 90%
/// of both the time and the memory of building every density.
const EAGER_LEVEL: i8 = -5;

/// Finest mesh worth keeping.
///
/// Output resolution is bounded — a 1080-line projector with a figure filling
/// the frame puts one figure-space unit at 540 pixels — so past this the
/// triangles are smaller than a pixel and the extra ones buy nothing.
const FINEST_LEVEL: i8 = -7;

/// A refined figure, with vertices shared between the triangles that use them.
///
/// Sharing matters because phase is evaluated per vertex every frame: a flat
/// triangle list repeats each shared vertex about six times, and each repeat
/// would be another transcendental evaluated for an answer already known.
pub struct RefinedMesh {
    pub verts: Vec<Point>,
    indices: Vec<u32>,
}

impl RefinedMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Runs of whole triangles, each within `max_vertices`.
    pub fn batches(&self, max_vertices: usize) -> impl Iterator<Item = IndexBatch<'_>> {
        self.indices
            .chunks((max_vertices / 3 * 3).max(3))
            .map(IndexBatch::new)
    }
}

/// A mesh density, as the power of two giving its target edge length.
///
/// Bucketing is what makes the meshes reusable: a figure scaled anywhere
/// within a bucket draws from the same mesh, so a size knob steps between a
/// handful of prebuilt levels rather than rebuilding continuously.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Level(i8);

impl Level {
    /// The bucket whose triangles land nearest `target_px` on screen.
    ///
    /// `px_per_unit` is how many pixels one figure-space unit covers, which is
    /// the whole of what density depends on.
    ///
    /// A radial animation does not reach this. It scales each point where the
    /// vertices are walked, leaving the layer's extent untouched, so a figure
    /// animated to twice its size is drawn at the density of its unanimated
    /// size. Density follows the knob and not the animation. That is inherited
    /// from the prototype this was ported from rather than introduced here,
    /// and it is worth knowing before reading it as a bug.
    pub fn for_screen(px_per_unit: f64, target_px: f64) -> Self {
        let raw = target_px / px_per_unit.max(1.0);
        let exp = raw.log2().round();
        // `as i8` on a NaN or an enormous value saturates rather than wrapping,
        // and the clamp then puts it on a real level.
        Level((exp as i8).clamp(FINEST_LEVEL, COARSEST_LEVEL))
    }

    /// Target edge length in figure space.
    pub fn target_edge(self) -> f32 {
        2f32.powi(i32::from(self.0))
    }

    /// The densities built before the show starts, coarsest first.
    pub fn eager() -> impl Iterator<Item = Self> {
        (EAGER_LEVEL..=COARSEST_LEVEL).rev().map(Level)
    }

    /// Pixels one figure-space unit covers at this density's nominal size.
    ///
    /// The density a figure actually draws at is within a factor of root two
    /// of this, since levels are powers of two. Quantities that must not move
    /// continuously with the size knob — a stroke width bucket — are measured
    /// against this rather than against the real scale.
    pub fn nominal_px_per_unit(self, target_px: f64) -> f64 {
        target_px / f64::from(self.target_edge())
    }
}

/// Identifies one cached mesh.
///
/// Only a figure's interior is meshed. An outline takes its colour from the
/// contour rather than from where its vertices land, so nothing varies across
/// a ribbon for a mesh to carry — which is also what keeps the animated term
/// out of this key.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct MeshId {
    pub sprite: SpriteId,
    pub level: Level,
}

/// Meshes built so far, keyed by figure, what is being meshed, and density.
///
/// Never evicts, and nothing animated reaches the key: a figure is drawn at
/// one of six densities and a show touches a handful of figures, so the set
/// converges within seconds and nothing rebuilds mid-show. Building every
/// level of every figure up front would instead cost millions of triangles for
/// meshes that are never drawn.
#[derive(Default)]
pub struct MeshLibrary {
    built: HashMap<MeshId, RefinedMesh>,
}

impl MeshLibrary {
    /// The mesh for this id, building it on first use.
    pub fn get(&mut self, id: MeshId, source: &TriangleList) -> &RefinedMesh {
        self.built
            .entry(id)
            .or_insert_with(|| refine(source, id.level.target_edge()))
    }

    /// Total triangles held, for reporting memory pressure.
    pub fn triangles(&self) -> usize {
        self.built.values().map(RefinedMesh::triangle_count).sum()
    }
}

/// Split a triangle list until no edge is longer than `target`, sharing
/// vertices between the results.
///
/// Nothing here consults a colour setting. The mesh exists to carry phase,
/// which is a function of position alone, so it is built once per figure and
/// density and then holds for every colour a layer can take — including one
/// changing every frame.
fn refine(tris: &TriangleList, target: f32) -> RefinedMesh {
    let mut flat = Vec::new();
    for tri in tris.triangles() {
        bisect(tri, target, 0, &mut flat);
    }

    // Midpoints are computed as (a + b) / 2 from both triangles sharing an
    // edge, and float addition is commutative, so the two agree bit for bit and
    // this dedup finds them.
    let mut seen: HashMap<(u32, u32), u32> = HashMap::new();
    let mut verts: Vec<Point> = Vec::new();
    let mut indices = Vec::with_capacity(flat.len());
    for v in flat {
        let idx = *seen.entry(v.bits()).or_insert_with(|| {
            verts.push(v);
            (verts.len() - 1) as u32
        });
        indices.push(idx);
    }
    RefinedMesh { verts, indices }
}

/// Halve a triangle repeatedly until no edge is longer than the limit.
///
/// The limit tightens near the origin, where angular phase varies fastest, so
/// a triangle's own position decides how finely it is cut.
fn bisect(tri: Triangle, target: f32, depth: u32, out: &mut Vec<Point>) {
    // Triangles near the origin get a tighter limit, since angular phase varies
    // fastest there.
    let nearest = tri.nearest_radius();
    let limit = if nearest < CENTRE_RADIUS {
        let t = (nearest / CENTRE_RADIUS).clamp(0.0, 1.0);
        target * (1.0 / CENTRE_REFINEMENT).mul_add(1.0 - t, t)
    } else {
        target
    };

    if depth >= MAX_DEPTH || tri.longest_edge() <= limit {
        out.extend_from_slice(&tri.points());
        return;
    }

    for half in tri.split_longest() {
        bisect(half, target, depth + 1, out);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_level_buckets_scale_by_powers_of_two() {
        // Densities are powers of two, so scales a factor of two apart land on
        // adjacent levels and scales within a factor of root two land on one.
        let target = 14.0;
        let coarse = Level::for_screen(100.0, target);
        assert_eq!(coarse, Level::for_screen(120.0, target), "same bucket");
        assert!(
            Level::for_screen(400.0, target) < coarse,
            "a bigger figure gets a finer mesh"
        );
        // Both ends clamp rather than running away.
        assert_eq!(
            Level::for_screen(1e9, target),
            Level::for_screen(1e12, target)
        );
        assert_eq!(Level::for_screen(0.0, target), Level(COARSEST_LEVEL));
    }

    #[test]
    fn refining_shares_vertices_and_bounds_edge_length() {
        // One large triangle, refined well past its own size.
        let target = 0.25;
        let mut source = TriangleList::default();
        source.push(Triangle::new(
            Point::new(-1.0, -1.0),
            Point::new(1.0, -1.0),
            Point::new(0.0, 1.0),
        ));
        let mesh = refine(&source, target);
        assert!(mesh.triangle_count() > 1, "nothing was refined");

        let mut worst: f32 = 0.0;
        // A cap past the whole mesh gives one batch, which is every triangle.
        for tri in mesh.batches(usize::MAX).flat_map(IndexBatch::triangles) {
            let corners = tri.map(|i| mesh.verts[i as usize]);
            for (p, q) in [
                (corners[0], corners[1]),
                (corners[1], corners[2]),
                (corners[2], corners[0]),
            ] {
                worst = worst.max(p.distance(q));
            }
        }
        assert!(worst <= target, "an edge spans {worst}, over {target}");

        // Sharing is the point: a flat list would carry three vertices per
        // triangle and every interior one is used by more than one triangle.
        assert!(
            mesh.verts.len() < mesh.triangle_count() * 3,
            "{} vertices for {} triangles — nothing was shared",
            mesh.verts.len(),
            mesh.triangle_count()
        );
    }
}
