//! Refining a triangle list to a uniform on-screen resolution.
//!
//! Mesh density is chosen from how large a figure is on screen and nothing
//! else. Keeping it independent of colour is what lets a refined mesh be
//! cached across frames while the colour on it changes freely — including
//! waveforms driven by a clock, which change every frame.

use super::Frame;
use super::geom::{IndexBatch, Triangle, TriangleList};
use log::debug;
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

/// Finest density a client refines to unless it is told otherwise.
///
/// This is where a figure at the default size knob lands on a 1080-line
/// projector, so it covers the common case and everything smaller. The two
/// finer ones are reachable — a figure's extent runs to twice the size knob,
/// crossing into the next at about 0.59 against a default of 0.5 — and cost
/// four and sixteen times the memory of this one for a difference visible
/// only on a figure drawn larger than the knob's default.
const DEFAULT_FINEST_LEVEL: i8 = -5;

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
/// A vertex is a pair of `i16`, which is half what a pair of `f32` weighs.
/// This is the whole of the mapping: the pair spans **±4 figure units**, a
/// power of two and so exact both ways, and every figure either library can
/// draw sits inside it — the furthest, a star lattice, reaches 2.4314, so the
/// range is 1.64 times wider than anything drawn on it. Past the range it
/// clamps rather than wrapping, which turns a figure that overran into one
/// folded onto the edge instead of one appearing on the far side. That no
/// figure overruns is a property of the libraries and not of this number, so
/// it is held by a test rather than by the arithmetic.
///
/// **The resolution is four orders of magnitude finer than the mesh it
/// carries.** One step is 1/8192 of a figure unit, which at 1920 lines with
/// the figure filling the frame is 0.23 of a pixel, against triangles refined
/// to fourteen; the whole grid is 128 steps across one triangle edge at the
/// density that a screen reaches.
///
/// **Snapping moves a figure less than a sixteenth of a pixel of translation
/// does**, measured across both libraries at 1024 and at 1920 lines on
/// coverage overlap, on a two-sided Hausdorff distance, and on how many pixels
/// change at all — and a sixteenth of a pixel is a displacement no knob can
/// ask for. It moves the figure's own area by 0.011% across the library and by
/// 1.08% on the worst single figure, so nothing thin is swallowed either.
///
/// **The decode is free where it happens.** The per-vertex pass already
/// touches every vertex every frame to work out polar coordinates and phase,
/// so widening an `i16` there disappears beside the arctangent next to it. The
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
/// largest to 354,089 at the finest, which no `u16` reaches. One width for all
/// of them is either `u32` everywhere or a cap on how finely a figure may be
/// refined.
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

    /// What these indices hold, counting what is allocated rather than what
    /// is used, since the difference is memory either way.
    fn bytes(&self) -> usize {
        match self {
            Self::Narrow(i) => i.capacity() * size_of::<u16>(),
            Self::Wide(i) => i.capacity() * size_of::<u32>(),
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
        self.verts.capacity() * size_of::<StoredPoint>() + self.indices.bytes()
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
    /// The finest density a client refines to by default.
    pub const DEFAULT_FINEST: Self = Self(DEFAULT_FINEST_LEVEL);

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

/// How long a mesh nothing has drawn is kept, in frames.
///
/// A figure goes dark and comes back all the time — a blackout, a mask
/// closing over it, a level taken down and put back — and rebuilding it every
/// time would be work done to save memory that was about to be wanted again.
/// So a mesh outlives a gap of this long and no longer.
///
/// Five seconds at the client's 120 frames a second, which is twenty times
/// the longest the model spends smoothing a geometry change, so nothing the
/// show itself does can drop a mesh still in use. **Being wrong either way is
/// cheap, which is why the number does not have to be exact**: too short costs
/// one rebuild, measured at 2.4 ms on the average figure and 13.9 at the
/// worst against an 8.3 ms frame; too long costs the mesh's bytes until the
/// next sweep.
///
/// **Lengthening this decides how slowly a client comes down from a high-water
/// mark, not how high the mark can be** — [`BYTE_BUDGET`] caps that however
/// long this is. What it does decide is how much a client carries between
/// looks, and carrying is the axis worth keeping light: a budget is permitted,
/// where this is occupied.
const MAX_AGE: u64 = 600;

/// How often the held meshes are looked over, in frames.
///
/// Reaping walks every mesh held, and almost every walk finds nothing to drop,
/// so doing it per frame would put a scan in the draw path to no purpose. Once
/// a second costs a walk of a few tens of entries and blunts [`MAX_AGE`] by at
/// most a second, which a five-second age does not notice.
const REAP_INTERVAL: u64 = 120;

/// Most the held meshes may weigh before the least recently drawn are dropped,
/// in bytes.
///
/// **A backstop, not the working set.** Age is what returns memory in the
/// ordinary case; this only catches growth faster than the reaper runs — an
/// operator sweeping the figure knob builds a mesh per position and would
/// otherwise hold every one of them until the sweep stopped.
///
/// What an honest look needs is 55 MB: a mixer is at most two pages of eight
/// channels, each channel drawing one figure, and a size knob moving across a
/// bucket boundary holds two densities of it — sixteen channels times two
/// densities times the largest single mesh at the default finest density,
/// 1.73 MB. **This sits at more than twice that on purpose**, because it is a
/// ceiling and not an occupancy: a bound that does not fire is permitted
/// rather than spent, and setting it tight buys nothing while costing a storm
/// of rebuilds during a sweep.
///
/// **[`MAX_AGE`] is the other kind of number and does not get the same
/// latitude.** It decides how much of recent history a client is still
/// holding, so it is paid continuously rather than merely permitted, and a
/// console runs one client per video channel — whatever each holds is
/// multiplied by eight. Generous where a number is only a cap, tight where it
/// is what a client actually carries.
///
/// A client told to refine large figures reaches meshes four and sixteen times
/// that size, and can hold a working set this does not cover; it evicts and
/// rebuilds if so, which is the trade that switch already makes elsewhere.
const BYTE_BUDGET: usize = 128 << 20;

/// One mesh, and when a draw last wanted it.
struct Held {
    mesh: RefinedMesh,
    last_used: Frame,
}

/// The meshes drawn recently, keyed by figure and density.
///
/// **Built on demand and dropped when nothing is drawing them.** A mesh costs
/// a few milliseconds to refine and up to megabytes to hold, so the balance
/// runs the opposite way to a tessellated interior: it is worth rebuilding and
/// not worth keeping. What is held tracks what is on screen — order tens of
/// megabytes for a look — rather than what the libraries could produce, which
/// is 2.1 GB at every density and 168 MB at the four coarsest.
///
/// Nothing animated reaches the key, so a mesh built for a figure at a density
/// holds while the colour on it changes every frame.
///
/// Two things bound it, and they have different jobs. [`MAX_AGE`] returns
/// memory nobody is using, which is the common case and the one that matters:
/// without it the process would sit at its high-water mark for the rest of the
/// show, so an operator who swept the figure knob once during setup would have
/// bought every figure they touched for the evening. [`BYTE_BUDGET`] catches
/// growth arriving faster than the reaper runs, and is expected never to fire
/// during a show.
pub struct MeshLibrary {
    built: HashMap<MeshId, Held>,
    /// What `built` weighs, carried along rather than summed, so testing the
    /// budget costs nothing on a path that runs per layer per frame.
    bytes: usize,
    budget: usize,
    reaped: Frame,
}

impl Default for MeshLibrary {
    fn default() -> Self {
        Self {
            built: HashMap::new(),
            bytes: 0,
            budget: BYTE_BUDGET,
            reaped: Frame::default(),
        }
    }
}

impl MeshLibrary {
    /// The mesh for this id, building it on first use, and noting that this
    /// frame wanted it.
    pub fn get(&mut self, id: MeshId, source: &TriangleList, frame: Frame) -> &RefinedMesh {
        if self.bytes > self.budget {
            self.shed_to_budget();
        }
        let Self { built, bytes, .. } = self;
        let held = built.entry(id).or_insert_with(|| {
            let mesh = refine(source, id.level.target_edge());
            *bytes += mesh.bytes();
            Held {
                mesh,
                last_used: frame,
            }
        });
        held.last_used = frame;
        &held.mesh
    }

    /// Drop what nothing has drawn for [`MAX_AGE`], every [`REAP_INTERVAL`]
    /// frames.
    ///
    /// Called once per drawn frame; it decides for itself whether this is one
    /// of the frames that does the work.
    pub fn reap(&mut self, frame: Frame) {
        if frame.since(self.reaped) < REAP_INTERVAL {
            return;
        }
        self.reaped = frame;
        let mut freed = 0;
        self.built.retain(|_, held| {
            let keep = frame.since(held.last_used) < MAX_AGE;
            if !keep {
                freed += held.mesh.bytes();
            }
            keep
        });
        self.bytes -= freed;
        if freed > 0 {
            debug!(
                "Reaped figure meshes; {} held, {:.0} MB.",
                self.len(),
                self.bytes() as f64 / 1e6
            );
        }
    }

    /// Drop the least recently drawn until the budget is met.
    ///
    /// Ordering the whole table is affordable because this runs only when the
    /// budget is already exceeded, which a show is not expected to do at all.
    fn shed_to_budget(&mut self) {
        let mut ages: Vec<(Frame, MeshId)> = self
            .built
            .iter()
            .map(|(id, held)| (held.last_used, *id))
            .collect();
        ages.sort_unstable_by_key(|(last_used, _)| *last_used);
        for (_, id) in ages {
            if self.bytes <= self.budget {
                return;
            }
            if let Some(held) = self.built.remove(&id) {
                self.bytes -= held.mesh.bytes();
            }
        }
    }

    /// Total triangles held, for reporting memory pressure.
    pub fn triangles(&self) -> usize {
        self.built.values().map(|h| h.mesh.triangle_count()).sum()
    }

    /// What the meshes held weigh.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// How many meshes are held.
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
    // A vertex list grows by doubling and how many a figure refines to is not
    // known until it has, so the last doubling leaves 45% of the library's
    // vertices allocated and unused — 23 MB across the densities built before
    // the show. These are kept for the run, so the slack is worth a walk of
    // the list to give back.
    verts.shrink_to_fit();
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
        assert_eq!(
            Level::for_screen(1e9, Level::DEFAULT_FINEST),
            Level::DEFAULT_FINEST
        );
    }

    /// What is held tracks what is being drawn: a mesh survives a gap and not
    /// an absence, and the budget catches growth arriving faster than the
    /// reaper runs.
    ///
    /// Without the age, the process would sit at its high-water mark for the
    /// rest of the show — one sweep of the figure knob during setup would buy
    /// every figure it passed for the evening.
    #[test]
    fn a_mesh_outlives_a_gap_in_drawing_but_not_an_absence() {
        let mut source = TriangleList::default();
        source.push(Triangle::new(
            Point::new(-1.0, -1.0),
            Point::new(1.0, -1.0),
            Point::new(0.0, 1.0),
        ));
        let id = |n: u16| MeshId {
            figure: FigureId::Baked(SpriteId(n)),
            level: Level::DEFAULT_FINEST,
        };

        let mut library = MeshLibrary::default();
        library.get(id(0), &source, Frame(0));
        assert_eq!(library.len(), 1);
        assert!(library.bytes() > 0, "a mesh was held that weighs nothing");

        // Drawn at every reap, so the gap never reaches the age however long
        // the show runs.
        for frame in (0..MAX_AGE * 4).step_by(REAP_INTERVAL as usize) {
            library.get(id(0), &source, Frame(frame));
            library.reap(Frame(frame));
        }
        assert_eq!(library.len(), 1, "a mesh drawn at every reap was dropped");

        // Reaping is periodic, so a mesh that ages out between two sweeps
        // survives until the next one rather than going the instant it is old.
        let last_drawn = MAX_AGE * 4;
        library.get(id(0), &source, Frame(last_drawn));
        library.reap(Frame(last_drawn + MAX_AGE - 1));
        assert_eq!(library.len(), 1, "dropped a mesh a frame before its age");
        library.reap(Frame(last_drawn + MAX_AGE + 1));
        assert_eq!(
            library.len(),
            1,
            "swept again inside the interval instead of waiting for it"
        );
        library.reap(Frame(last_drawn + MAX_AGE + REAP_INTERVAL));
        assert_eq!(library.len(), 0, "a mesh nothing drew was kept");
        assert_eq!(
            library.bytes(),
            0,
            "dropping the last mesh did not return its bytes"
        );

        // Budgeted at one mesh, so the third figure has to shed — and what
        // goes is the one drawn least recently, not whichever comes to hand.
        let one_mesh = {
            let mut scratch = MeshLibrary::default();
            scratch.get(id(0), &source, Frame(0));
            scratch.bytes()
        };
        let mut library = MeshLibrary {
            budget: one_mesh,
            ..Default::default()
        };
        library.get(id(0), &source, Frame(0));
        library.get(id(1), &source, Frame(1));
        library.get(id(2), &source, Frame(2));
        assert_eq!(library.len(), 2, "the budget shed the wrong number");
        assert!(
            !library.built.contains_key(&id(0)),
            "the budget kept the mesh drawn longest ago"
        );
        assert!(
            library.built.contains_key(&id(1)) && library.built.contains_key(&id(2)),
            "the budget shed a mesh drawn more recently than one it kept"
        );
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
