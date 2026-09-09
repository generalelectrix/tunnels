//! Refining a triangle list to a uniform on-screen resolution.
//!
//! Mesh density is chosen from how large a figure is on screen and nothing
//! else. Keeping it independent of colour is what lets a refined mesh be
//! cached across frames while the colour on it changes freely — including
//! waveforms driven by a clock, which change every frame.

use super::geom::{IndexBatch, Triangle, TriangleList};
use std::collections::HashMap;
use tunnels_model::layer::FigureId;
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

/// How many pixels a filled figure's mesh triangles should span on screen.
///
/// Triangle count goes as the inverse square of this, so it is the strongest
/// lever there is on what a figure costs per frame.
///
/// A figure's mesh carries phase, not colour: colour is resolved per fragment
/// against the ramp. Phase is smooth, so the mesh only has to be fine enough
/// that a straight line approximates an arctangent or a square root over one
/// triangle, which is a far weaker requirement than approximating a hue sweep
/// would be.
///
/// One number covers every output resolution, because density is chosen from
/// how many pixels a figure actually covers: a larger frame puts more pixels
/// on the same figure and reaches a finer level on its own.
const TARGET_PX: f64 = 14.0;

/// Finest mesh worth keeping.
///
/// Output resolution is bounded — a 1080-line projector with a figure filling
/// the frame puts one figure-space unit at 540 pixels — so past this the
/// triangles are smaller than a pixel and the extra ones buy nothing.
const FINEST_LEVEL: i8 = -7;

/// Steps of the grid a mesh's vertices are stored on, in one figure-space unit.
///
/// A vertex is kept as a pair of `i16` rather than a pair of `f32`, halving
/// what a mesh's vertices weigh. This is the whole of the mapping: the pair
/// spans **±4 figure units**, which is a power of two and so exact both ways,
/// and every figure either library can draw sits inside it — the furthest,
/// a star lattice, reaches 2.4314, which leaves the range 1.64 times wider
/// than anything drawn on it. It clamps rather than wrapping, so a figure that
/// somehow ran past the range would be folded onto its edge instead of
/// appearing on the far side; [`every_figure_fits_the_grid_it_is_stored_on`]
/// is what keeps that from being reached.
///
/// **The resolution is four orders of magnitude finer than the mesh it
/// carries.** One step is 1/8192 of a figure unit, which at 1920 lines with
/// the figure filling the frame is 0.23 of a pixel, against triangles refined
/// to fourteen; the whole grid is 128 steps across one triangle edge at the
/// density that a screen reaches.
///
/// That it is invisible was measured rather than argued, over all 401 figures
/// rendered at 1024 and at 1920: **snapping disturbs the raster less than
/// translating the same unsnapped figure by a sixteenth of a pixel does**, on
/// coverage IoU, on a two-sided Hausdorff distance, and on how many pixels
/// change at all. Snapping also moves the figure's own area by 0.011% across
/// the library and by 1.08% on the worst single figure, so nothing thin is
/// being swallowed.
///
/// **The decode is what makes it worth doing.** The per-vertex pass already
/// touches every vertex every frame to work out polar coordinates and phase,
/// so widening an `i16` there disappears beside the arctangent beside it. The
/// stored form never has to be the form a backend sees.
const QUANTISATION: f32 = 8192.0;

/// A mesh vertex, snapped to the grid [`QUANTISATION`] describes.
///
/// Equality is equality of the stored cell, which is what a refinement dedups
/// on: two vertices that land in one cell are one vertex, and the triangle
/// between them collapses to no area and draws nothing.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct StoredPoint([i16; 2]);

impl StoredPoint {
    fn of(p: Point) -> Self {
        let snap = |v: f32| {
            (v * QUANTISATION)
                .round()
                .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
        };
        Self([snap(p.x()), snap(p.y())])
    }

    fn widen(self) -> Point {
        Point::new(
            f32::from(self.0[0]) / QUANTISATION,
            f32::from(self.0[1]) / QUANTISATION,
        )
    }
}

/// A mesh's triangles, as indices into its vertices, in the narrowest width
/// that addresses them.
///
/// **The width is a property of one mesh and not of the library**, because the
/// two ends of the library are three orders of magnitude apart: the smallest
/// figure refines to a thousand vertices at the coarsest density and the
/// largest to 354,089 at the finest, which no `u16` reaches. Picking one width
/// for all of them would either be `u32` everywhere, which is what this
/// replaces, or a cap on how finely a figure may be refined.
///
/// Indices are two-thirds of what a mesh weighs — a triangle costs three of
/// them against the two stored coordinates of about half a vertex — so this is
/// the term worth narrowing first. It halves the coarse densities, where every
/// mesh fits, and does almost nothing at the finest, where the meshes that do
/// not fit hold nine tenths of the triangles.
enum Indices {
    Narrow(Vec<u16>),
    Wide(Vec<u32>),
}

impl Indices {
    /// The indices of `flat`, narrowed if every one of them fits.
    fn of(flat: Vec<u32>, vertices: usize) -> Self {
        if vertices > usize::from(u16::MAX) + 1 {
            return Self::Wide(flat);
        }
        Self::Narrow(flat.into_iter().map(|i| i as u16).collect())
    }

    fn len(&self) -> usize {
        match self {
            Self::Narrow(i) => i.len(),
            Self::Wide(i) => i.len(),
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Narrow(i) => i.len() * size_of::<u16>(),
            Self::Wide(i) => i.len() * size_of::<u32>(),
        }
    }
}

/// A refined figure, with vertices shared between the triangles that use them.
///
/// Sharing matters because phase is evaluated per vertex every frame: a flat
/// triangle list repeats each shared vertex about six times, and each repeat
/// would be another transcendental evaluated for an answer already known.
pub struct RefinedMesh {
    verts: Vec<StoredPoint>,
    indices: Indices,
}

impl RefinedMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.len() == 0
    }

    /// Every vertex, in the order the indices address them, widened back into
    /// figure space.
    pub fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.verts.iter().map(|v| v.widen())
    }

    /// What this mesh weighs, which is what the caches holding it are sized
    /// against.
    fn bytes(&self) -> usize {
        self.verts.len() * size_of::<StoredPoint>() + self.indices.bytes()
    }

    /// Runs of whole triangles, each within `max_vertices`.
    pub fn batches(&self, max_vertices: usize) -> impl Iterator<Item = IndexBatch<'_>> {
        let run = (max_vertices / 3 * 3).max(3);
        // Only one of the two is ever populated; the other contributes no
        // batches, which is what lets both widths come back as one iterator.
        let (narrow, wide) = match &self.indices {
            Indices::Narrow(i) => (i.as_slice(), [].as_slice()),
            Indices::Wide(i) => ([].as_slice(), i.as_slice()),
        };
        narrow
            .chunks(run)
            .map(IndexBatch::Narrow)
            .chain(wide.chunks(run).map(IndexBatch::Wide))
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
    /// The finest density the startup table holds.
    pub const IN_TABLE: Self = Self(EAGER_LEVEL);

    /// The finest density there is.
    pub const FINEST: Self = Self(FINEST_LEVEL);

    /// The bucket whose triangles land nearest [`TARGET_PX`] on screen.
    ///
    /// `px_per_unit` is how many pixels one figure-space unit covers, which is
    /// the whole of what density depends on.
    ///
    /// A radial animation does not reach this. It scales each point where the
    /// vertices are walked, leaving the layer's extent untouched, so a figure
    /// animated to twice its size is drawn at the density of its unanimated
    /// size. Density follows the knob and not the animation, which is worth
    /// knowing before reading it as a bug.
    ///
    /// `finest` is the densest mesh the caller is willing to have built; a
    /// figure large enough to want more than that is drawn with it instead.
    pub fn for_screen(px_per_unit: f64, finest: Self) -> Self {
        let raw = TARGET_PX / px_per_unit.max(1.0);
        let exp = raw.log2().round();
        // `as i8` on a NaN or an enormous value saturates rather than wrapping,
        // and the clamp then puts it on a real level.
        Level((exp as i8).clamp(finest.0, COARSEST_LEVEL))
    }

    /// Target edge length in figure space.
    pub fn target_edge(self) -> f32 {
        2f32.powi(i32::from(self.0))
    }

    /// The densities built before the show starts, coarsest first.
    pub fn eager() -> impl Iterator<Item = Self> {
        (EAGER_LEVEL..=COARSEST_LEVEL).rev().map(Level)
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
    pub figure: FigureId,
    pub level: Level,
}

/// Meshes built so far, keyed by figure and density.
///
/// Nothing evicts, and nothing animated reaches the key: a mesh built for a
/// figure at a density holds while the colour on it changes every frame.
///
/// **The set is bounded, because both libraries are tables.** Each names a
/// fixed list of figures, so the key runs over those figures times the six
/// densities and no further: 62 baked and 514 generated, 3,456 meshes, 176
/// million triangles, **2.1 GB** if every one of them were ever drawn.
///
/// Bounded is not small, and where the two libraries sit in that number is
/// worth knowing: the baked share is 239 MB of it and the generated share is
/// the other 1.91 GB, because a generated figure carries an order of magnitude
/// more triangles than a piece of artwork does. Most of that is the two finest
/// densities, which only a figure drawn larger than the size knob's default
/// reaches; the four coarser ones come to 168 MB between them.
///
/// **How much the narrow index saves depends on the density, and the coarse
/// end is where it lands.** 2,947 of these meshes are addressed by a `u16`,
/// but the 509 that are not carry most of the triangles at the two finest
/// densities — so the four coarse densities fall by a third while the ceiling
/// moves by an eighth. That is the useful way round: the coarse four are the
/// densities built before the show, and the only ones a client reaches at all
/// until it is told to refine large figures.
///
/// Nothing is evicted, and no cap is wanted, because a figure dropped is a
/// figure tessellated again and the families holding the most triangles are
/// the ones that cost the most to build. A bound on top of the table would
/// trade memory for latency on exactly the figures whose latency is already
/// worst.
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

    /// What the meshes held weigh, which is the number the docstrings above
    /// quote and the one a client reports at startup.
    pub fn bytes(&self) -> usize {
        self.built.values().map(RefinedMesh::bytes).sum()
    }

    /// How many meshes are held, against the table that bounds them.
    pub fn len(&self) -> usize {
        self.built.len()
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

    // Refinement is done in full precision and only the answer is snapped, so
    // an edge cut a dozen times is cut where it should be and not on a grid
    // that compounds. What the dedup then welds is a grid cell rather than a
    // bit pattern: the midpoints two triangles share agree exactly, and so now
    // do the few vertices that merely landed within a step of each other.
    let mut seen: HashMap<StoredPoint, u32> = HashMap::new();
    let mut verts: Vec<StoredPoint> = Vec::new();
    let mut indices = Vec::with_capacity(flat.len());
    for v in flat {
        let v = StoredPoint::of(v);
        let idx = *seen.entry(v).or_insert_with(|| {
            verts.push(v);
            (verts.len() - 1) as u32
        });
        indices.push(idx);
    }
    let indices = Indices::of(indices, verts.len());
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
    use crate::fill::figure::FigureCache;
    use tunnels_model::layer::{GeneratedId, SpriteId};

    #[test]
    fn a_level_buckets_scale_by_powers_of_two() {
        // Densities are powers of two, so scales a factor of two apart land on
        // adjacent levels and scales within a factor of root two land on one.
        let finest = Level::FINEST;
        let coarse = Level::for_screen(100.0, finest);
        assert_eq!(coarse, Level::for_screen(120.0, finest), "same bucket");
        assert!(
            Level::for_screen(400.0, finest) < coarse,
            "a bigger figure gets a finer mesh"
        );
        // Both ends clamp rather than running away.
        assert_eq!(
            Level::for_screen(1e9, finest),
            Level::for_screen(1e12, finest)
        );
        assert_eq!(Level::for_screen(0.0, finest), Level(COARSEST_LEVEL));
        // And a caller that will not have finer meshes built gets the finest
        // it is willing to hold, however large the figure is.
        assert_eq!(Level::for_screen(1e9, Level::IN_TABLE), Level::IN_TABLE);
    }

    /// The grid vertices are stored on reaches every figure either library can
    /// draw, and resolves each of them far past what a screen can show.
    ///
    /// Both halves are worth pinning. Snapping clamps, so a figure that ran
    /// outside the range would be folded onto its edge rather than land on the
    /// far side — and several families are composed to run outside the frame
    /// they are built in, so how far the library reaches is not a thing to
    /// assume. A contour bounds the mesh refined from it, because refinement
    /// only ever adds midpoints of edges it already has.
    #[test]
    fn every_figure_fits_the_grid_it_is_stored_on() {
        let limit = f32::from(i16::MAX) / QUANTISATION;
        // One step, and what a vertex may move by: half a step each way.
        let step = 1.0 / QUANTISATION;
        for p in [
            Point::new(0.0, 0.0),
            Point::new(1.0, -1.0),
            Point::new(0.499_97, -2.431_43),
        ] {
            let back = StoredPoint::of(p).widen();
            assert!(
                (back.x() - p.x()).abs() <= step / 2.0 && (back.y() - p.y()).abs() <= step / 2.0,
                "{p:?} came back as {back:?}, further than half a step of {step}"
            );
        }
        // Past the range it folds onto the edge, and never onto the far side.
        assert_eq!(StoredPoint::of(Point::new(50.0, -50.0)).widen().x(), limit);
        assert_eq!(
            StoredPoint::of(Point::new(50.0, -50.0)).widen().y(),
            -limit - step
        );

        let mut cache = FigureCache::default();
        let baked = (0..tunnels_sprites::count())
            .filter_map(|id| u16::try_from(id).ok())
            .map(|id| FigureId::Baked(SpriteId(id)));
        let generated = GeneratedId::library().map(FigureId::Generated);
        let mut worst = (0.0f32, None);
        for figure in baked.chain(generated) {
            let Some(figures) = cache.get(figure) else {
                continue;
            };
            for p in figures
                .iter()
                .flat_map(|f| f.subpaths.iter())
                .flat_map(|c| c.points())
            {
                let reach = p.x().abs().max(p.y().abs());
                if reach > worst.0 {
                    worst = (reach, Some(figure));
                }
            }
        }
        // Room to spare rather than merely fitting, because the figure that
        // reaches furthest is a field cut out of a tiling and a family added
        // later could cut a wider one.
        assert!(
            worst.0 < limit * 0.75,
            "{:?} reaches {}, too near the edge of a grid that stops at {limit}",
            worst.1,
            worst.0
        );
    }

    /// The index width is a property of one mesh, and nothing downstream can
    /// tell which width a mesh it is walking chose.
    ///
    /// A figure whose mesh outgrows `u16` between one density and the next
    /// would otherwise index the wrong vertices rather than fail, because a
    /// truncated index is a valid index into a shorter list.
    #[test]
    fn the_index_width_follows_the_vertex_count_and_reads_back_the_same() {
        let triangles: [[u32; 3]; 3] = [[0, 1, 2], [2, 1, 3], [65_534, 3, 0]];
        let flat: Vec<u32> = triangles.iter().flatten().copied().collect();
        let mesh = |vertices: usize| RefinedMesh {
            verts: vec![StoredPoint::of(Point::new(0.0, 0.0)); vertices],
            indices: Indices::of(flat.clone(), vertices),
        };
        // The largest index a `u16` carries is 65,535, so a mesh of that many
        // vertices and one more is the last that still fits.
        let narrow = mesh(usize::from(u16::MAX) + 1);
        let wide = mesh(usize::from(u16::MAX) + 2);
        assert!(
            matches!(narrow.indices, Indices::Narrow(_)),
            "a mesh a u16 addresses was stored wide"
        );
        assert!(
            matches!(wide.indices, Indices::Wide(_)),
            "a mesh a u16 cannot address was narrowed, which loses vertices"
        );
        assert!(wide.bytes() > narrow.bytes(), "narrowing saved nothing");

        for held in [&narrow, &wide] {
            assert_eq!(held.triangle_count(), triangles.len());
            // A cap past the whole mesh gives one batch of everything.
            let read: Vec<[u32; 3]> = held
                .batches(usize::MAX)
                .flat_map(IndexBatch::triangles)
                .collect();
            assert_eq!(read, triangles, "the triangles came back changed");
            let ungrouped: Vec<u32> = held
                .batches(usize::MAX)
                .flat_map(IndexBatch::indices)
                .collect();
            assert_eq!(ungrouped, flat, "the indices came back changed");
            // And a cap of one triangle cuts three batches of whole triangles
            // rather than tearing one apart.
            assert_eq!(held.batches(3).count(), 3);
            let cut: Vec<[u32; 3]> = held.batches(3).flat_map(IndexBatch::triangles).collect();
            assert_eq!(cut, triangles, "a batched walk lost a triangle");
        }
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
        let verts: Vec<Point> = mesh.points().collect();
        // A cap past the whole mesh gives one batch, which is every triangle.
        for tri in mesh.batches(usize::MAX).flat_map(IndexBatch::triangles) {
            let corners = tri.map(|i| verts[i as usize]);
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
            verts.len() < mesh.triangle_count() * 3,
            "{} vertices for {} triangles — nothing was shared",
            verts.len(),
            mesh.triangle_count()
        );
    }
}
