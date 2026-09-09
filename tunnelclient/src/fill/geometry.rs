//! Turning a figure's contours into triangles.
//!
//! A library ships loops, not triangles, because a figure's winding rule
//! decides which side of a loop fills — on a ring, the difference between a
//! band and a disc. Resolving that is this module's job, and doing it here
//! means the stroke gets the same loops for free.
//!
//! What a figure costs here is decided by how much of it crosses itself and
//! not by how many points it carries, because a crossing is a vertex that has
//! to be found and finding them is the work. Most figures are a few
//! milliseconds whether they were baked or built from a family, so a figure
//! computed while the show runs is not on its own the expensive case.
//!
//! Dense chords under an even-odd rule are the expensive case. A modular chord
//! figure rises smoothly with its chord count, to 42 ms at the top of its
//! range. A Maurer rose is worst at the *bottom* of its range, at 155 ms for
//! two petals, where a large step drives 360 chords through a small figure —
//! the end of a range nobody checks, because cost is looked for at the top.

use super::geom::{Indices, StoredPoint, Triangle, TriangleList};
use lyon_path::Path;
use lyon_path::math::point;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFillRule, FillTessellator, FillVertex, LineCap,
    LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use std::collections::HashMap;
use tunnels_model::layer::FigureId;
use tunnels_sprites::{Figure, FillRule, Point};

/// How finely the tessellator may deviate, in figure units.
///
/// The contours are already flattened to this, so anything finer only refines
/// what is already straight.
const TOLERANCE: f32 = 0.002;

/// Longest contour segment an outline is stroked from, in figure units.
///
/// A stroke's colour comes from the contour, and the ramp coordinate is
/// interpolated between one vertex and the next — so how finely the *contour*
/// is sampled is what decides whether the colour along a stroke is smooth.
/// Flattening only samples curves; a straight edge is one segment however long
/// it is, and a figure crossed by a single straight line would otherwise take
/// one colour along its whole length.
///
/// Splitting the contour is where that is fixed, rather than by meshing the
/// ribbon: the ribbon's colour does not vary across its width, so refining it
/// adds triangles that all resolve to the same answer. Measured at the
/// spread knob's maximum, on the two figures with the longest straight edges,
/// 0.05 already renders indistinguishably from a refined mesh; this is half
/// of that.
const STROKE_SEGMENT: f32 = 0.025;

/// The width every outline is tessellated at, in figure units.
///
/// An outline is stroked once, this wide, and reaches whatever width it is
/// actually drawn at by contracting each vertex toward the contour point it
/// was offset from. So this is a ceiling and not a nominal size: a stroke
/// wider than this cannot be reached, and one narrower costs nothing.
///
/// Set at the generous end of what the knobs can ask for, because the choice
/// is nearly free: what a stroke costs is set by how finely the contour is
/// sampled and not by how wide it is, so the whole library measures 281 MB
/// stroked at 0.02 and 305 MB stroked at 1.0 — an 8% spread across a fiftyfold
/// range of widths. The only thing a narrow reference would buy is a clamp,
/// where a beam asks to be wider than its outline was cut.
pub const REFERENCE_WIDTH: f32 = 1.0;

/// Figure interiors tessellated so far, before any refinement.
///
/// Never emptied, and never needs to be: an interior does not depend on how
/// densely it will be drawn, so a figure has exactly one however the knobs
/// move, and the knobs reach a table. 401 interiors is the whole of it — every
/// figure the build ships and every figure the generated library names.
#[derive(Default)]
pub struct FillGeometry(HashMap<FigureId, TriangleList>);

impl FillGeometry {
    /// How many interiors are held.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.0.len()
    }

    /// What the interiors held weigh.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.0.values().map(|tris| size_of_val(tris.points())).sum()
    }

    /// The figure's interior, tessellated on first use.
    pub fn get(&mut self, id: FigureId, figures: &[Figure]) -> &TriangleList {
        self.0.entry(id).or_insert_with(|| {
            let mut out = TriangleList::default();
            for figure in figures {
                let rule = match figure.rule {
                    FillRule::NonZero => LyonFillRule::NonZero,
                    FillRule::EvenOdd => LyonFillRule::EvenOdd,
                };
                let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(rule);
                let mut buffers: VertexBuffers<Point, u32> = VertexBuffers::new();
                let mut builder = BuffersBuilder::new(&mut buffers, |v: FillVertex| {
                    Point::from_array(v.position().to_array())
                });
                // A figure that will not tessellate contributes nothing rather
                // than stopping the frame.
                if FillTessellator::new()
                    .tessellate_path(&path_of(figure), &options, &mut builder)
                    .is_ok()
                {
                    for [a, b, c] in triangles(&buffers) {
                        out.push(Triangle::new(a, b, c));
                    }
                }
            }
            out
        })
    }
}

/// The most the outline table may hold, in bytes.
///
/// **Sized in bytes rather than in entries or in vertices, because neither of
/// those is a bound on anything.** A figure strokes to 12,869 distinct
/// vertices on average and to 108,212 at the largest, a spread of eight times,
/// so a count of outlines says almost nothing about what they weigh — and once
/// vertices are shared, a count of them does not either, since how many
/// triangles each one serves varies with the figure. Sixty-four megabytes is
/// 352 figures at the mean of 190 kB, or 31 at the largest, and either way it
/// is sixty-four megabytes.
///
/// The whole library at once is 109.6 MB, against the 29.7 MB its interiors
/// come to. Outlines outweigh the interiors they follow because what a stroke
/// costs is set by how finely the contour is sampled — [`STROKE_SEGMENT`] puts
/// a join every 0.025 units along it — and hardly at all by how wide it is.
const STROKE_BUDGET: usize = 64 << 20;

/// Outlines tessellated so far.
///
/// An outline is stroked once at [`REFERENCE_WIDTH`] and narrowed per vertex
/// wherever it is drawn, so a figure has exactly one however the thickness
/// knob moves and however many animations taper it. That is what took the
/// width out of the key, and with it the thing that used to empty this table:
/// the key now moves when the operator reaches a different figure and not once
/// a frame while an animation walks a knob.
///
/// So what remains to bound is how many distinct figures are drawn at once,
/// which is bounded by what the mixer puts on screen. Emptying it rather than
/// evicting from it costs one tessellation per outline a frame actually draws,
/// and it is not expected to happen during a show at all.
#[derive(Default)]
pub struct StrokeGeometry {
    outlines: HashMap<FigureId, StrokeMesh>,
    /// What the outlines held weigh, against [`STROKE_BUDGET`].
    bytes: usize,
}

impl StrokeGeometry {
    /// How many outlines are held.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.outlines.len()
    }

    /// The figure's outline at [`REFERENCE_WIDTH`], tessellated on first use.
    pub fn get(&mut self, id: FigureId, figures: &[Figure]) -> &StrokeMesh {
        if !self.outlines.contains_key(&id) {
            if self.bytes >= STROKE_BUDGET {
                self.outlines.clear();
                self.bytes = 0;
            }
            let mesh = StrokeMesh::of(stroke(figures, REFERENCE_WIDTH));
            self.bytes += mesh.bytes();
            self.outlines.insert(id, mesh);
        }
        &self.outlines[&id]
    }
}

/// Stroke a figure's contours into a ribbon of triangles `width` across.
///
/// **A round join is what lets the result stand in for every narrower stroke,
/// and changing the join style would break that silently.** Lyon puts a
/// stroked vertex at `on_path + n̂ · w/2`, where the direction `n̂` and the
/// multiple of the half-width both come from the path and not from `w` — the
/// outer side of a join rides an arc of radius `w/2`, and the inner side sits
/// where the two offset edges cross, further out as the corner sharpens but
/// further out in proportion. So every offset is linear in the width, and
/// scaling them all by `s` lands on the mesh this would have produced at
/// `s · w`. A miter join clamped by a limit is the counterexample: past the
/// limit its corner stops moving with the width, and a narrowed stroke would
/// keep a corner cut for a wider one.
///
/// The cap is set for completeness and never reached: `path_of_capped` closes
/// every subpath, so an outline is all joins and has no ends to cap.
fn stroke(figures: &[Figure], width: f32) -> TessellatedStroke {
    let options = StrokeOptions::tolerance(TOLERANCE)
        .with_line_width(width)
        .with_line_join(LineJoin::Round)
        .with_line_cap(LineCap::Round);
    let mut out = TessellatedStroke::default();
    let mut flat = Vec::new();
    for figure in figures {
        let mut buffers: VertexBuffers<StrokeVertexPair, u32> = VertexBuffers::new();
        let mut builder = BuffersBuilder::new(&mut buffers, |v: StrokeVertex| StrokeVertexPair {
            position: Point::from_array(v.position().to_array()),
            on_path: Point::from_array(v.position_on_path().to_array()),
        });
        if StrokeTessellator::new()
            .tessellate_path(
                &path_of_capped(figure, STROKE_SEGMENT),
                &options,
                &mut builder,
            )
            .is_ok()
        {
            out.extend(&buffers, &mut flat);
        }
    }
    // Both runs grow by doubling and how many vertices a figure strokes to is
    // not known until it has, so the last doubling leaves slack that is held
    // for as long as the outline is.
    out.positions.shrink_to_fit();
    out.on_path.shrink_to_fit();
    flat.shrink_to_fit();
    out.indices = Indices::of(flat, out.positions.len());
    out
}

/// A stroked vertex: where it is, and where on the contour it came from.
///
/// The pair is what makes the offset recoverable. `position - on_path` is the
/// vector the tessellator pushed this vertex out along, so scaling it and
/// adding it back to `on_path` gives the same vertex at another width — which
/// is how one outline serves every width the knobs ask for.
#[derive(Copy, Clone)]
pub struct StrokeVertexPair {
    pub position: Point,
    /// Where on the contour this vertex was offset from, which is what
    /// colours it and what it narrows toward.
    pub on_path: Point,
}

/// A stroked outline, as shared vertices carrying their contour points and the
/// triangles that index them.
///
/// A ribbon takes its colour from where it sits **on the contour**, not from
/// where each offset vertex happens to land. That is what a stroke is: one
/// segment at one place on the figure, so its colour is constant across its
/// width by definition.
///
/// **Vertices are shared rather than repeated per corner, which decides what a
/// vertex *is* and not only what it costs.** A stroked vertex serves about
/// three triangles, so repeating it triples the store — and it also gives the
/// same point three identities. Everything resolved per vertex then answers
/// three times for one place: three transcendentals where one would do, and,
/// for the one waveform that reads a vertex's index rather than only its
/// position, three different displacements that pull a triangle apart at a
/// corner its neighbours share.
///
/// It is also why a stroke needs no refinement. Refinement exists so that
/// phase varies little enough across a triangle for the ramp lookup to
/// interpolate it; across a ribbon's width phase does not vary at all, so
/// there is nothing for a finer mesh to resolve.
#[derive(Default)]
pub struct StrokeMesh {
    positions: Vec<StoredPoint>,
    on_path: Vec<StoredPoint>,
    indices: Indices,
}

/// A stroked outline as the tessellator produced it, before anything is
/// decided about keeping it.
///
/// **Separate from [`StrokeMesh`] because exactness is a property of the
/// tessellator and precision is a property of the store, and only one of them
/// is worth testing.** What earns the stroke-once design is that lyon's round
/// join puts every offset linear in the width, so scaling one stroke lands
/// where stroking at that width would — a claim about the tessellator, which
/// nothing downstream can weaken. Snapping happens on the way into the table,
/// so that claim stays testable on the numbers lyon actually produced.
#[derive(Default)]
pub struct TessellatedStroke {
    positions: Vec<Point>,
    on_path: Vec<Point>,
    indices: Indices,
}

impl TessellatedStroke {
    /// Take the tessellator's indexed output, splitting each vertex into the
    /// two runs and keeping the indices that address them.
    ///
    /// A figure's contours are stroked one at a time into a single stroke, so
    /// each set of indices is shifted past the vertices already held.
    fn extend(&mut self, buffers: &VertexBuffers<StrokeVertexPair, u32>, flat: &mut Vec<u32>) {
        let base = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        for vertex in &buffers.vertices {
            self.positions.push(vertex.position);
            self.on_path.push(vertex.on_path);
        }
        // A triangle naming a vertex that is not there is dropped entire.
        // Lyon does not emit one, but keeping the odd corner would shift every
        // later index and scramble the rest of the figure.
        let held = buffers.vertices.len();
        for triangle in buffers.indices.as_chunks::<3>().0 {
            if triangle.iter().all(|&i| (i as usize) < held) {
                flat.extend(triangle.iter().map(|&i| i + base));
            }
        }
    }

    /// Each vertex, paired with the contour point it was offset from, at the
    /// precision the tessellator produced.
    #[cfg(test)]
    pub fn vertices(&self) -> impl Iterator<Item = StrokeVertexPair> + '_ {
        self.positions
            .iter()
            .zip(self.on_path.iter())
            .map(|(&position, &on_path)| StrokeVertexPair { position, on_path })
    }
}

impl StrokeMesh {
    /// Keep a tessellated stroke, snapped to the grid stored vertices sit on.
    fn of(stroke: TessellatedStroke) -> Self {
        let mut snapped = Self {
            positions: stroke
                .positions
                .iter()
                .copied()
                .map(StoredPoint::of)
                .collect(),
            on_path: stroke
                .on_path
                .iter()
                .copied()
                .map(StoredPoint::of)
                .collect(),
            indices: stroke.indices,
        };
        snapped.positions.shrink_to_fit();
        snapped.on_path.shrink_to_fit();
        snapped
    }

    pub fn is_empty(&self) -> bool {
        self.indices.len() == 0
    }

    /// The triangles, as indices into the vertices a pass has just walked.
    pub fn indices(&self) -> &Indices {
        &self.indices
    }

    /// What this outline weighs, counting what is allocated rather than what
    /// is used, since the difference is memory either way.
    fn bytes(&self) -> usize {
        (self.positions.capacity() + self.on_path.capacity()) * size_of::<StoredPoint>()
            + self.indices.bytes()
    }

    /// Each vertex, paired with the contour point it was offset from.
    ///
    /// Stored as parallel runs because that is how they are written, and
    /// paired back up here because the two points mean opposite things and a
    /// caller that takes them the wrong way round silently colours a ribbon by
    /// where it landed and narrows it toward the wrong place.
    pub fn vertices(&self) -> impl Iterator<Item = StrokeVertexPair> + '_ {
        self.positions
            .iter()
            .zip(self.on_path.iter())
            .map(|(&position, &on_path)| StrokeVertexPair {
                position: position.widen(),
                on_path: on_path.widen(),
            })
    }
}

/// One figure's subpaths as a lyon path, every loop closed.
fn path_of(figure: &Figure) -> Path {
    path_of_capped(figure, f32::MAX)
}

/// As `path_of`, with no segment longer than `max`.
///
/// A figure's own points are handed over unaltered, and only the points
/// invented between them are computed. `prev + (p - prev) * t` at `t == 1` is
/// an identity in algebra and not in floating point, so computing the endpoint
/// too would move it by an ulp — and a point moved by an ulp is a point that no
/// longer coincides with the others meeting it. Where several subpaths return
/// to one shared vertex, that is the difference between a winding rule seeing
/// one point and seeing a cluster of nearly-identical ones.
fn path_of_capped(figure: &Figure, max: f32) -> Path {
    let mut builder = Path::builder();
    for subpath in &figure.subpaths {
        let Some((first, rest)) = subpath.points().split_first() else {
            continue;
        };
        builder.begin(point(first.x(), first.y()));
        let mut prev = *first;
        for p in rest.iter().chain(std::iter::once(first)) {
            let steps = (prev.distance(*p) / max).ceil().max(1.0) as u32;
            for i in 1..steps {
                let t = i as f32 / steps as f32;
                builder.line_to(point(
                    prev.x() + (p.x() - prev.x()) * t,
                    prev.y() + (p.y() - prev.y()) * t,
                ));
            }
            builder.line_to(point(p.x(), p.y()));
            prev = *p;
        }
        builder.end(true);
    }
    builder.build()
}

/// Resolve the tessellator's indexed output into whole triangles.
///
/// A triangle naming a vertex that is not there is dropped entire. Lyon does
/// not emit one, but dropping the odd point instead would shift every later
/// vertex by one and scramble the rest of the figure.
fn triangles<V: Copy>(buffers: &VertexBuffers<V, u32>) -> impl Iterator<Item = [V; 3]> + '_ {
    buffers.indices.as_chunks::<3>().0.iter().filter_map(|tri| {
        match tri.map(|i| buffers.vertices.get(i as usize).copied()) {
            [Some(a), Some(b), Some(c)] => Some([a, b, c]),
            _ => None,
        }
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::fill::geom::QUANTISATION;

    /// A square, as the simplest closed contour with corners to join.
    fn square() -> [Figure; 1] {
        use tunnels_sprites::Contour;
        [Figure {
            rule: FillRule::NonZero,
            subpaths: vec![
                Contour::new(vec![
                    Point::new(-1.0, -1.0),
                    Point::new(1.0, -1.0),
                    Point::new(1.0, 1.0),
                    Point::new(-1.0, 1.0),
                ])
                .expect("four corners is a loop"),
            ],
        }]
    }

    /// What a stroked vertex is offset by is proportional to the width.
    ///
    /// This is the whole of why one outline serves every width: scale the
    /// offsets and you land on the mesh the tessellator would have produced at
    /// the scaled width, so a narrowed stroke is a real stroke and not an
    /// approximation of one. The offsets are not all the half-width — the
    /// inside of a corner reaches further, and further as the corner sharpens
    /// — which is why proportionality is what is checked and not magnitude.
    ///
    /// A join style whose corner stops moving with the width, such as a miter
    /// under its limit, is what this is here to catch.
    /// A square whose corners do not land on the grid, rotated and offset.
    ///
    /// **A square at ±1 is the shape that hides a snap**: stroked at a
    /// power-of-two width it puts its furthest vertices — the corners, where
    /// the offset is largest and where a proportionality check looks — on
    /// exact multiples of the step, so a statistic taken from the extremes
    /// reads unchanged however coarse the grid is. The vertices along a join's
    /// arc still move, so it does not hide everything; it hides exactly the
    /// measurement most likely to be reached for.
    fn skew_square() -> [Figure; 1] {
        use tunnels_sprites::Contour;
        let (sin, cos) = (0.3f32).sin_cos();
        let points: Vec<Point> = [
            (0.0f32, 0.9137f32),
            (0.9137, 0.0),
            (0.0, -0.9137),
            (-0.9137, 0.0),
        ]
        .iter()
        .map(|&(x, y)| Point::new(x * sin - y * cos + 0.0413, x * cos + y * sin - 0.0271))
        .collect();
        [Figure {
            rule: FillRule::NonZero,
            subpaths: vec![Contour::new(points).expect("four corners is a loop")],
        }]
    }

    /// What keeping a stroke on the grid costs the offset it is narrowed by.
    ///
    /// A stroke is stored once at the reference width and reaches every
    /// narrower one by scaling that offset, so an error here is an error in
    /// the drawn width at every thickness alike rather than one that shrinks
    /// with the beam. Both endpoints snap, each by up to half a step in each
    /// axis, so the offset between them moves by at most a step in each axis —
    /// and that bound is what is asserted, rather than a tolerance chosen to
    /// pass.
    #[test]
    fn snapping_a_stroke_moves_its_offset_by_less_than_a_step() {
        let offsets = |v: StrokeVertexPair| {
            (
                v.position.x() - v.on_path.x(),
                v.position.y() - v.on_path.y(),
            )
        };
        let exact: Vec<(f32, f32)> = stroke(&skew_square(), REFERENCE_WIDTH)
            .vertices()
            .map(offsets)
            .collect();
        let stored: Vec<(f32, f32)> = StrokeMesh::of(stroke(&skew_square(), REFERENCE_WIDTH))
            .vertices()
            .map(offsets)
            .collect();
        assert_eq!(exact.len(), stored.len(), "storing lost a vertex");
        assert!(!exact.is_empty(), "the fixture stroked to nothing");

        let step = 1.0 / QUANTISATION;
        let mut worst = 0.0f32;
        for (a, b) in exact.iter().zip(&stored) {
            worst = worst.max((a.0 - b.0).abs()).max((a.1 - b.1).abs());
        }
        assert!(
            worst <= step,
            "an offset moved {worst}, further than the {step} a snap of both ends allows"
        );
        // And it does move: a fixture nothing snapped would prove nothing.
        assert!(
            worst > 0.0,
            "nothing moved, so this measured a grid-aligned fixture"
        );
    }

    #[test]
    fn what_a_stroked_vertex_is_offset_by_is_proportional_to_the_width() {
        let widest = |width| {
            stroke(&square(), width)
                .vertices()
                .map(|v| v.position.distance(v.on_path))
                .fold(0.0f32, f32::max)
        };
        let reference = widest(REFERENCE_WIDTH);
        // The inside of the square's right-angle corners, at half the width
        // times root two, is the furthest any vertex reaches.
        let corner = REFERENCE_WIDTH / 2.0 * std::f32::consts::SQRT_2;
        assert!(
            (reference - corner).abs() <= corner * 1e-3,
            "the furthest vertex reaches {reference}, not the {corner} a right-angle corner puts it at"
        );
        for divisor in [2.0, 8.0, 64.0] {
            let narrow = widest(REFERENCE_WIDTH / divisor);
            let expected = reference / divisor;
            assert!(
                (narrow - expected).abs() <= expected * 1e-3,
                "a stroke {divisor} times narrower reaches {narrow}, not the {expected} proportion asks for"
            );
        }
    }

    /// Contracted to nothing, every vertex lands on its contour point exactly.
    ///
    /// A beam's waveform reaches zero at its trough, so this is the ordinary
    /// bottom of the range rather than an edge case. Landing exactly there is
    /// what makes the triangles collapse to no area and draw nothing; landing
    /// an ulp away would leave a sliver of a figure that has gone out.
    #[test]
    fn a_vertex_contracted_to_nothing_lands_on_its_contour_point() {
        use tunnels_model::layer::SpriteId;

        let mut outlines = StrokeGeometry::default();
        let mesh = outlines.get(FigureId::Baked(SpriteId(0)), &square());
        for v in mesh.vertices() {
            let collapsed = Point::new(
                v.on_path.x() + (v.position.x() - v.on_path.x()) * 0.0,
                v.on_path.y() + (v.position.y() - v.on_path.y()) * 0.0,
            );
            assert_eq!(
                collapsed.bits(),
                v.on_path.bits(),
                "a vertex contracted to nothing landed beside its contour point"
            );
        }
    }

    /// A figure is stroked once however many widths are asked of it.
    ///
    /// The width left the key when it became a per-vertex quantity, so the map
    /// is one entry per figure and has nothing left to bound.
    #[test]
    fn an_outline_is_tessellated_once_per_figure() {
        use tunnels_model::layer::SpriteId;

        let mut outlines = StrokeGeometry::default();
        for _ in 0..64 {
            outlines.get(FigureId::Baked(SpriteId(0)), &square());
        }
        outlines.get(FigureId::Baked(SpriteId(1)), &square());
        assert_eq!(
            outlines.outlines.len(),
            2,
            "two figures came to {} outlines",
            outlines.outlines.len()
        );
    }

    /// A ring is two loops, and the rule between them is what makes it a ring.
    #[test]
    fn the_winding_rule_decides_whether_a_ring_has_a_hole() {
        use tunnels_model::layer::SpriteId;
        use tunnels_sprites::Contour;

        let square = |half: f32| {
            Contour::new(vec![
                Point::new(-half, -half),
                Point::new(half, -half),
                Point::new(half, half),
                Point::new(-half, half),
            ])
            .expect("four corners is a loop")
        };
        let ring = |rule| {
            [Figure {
                rule,
                subpaths: vec![square(1.0), square(0.5)],
            }]
        };

        let area = |tris: &TriangleList| -> f32 {
            tris.triangles()
                .map(|t| {
                    let [a, b, c] = t.points();
                    ((b.x() - a.x()) * (c.y() - a.y()) - (c.x() - a.x()) * (b.y() - a.y())).abs()
                        / 2.0
                })
                .sum()
        };

        let mut interiors = FillGeometry::default();
        let hollow = area(interiors.get(FigureId::Baked(SpriteId(0)), &ring(FillRule::EvenOdd)));
        let solid = area(interiors.get(FigureId::Baked(SpriteId(1)), &ring(FillRule::NonZero)));

        // The outer square is 4 units of area, the inner 1.
        assert!((solid - 4.0).abs() < 0.01, "nonzero filled {solid}, not 4");
        assert!(
            (hollow - 3.0).abs() < 0.01,
            "even-odd filled {hollow}, not 3"
        );
    }
}
