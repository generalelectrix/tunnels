//! Turning a baked figure's contours into triangles.
//!
//! The build ships loops, not triangles, because a figure's winding rule
//! decides which side of a loop fills — on a ring, the difference between a
//! band and a disc. Resolving that is this module's job, and doing it here
//! means the stroke gets the same loops for free.

use super::geom::{Triangle, TriangleList};
use lyon_path::Path;
use lyon_path::math::point;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFillRule, FillTessellator, FillVertex, LineCap,
    LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use std::collections::HashMap;
use tunnels_model::layer::SpriteId;
use tunnels_sprites::{FillRule, Point, Sprite};

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

/// Identifies one tessellated outline.
///
/// Keyed on the thickness the outline was actually stroked at, so the key
/// names the geometry it stands for and nothing else has to be checked.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
struct StrokeId {
    sprite: SpriteId,
    thickness_bits: u32,
}

/// Outlines held before the map is emptied and refilled.
///
/// A stroke mesh measures 7,800 vertices on the average figure and 18,800 on
/// the largest, at sixteen bytes a vertex, so this is a ceiling of about 30 MB
/// typically and 75 MB if every entry were the largest figure in the library.
///
/// Emptying it rather than evicting from it is what the growth law asks for.
/// A thickness animation sweeps the buckets in order and comes back round, so
/// the entry least recently used is also the one about to be wanted again, and
/// any recency policy would evict exactly wrong. Refilling costs one
/// tessellation per outline drawn — 121 µs on the average figure against an
/// 8.3 ms frame — and only the outlines a frame actually draws.
const STROKE_CAP: usize = 256;

/// Figure interiors tessellated so far, before any refinement.
///
/// Bounded by its own key and so never emptied: an interior does not depend on
/// how densely it will be drawn, so a figure has one however the knobs move,
/// and this converges on the library the build ships.
#[derive(Default)]
pub struct FillGeometry(HashMap<SpriteId, TriangleList>);

impl FillGeometry {
    /// The figure's interior, tessellated on first use.
    pub fn get(&mut self, id: SpriteId, sprite: &Sprite) -> &TriangleList {
        self.0.entry(id).or_insert_with(|| {
            let mut out = TriangleList::default();
            for figure in &sprite.figures {
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
                    expand(&buffers, &mut out);
                }
            }
            out
        })
    }
}

/// Outlines tessellated so far, before any refinement.
///
/// Kept apart from the interiors because the work differs — an outline depends
/// on how wide the stroke is — and so does the growth law that follows from
/// that. Thickness is a knob and an animation target, so this key moves while
/// the show runs and the map would grow without limit; it is capped at
/// [`STROKE_CAP`] instead.
#[derive(Default)]
pub struct StrokeGeometry(HashMap<StrokeId, StrokeMesh>);

impl StrokeGeometry {
    /// The figure's outline stroked at `thickness`, tessellated on first use.
    pub fn get(&mut self, id: SpriteId, sprite: &Sprite, thickness: Thickness) -> &StrokeMesh {
        let key = StrokeId {
            sprite: id,
            thickness_bits: thickness.key(),
        };
        if self.0.len() >= STROKE_CAP && !self.0.contains_key(&key) {
            self.0.clear();
        }
        self.0.entry(key).or_insert_with(|| {
            let options = StrokeOptions::tolerance(TOLERANCE)
                .with_line_width(thickness.figure_units.max(1e-4))
                .with_line_join(LineJoin::Round)
                .with_line_cap(LineCap::Round);
            let mut out = StrokeMesh::default();
            for figure in &sprite.figures {
                let mut buffers: VertexBuffers<StrokeVertexPair, u32> = VertexBuffers::new();
                let mut builder =
                    BuffersBuilder::new(&mut buffers, |v: StrokeVertex| StrokeVertexPair {
                        position: Point::from_array(v.position().to_array()),
                        // Where on the contour this vertex was offset
                        // from, which is what colours it.
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
        })
    }
}

/// A stroked vertex: where it is, and where on the contour it came from.
#[derive(Copy, Clone)]
struct StrokeVertexPair {
    position: Point,
    on_path: Point,
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
    /// Take the tessellator's indexed output as whole triangles.
    ///
    /// A triangle naming a vertex that is not there is dropped entire; lyon
    /// emits none, but dropping the odd vertex instead would shift every later
    /// one and scramble the rest of the outline.
    fn extend(&mut self, buffers: &VertexBuffers<StrokeVertexPair, u32>) {
        for tri in buffers.indices.as_chunks::<3>().0 {
            let corners = tri.map(|i| buffers.vertices.get(i as usize).copied());
            if let [Some(a), Some(b), Some(c)] = corners {
                for v in [a, b, c] {
                    self.positions.push(v.position);
                    self.on_path.push(v.on_path);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Every vertex in order, three to a triangle.
    pub fn points(&self) -> &[Point] {
        &self.positions
    }

    /// Each vertex, paired with the contour point that colours it.
    pub fn vertices(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.positions
            .iter()
            .copied()
            .zip(self.on_path.iter().copied())
    }
}

/// The beam's thickness knob, resolved into figure space and quantised so an
/// animated thickness does not re-tessellate the outline on every frame.
///
/// The step is half a pixel on screen rather than a fixed amount of shape
/// space. A change moves each edge by half of it, and the client multisamples,
/// so half a pixel sits under anything an edge can resolve — and measuring it
/// on the screen is what makes the granularity independent of output
/// resolution and of how large the figure is drawn.
///
/// Without this an animated thickness re-strokes every frame, and re-refines
/// the mesh behind it, since the mesh is keyed on the stroke too. It does not
/// make the first sweep of a knob free — every bucket is visited once whatever
/// the step — but a periodic animation warms the set in one cycle and every
/// later cycle is a hit.
#[derive(Copy, Clone, Debug)]
pub struct Thickness {
    pub figure_units: f32,
}

/// How a figure's own units map onto the screen.
#[derive(Copy, Clone, Debug)]
pub struct Scale {
    /// Pixels one figure-space unit covers as the figure is actually drawn.
    pub px_per_unit: f64,
    /// Pixels it covers at the nominal size of the density it is drawn at.
    ///
    /// The bucket size is taken from this rather than from `px_per_unit` so
    /// that it holds still while the size knob moves: a quantum that slid with
    /// the scale would put every frame of a size sweep in its own bucket.
    pub nominal_px_per_unit: f64,
}

/// One bucket of stroke thickness, in pixels on screen.
const QUANTUM_PX: f64 = 0.5;

impl Thickness {
    /// Bucket an on-screen stroke thickness against the density it is drawn at.
    pub fn bucketed(screen_px: f64, scale: Scale) -> Self {
        let figure_units = screen_px / scale.px_per_unit.max(f64::MIN_POSITIVE);
        let quantum =
            (QUANTUM_PX / scale.nominal_px_per_unit.max(f64::MIN_POSITIVE)).max(f64::MIN_POSITIVE);
        let buckets = (figure_units / quantum)
            .round()
            .clamp(0.0, f64::from(u32::MAX));
        Self {
            figure_units: (buckets * quantum) as f32,
        }
    }

    /// The key naming this thickness's geometry.
    pub fn key(self) -> u32 {
        self.figure_units.to_bits()
    }
}

/// One `<path>` element's subpaths as a lyon path, every loop closed.
fn path_of(figure: &tunnels_sprites::Figure) -> Path {
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
fn path_of_capped(figure: &tunnels_sprites::Figure, max: f32) -> Path {
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
fn expand(buffers: &VertexBuffers<Point, u32>, out: &mut TriangleList) {
    for tri in buffers.indices.as_chunks::<3>().0 {
        let corners = tri.map(|i| buffers.vertices.get(i as usize).copied());
        if let [Some(a), Some(b), Some(c)] = corners {
            out.push(Triangle::new(a, b, c));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_thickness_lands_in_half_pixel_buckets_whatever_the_scale() {
        let scale = Scale {
            px_per_unit: 200.0,
            nominal_px_per_unit: 200.0,
        };
        // Thicknesses a tenth of a pixel apart share a bucket; half a pixel apart
        // do not.
        let a = Thickness::bucketed(10.0, scale);
        assert_eq!(a.key(), Thickness::bucketed(10.1, scale).key());
        assert_ne!(a.key(), Thickness::bucketed(10.6, scale).key());
        // The thickness used is within half a bucket of the one asked for.
        let on_screen = f64::from(a.figure_units) * scale.px_per_unit;
        assert!((on_screen - 10.0).abs() <= 0.25, "stroked at {on_screen}px");

        // A figure drawn twice as large, at the same density level, gets half
        // the thickness in figure units — the same thickness on screen.
        let large = Thickness::bucketed(
            10.0,
            Scale {
                px_per_unit: 400.0,
                ..scale
            },
        );
        assert!(
            (f64::from(large.figure_units) * 400.0 - 10.0).abs() <= 0.25,
            "stroked at {}px",
            f64::from(large.figure_units) * 400.0
        );

        // The bucket holds still as the size knob moves within a level, which
        // is the whole point: the quantum comes from the level, not the scale.
        assert_eq!(
            Thickness::bucketed(10.0, scale).key(),
            Thickness::bucketed(
                10.0,
                Scale {
                    px_per_unit: 200.4,
                    ..scale
                }
            )
            .key()
        );
    }

    /// Thickness is animated, so its bucket moves every frame and the map has
    /// to have a ceiling. A sweep of the knob is what puts it there.
    #[test]
    fn a_swept_thickness_does_not_grow_the_outline_map_without_limit() {
        use tunnels_sprites::{Contour, Figure};

        let sprite = Sprite {
            name: "square",
            figures: vec![Figure {
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
            }],
        };
        let scale = Scale {
            px_per_unit: 200.0,
            nominal_px_per_unit: 200.0,
        };

        let mut outlines = StrokeGeometry::default();
        for step in 0..4 * STROKE_CAP {
            let thickness = Thickness::bucketed(step as f64 * 0.5, scale);
            outlines.get(SpriteId(0), &sprite, thickness);
            assert!(
                outlines.0.len() <= STROKE_CAP,
                "{} outlines held after {step} distinct thicknesses",
                outlines.0.len()
            );
        }
    }

    /// A ring is two loops, and the rule between them is what makes it a ring.
    #[test]
    fn the_winding_rule_decides_whether_a_ring_has_a_hole() {
        use tunnels_sprites::{Contour, Figure};

        let square = |half: f32| {
            Contour::new(vec![
                Point::new(-half, -half),
                Point::new(half, -half),
                Point::new(half, half),
                Point::new(-half, half),
            ])
            .expect("four corners is a loop")
        };
        let ring = |rule| Sprite {
            name: "ring",
            figures: vec![Figure {
                rule,
                subpaths: vec![square(1.0), square(0.5)],
            }],
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
        let hollow = area(interiors.get(SpriteId(0), &ring(FillRule::EvenOdd)));
        let solid = area(interiors.get(SpriteId(1), &ring(FillRule::NonZero)));

        // The outer square is 4 units of area, the inner 1.
        assert!((solid - 4.0).abs() < 0.01, "nonzero filled {solid}, not 4");
        assert!(
            (hollow - 3.0).abs() < 0.01,
            "even-odd filled {hollow}, not 3"
        );
    }
}
