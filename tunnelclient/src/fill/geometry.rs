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

use super::geom::{Triangle, TriangleList};
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

/// Outlines tessellated so far.
///
/// An outline is stroked once at [`REFERENCE_WIDTH`] and narrowed per vertex
/// wherever it is drawn, so a figure has exactly one however the thickness
/// knob moves and however many animations taper it. 401 outlines is the whole
/// of what this can come to hold, the same table the interiors come to.
///
/// It is a large table: 47,554 vertices on the average figure and 312,249 on
/// the largest, at sixteen bytes a vertex, so the whole library is 305 MB
/// against the 37 MB its interiors come to. What a stroke costs is decided by
/// how finely the contour is sampled — [`STROKE_SEGMENT`] puts a join every
/// 0.025 units along it — and not by the width, which is why the outlines
/// outweigh the interiors they follow.
#[derive(Default)]
pub struct StrokeGeometry(HashMap<FigureId, StrokeMesh>);

impl StrokeGeometry {
    /// How many figures have been stroked.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The figure's outline at [`REFERENCE_WIDTH`], tessellated on first use.
    pub fn get(&mut self, id: FigureId, figures: &[Figure]) -> &StrokeMesh {
        self.0
            .entry(id)
            .or_insert_with(|| stroke(figures, REFERENCE_WIDTH))
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
fn stroke(figures: &[Figure], width: f32) -> StrokeMesh {
    let options = StrokeOptions::tolerance(TOLERANCE)
        .with_line_width(width)
        .with_line_join(LineJoin::Round)
        .with_line_cap(LineCap::Round);
    let mut out = StrokeMesh::default();
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
            out.extend(&buffers);
        }
    }
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

/// A stroked outline, as a flat triangle list carrying its contour points.
///
/// A ribbon takes its colour from where it sits **on the contour**, not from
/// where each offset vertex happens to land. That is what a stroke is: one
/// segment at one place on the figure, so its colour is constant across its
/// width by definition.
///
/// It is also why a stroke needs no refinement. Refinement exists so that
/// phase varies little enough across a triangle for the ramp lookup to
/// interpolate it; across a ribbon's width phase does not vary at all, so
/// there is nothing for a finer mesh to resolve.
#[derive(Default)]
pub struct StrokeMesh {
    positions: Vec<Point>,
    on_path: Vec<Point>,
}

impl StrokeMesh {
    /// Take the tessellator's indexed output, splitting each vertex into the
    /// two runs.
    fn extend(&mut self, buffers: &VertexBuffers<StrokeVertexPair, u32>) {
        for vertex in triangles(buffers).flatten() {
            self.positions.push(vertex.position);
            self.on_path.push(vertex.on_path);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
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
            .map(|(&position, &on_path)| StrokeVertexPair { position, on_path })
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
            outlines.0.len(),
            2,
            "two figures came to {} outlines",
            outlines.0.len()
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
