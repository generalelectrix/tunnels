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

/// How finely the tessellator may deviate, in shape units.
///
/// The contours are already flattened to this, so anything finer only refines
/// what is already straight.
const TOLERANCE: f32 = 0.002;

/// Identifies one tessellated outline.
///
/// Keyed on the width the outline was actually stroked at, so the key names
/// the geometry it stands for and nothing else has to be checked.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
struct StrokeId {
    sprite: SpriteId,
    width_bits: u32,
}

/// Triangles tessellated so far, before any refinement.
///
/// Two maps rather than one because the work differs: a figure's interior does
/// not depend on how densely it will be drawn, while its outline depends on
/// how wide the stroke is.
#[derive(Default)]
pub struct GeometryCache {
    fills: HashMap<SpriteId, TriangleList>,
    strokes: HashMap<StrokeId, TriangleList>,
}

impl GeometryCache {
    /// The figure's interior, tessellated on first use.
    pub fn fill(&mut self, id: SpriteId, sprite: &Sprite) -> &TriangleList {
        self.fills.entry(id).or_insert_with(|| {
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

    /// The figure's outline stroked at `width`, tessellated on first use.
    pub fn stroke(&mut self, id: SpriteId, sprite: &Sprite, width: Width) -> &TriangleList {
        self.strokes
            .entry(StrokeId {
                sprite: id,
                width_bits: width.key(),
            })
            .or_insert_with(|| {
                let options = StrokeOptions::tolerance(TOLERANCE)
                    .with_line_width(width.shape_units.max(1e-4))
                    .with_line_join(LineJoin::Round)
                    .with_line_cap(LineCap::Round);
                let mut out = TriangleList::default();
                for figure in &sprite.figures {
                    let mut buffers: VertexBuffers<Point, u32> = VertexBuffers::new();
                    let mut builder = BuffersBuilder::new(&mut buffers, |v: StrokeVertex| {
                        Point::from_array(v.position().to_array())
                    });
                    if StrokeTessellator::new()
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

/// A stroke width, quantised so an animated thickness does not re-tessellate
/// the outline on every frame.
///
/// The step is half a pixel on screen rather than a fixed amount of shape
/// space. A width change moves each edge by half of it, and the client
/// multisamples, so half a pixel of width sits under anything an edge can
/// resolve — and measuring it on the screen is what makes the granularity
/// independent of output resolution and of how large the figure is drawn.
///
/// Without this an animated thickness re-strokes every frame, and re-refines
/// the mesh behind it, since the mesh is keyed on the stroke too. It does not
/// make the first sweep of a knob free — every bucket is visited once whatever
/// the step — but a periodic animation warms the set in one cycle and every
/// later cycle is a hit.
#[derive(Copy, Clone, Debug)]
pub struct Width {
    pub shape_units: f32,
}

/// How a figure's own units map onto the screen.
#[derive(Copy, Clone, Debug)]
pub struct Scale {
    /// Pixels one shape-space unit covers as the figure is actually drawn.
    pub px_per_unit: f64,
    /// Pixels it covers at the nominal size of the density it is drawn at.
    ///
    /// The bucket size is taken from this rather than from `px_per_unit` so
    /// that it holds still while the size knob moves: a quantum that slid with
    /// the scale would put every frame of a size sweep in its own bucket.
    pub nominal_px_per_unit: f64,
}

/// One bucket of stroke width, in pixels on screen.
const QUANTUM_PX: f64 = 0.5;

impl Width {
    /// Bucket an on-screen stroke width against the density it is drawn at.
    pub fn bucketed(screen_px: f64, scale: Scale) -> Self {
        let shape_units = screen_px / scale.px_per_unit.max(f64::MIN_POSITIVE);
        let quantum =
            (QUANTUM_PX / scale.nominal_px_per_unit.max(f64::MIN_POSITIVE)).max(f64::MIN_POSITIVE);
        let buckets = (shape_units / quantum)
            .round()
            .clamp(0.0, f64::from(u32::MAX));
        Self {
            shape_units: (buckets * quantum) as f32,
        }
    }

    /// The key naming this width's geometry.
    pub fn key(self) -> u32 {
        self.shape_units.to_bits()
    }
}

/// One `<path>` element's subpaths as a lyon path, every loop closed.
fn path_of(figure: &tunnels_sprites::Figure) -> Path {
    let mut builder = Path::builder();
    for subpath in &figure.subpaths {
        let Some((first, rest)) = subpath.points().split_first() else {
            continue;
        };
        builder.begin(point(first.x(), first.y()));
        for p in rest {
            builder.line_to(point(p.x(), p.y()));
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
    fn a_width_lands_in_half_pixel_buckets_whatever_the_scale() {
        let scale = Scale {
            px_per_unit: 200.0,
            nominal_px_per_unit: 200.0,
        };
        // Widths a tenth of a pixel apart share a bucket; half a pixel apart
        // do not.
        let a = Width::bucketed(10.0, scale);
        assert_eq!(a.key(), Width::bucketed(10.1, scale).key());
        assert_ne!(a.key(), Width::bucketed(10.6, scale).key());
        // The width used is within half a bucket of the width asked for.
        let on_screen = f64::from(a.shape_units) * scale.px_per_unit;
        assert!((on_screen - 10.0).abs() <= 0.25, "stroked at {on_screen}px");

        // A figure drawn twice as large, at the same density level, gets half
        // the width in shape units — which is the same width on screen.
        let large = Width::bucketed(
            10.0,
            Scale {
                px_per_unit: 400.0,
                ..scale
            },
        );
        assert!(
            (f64::from(large.shape_units) * 400.0 - 10.0).abs() <= 0.25,
            "stroked at {}px",
            f64::from(large.shape_units) * 400.0
        );

        // The bucket holds still as the size knob moves within a level, which
        // is the whole point: the quantum comes from the level, not the scale.
        assert_eq!(
            Width::bucketed(10.0, scale).key(),
            Width::bucketed(
                10.0,
                Scale {
                    px_per_unit: 200.4,
                    ..scale
                }
            )
            .key()
        );
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

        let mut cache = GeometryCache::default();
        let hollow = area(cache.fill(SpriteId(0), &ring(FillRule::EvenOdd)));
        let solid = area(cache.fill(SpriteId(1), &ring(FillRule::NonZero)));

        // The outer square is 4 units of area, the inner 1.
        assert!((solid - 4.0).abs() < 0.01, "nonzero filled {solid}, not 4");
        assert!(
            (hollow - 3.0).abs() < 0.01,
            "even-odd filled {hollow}, not 3"
        );
    }
}
