//! Where a figure's contours come from.
//!
//! Two libraries stand behind one figure id: artwork parsed at build time, and
//! families computed from two numbers. One call is all a renderer sees of the
//! difference, because everything past this point — tessellating, refining,
//! colouring, drawing — is the same work whichever built it.

use std::collections::HashMap;
use tunnels_model::layer::{FigureId, GeneratedId};
use tunnels_shapes::geom::CENTER;
use tunnels_sprites::{Contour, Figure, FillRule, Point};

/// The contours behind the figures drawn so far.
///
/// A baked figure's contours come out of the binary. A generated figure's are
/// built the first time it is drawn and kept, because a figure is asked for its
/// contours once for its interior and again for every thickness its outline is
/// stroked at.
#[derive(Default)]
pub struct FigureCache {
    generated: HashMap<GeneratedId, Figure>,
}

impl FigureCache {
    /// This figure's contours, or `None` for a baked figure the build does not
    /// carry.
    ///
    /// Only a baked figure can be missing: a generated one is built rather than
    /// looked up, so there is nothing for it to be absent from.
    pub fn get(&mut self, id: FigureId) -> Option<&[Figure]> {
        match id {
            FigureId::Baked(sprite) => {
                tunnels_sprites::sprite(sprite.0).map(|sprite| sprite.figures.as_slice())
            }
            FigureId::Generated(id) => Some(std::slice::from_ref(
                self.generated.entry(id).or_insert_with(|| build(id)),
            )),
        }
    }
}

/// Build the figure an id names, in the coordinates a renderer draws in.
fn build(id: GeneratedId) -> Figure {
    let figure = id.family.resolve(id.arity, id.secondary).generate();
    Figure {
        rule: match figure.fill_rule {
            tunnels_shapes::FillRule::EvenOdd => FillRule::EvenOdd,
            tunnels_shapes::FillRule::NonZero => FillRule::NonZero,
        },
        subpaths: figure
            .contours
            .iter()
            .filter_map(|contour| Contour::new(contour.points().iter().map(unit).collect()))
            .collect(),
    }
}

/// A point of the frame a figure is generated in, in figure space.
///
/// The centre of that frame goes to the origin and its half-width to one unit.
/// The scale is fixed rather than fitted to the figure, which is the difference
/// between the two libraries: baked artwork arrives in whatever coordinates it
/// was drawn in and is normalised to fill the box, while a generated figure is
/// composed in this frame to begin with, and several families are composed to
/// reach past it. Fitting one of those to what it happens to reach is the one
/// operation that turns a field back into an object. What a figure overruns by,
/// the viewport ends — which is what the viewBox these were drawn in used to do.
fn unit(p: &tunnels_shapes::Point) -> Point {
    Point::new(
        ((p.x - CENTER) / CENTER) as f32,
        ((p.y - CENTER) / CENTER) as f32,
    )
}

#[cfg(test)]
mod test {
    use super::*;
    use tunnels_model::layer::SpriteId;
    use tunnels_shapes::{Arity, Secondary, ShapeFamily};

    fn id(family: ShapeFamily, arity: u32, secondary: f64) -> GeneratedId {
        GeneratedId {
            family,
            arity: Arity::new(arity),
            secondary: Secondary::new(secondary),
        }
    }

    /// The map is the same one for every figure, so a family composed to fill
    /// its frame lands on the edge of the box and one composed to overrun it
    /// lands outside — neither is scaled to what it happens to reach.
    #[test]
    fn the_frame_maps_onto_the_box_at_one_fixed_scale() {
        let reach = |id| {
            build(id)
                .subpaths
                .iter()
                .flat_map(|c| c.points())
                .fold(0.0f32, |acc, p| acc.max(p.x().abs()).max(p.y().abs()))
        };

        // Bars are drawn to the edges of the frame, so they land on the edges
        // of the box.
        let bars = reach(id(ShapeFamily::Slats, 8, 0.0));
        assert!(
            (bars - 1.0).abs() < 1e-6,
            "slats reach {bars}, not the edge of the box"
        );

        // A star lattice is a field cut out of a tiling and runs well past.
        let lattice = reach(id(ShapeFamily::StarLattice, 1, 1.0));
        assert!(
            lattice > 2.5,
            "the star lattice reaches only {lattice}; it is a field and should overrun"
        );

        // A rose is composed inside the frame and is not grown to fill the box.
        let rose = reach(id(ShapeFamily::Rose, 5, 0.0));
        assert!(
            (0.9..0.99).contains(&rose),
            "the rose reaches {rose}, which is not where it was composed"
        );
    }

    /// A generated figure is built once and then answered from what was built,
    /// so a fill and a stroke of one figure work from a single set of contours.
    #[test]
    fn a_generated_figure_is_built_once_and_a_baked_one_is_never_built() {
        let mut cache = FigureCache::default();
        let star = FigureId::Generated(id(ShapeFamily::StarPolygon, 7, 0.0));

        let first: Vec<Point> = cache.get(star).expect("a generated figure")[0]
            .subpaths
            .iter()
            .flat_map(|c| c.points())
            .copied()
            .collect();
        assert!(
            !first.is_empty(),
            "the star polygon built no contour points"
        );
        assert_eq!(cache.generated.len(), 1);

        let again: Vec<Point> = cache.get(star).expect("a generated figure")[0]
            .subpaths
            .iter()
            .flat_map(|c| c.points())
            .copied()
            .collect();
        assert_eq!(first, again, "the figure was rebuilt differently");
        assert_eq!(cache.generated.len(), 1, "the figure was built twice");

        // Another position in the same family is another figure.
        cache.get(FigureId::Generated(id(ShapeFamily::StarPolygon, 11, 0.0)));
        assert_eq!(cache.generated.len(), 2);

        // A baked figure is held by the build, not by this.
        assert!(cache.get(FigureId::Baked(SpriteId(0))).is_some());
        assert_eq!(cache.generated.len(), 2);
    }

    #[test]
    fn a_figure_the_build_does_not_carry_has_no_contours() {
        let mut cache = FigureCache::default();
        let past_the_end = u16::try_from(tunnels_sprites::count()).expect("a small library");
        assert!(cache.get(FigureId::Baked(SpriteId(past_the_end))).is_none());
    }
}
