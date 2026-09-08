//! The figure library, baked into the build.
//!
//! A render client is pushed to a machine during a show and reads no files, so
//! every figure it can draw is compiled into it. `build.rs` does the SVG
//! parsing; what survives into the binary is a blob of flattened contours and
//! a table of names, which this module hands back as borrowed geometry.
//!
//! Contours rather than triangles, because the winding rule is load-bearing:
//! most of these figures are rings, and a ring is an outer loop and an inner
//! loop that an even-odd fill subtracts from each other. Whoever fills them
//! decides that; this crate only says where the loops are.

pub mod geom;

pub use geom::{Contour, Point};

use std::sync::LazyLock;

/// One shelf of the library: figures that read as variations on each other.
///
/// A family is a contiguous run of ids, so a control that picks a family and
/// then picks within it needs only where the run starts and how long it is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SpriteFamily {
    pub name: &'static str,
    /// Id of the family's first figure.
    pub first: u16,
    pub len: u16,
}

impl SpriteFamily {
    /// The id at this position in the family, clamped to its last figure.
    pub fn member(&self, index: u16) -> u16 {
        self.first + index.min(self.len.saturating_sub(1))
    }
}

/// Where a figure sits in the library.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Index into the family table.
    pub family: u16,
    /// How far into that family the figure sits.
    pub index: u16,
}

include!(concat!(env!("OUT_DIR"), "/sprite_names.rs"));

/// The contour blob, written by `build.rs` in the same order as the names.
const BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprites.bin"));

/// How the parity of a point inside a figure's loops decides whether it fills.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FillRule {
    NonZero,
    /// Overlap cancels: the band between a ring's two loops fills and the disc
    /// inside the inner loop does not.
    EvenOdd,
}

/// One `<path>` element's closed subpaths, which its fill rule applies across.
///
/// The rule spans the whole figure rather than each loop, which is what lets
/// separate loops cancel where they overlap.
#[derive(Debug)]
pub struct Figure {
    pub rule: FillRule,
    pub subpaths: Vec<Contour>,
}

/// A drawable figure, normalised into a unit box centred on the origin.
///
/// Normalising is what makes a beam's size knob mean the same thing whatever
/// coordinate system the source artwork used.
#[derive(Debug)]
pub struct Sprite {
    pub name: &'static str,
    pub figures: Vec<Figure>,
}

static SPRITES: LazyLock<Vec<Sprite>> = LazyLock::new(decode);

/// The figure with this id, or `None` past the end of the library.
pub fn sprite(id: u16) -> Option<&'static Sprite> {
    SPRITES.get(usize::from(id))
}

/// How many figures the build carries.
pub fn count() -> usize {
    SPRITES.len()
}

/// Every figure, in id order.
pub fn all() -> &'static [Sprite] {
    &SPRITES
}

/// Every family, in the order a family control offers them.
pub fn families() -> &'static [SpriteFamily] {
    &SPRITE_FAMILIES
}

/// Which family a figure belongs to, and how far into it the figure sits.
pub fn placement(id: u16) -> Option<Placement> {
    SPRITE_FAMILIES
        .iter()
        .position(|f| id >= f.first && id < f.first + f.len)
        .map(|family| Placement {
            family: family as u16,
            index: id - SPRITE_FAMILIES[family].first,
        })
}

/// A cursor over the blob that runs out rather than reading past the end.
///
/// The blob is written by this crate's own build script, so a short read means
/// a bug rather than bad input — but this runs in a client that must not panic
/// once a show has started, so it returns what it has instead.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.rest.split_at_checked(n)?;
        self.rest = tail;
        Some(head)
    }

    fn u32(&mut self) -> Option<usize> {
        let bytes = self.take(4)?;
        Some(u32::from_le_bytes(bytes.try_into().ok()?) as usize)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn point(&mut self) -> Option<Point> {
        let bytes = self.take(8)?;
        Some(Point::new(
            f32::from_le_bytes(bytes[..4].try_into().ok()?),
            f32::from_le_bytes(bytes[4..].try_into().ok()?),
        ))
    }
}

fn decode() -> Vec<Sprite> {
    let mut r = Reader { rest: BLOB };
    let Some(n_sprites) = r.u32() else {
        return Vec::new();
    };
    let mut sprites = Vec::with_capacity(n_sprites);
    for id in 0..n_sprites {
        let Some(name) = SPRITE_NAMES.get(id) else {
            break;
        };
        let Some(figures) = read_figures(&mut r) else {
            break;
        };
        sprites.push(Sprite { name, figures });
    }
    sprites
}

fn read_figures(r: &mut Reader) -> Option<Vec<Figure>> {
    let n_figures = r.u32()?;
    let mut figures = Vec::with_capacity(n_figures);
    for _ in 0..n_figures {
        let rule = match r.u8()? {
            0 => FillRule::NonZero,
            _ => FillRule::EvenOdd,
        };
        let n_subpaths = r.u32()?;
        let mut subpaths = Vec::with_capacity(n_subpaths);
        for _ in 0..n_subpaths {
            let n_points = r.u32()?;
            let mut points = Vec::with_capacity(n_points);
            for _ in 0..n_points {
                points.push(r.point()?);
            }
            // A loop too short to enclose anything is dropped rather than
            // carried; `build.rs` already writes none, so this never fires.
            if let Some(contour) = Contour::new(points) {
                subpaths.push(contour);
            }
        }
        figures.push(Figure { rule, subpaths });
    }
    Some(figures)
}

#[cfg(test)]
mod test {
    use super::*;

    /// The bounding box of every point a figure draws.
    fn bounds(sprite: &Sprite) -> ([f32; 2], [f32; 2]) {
        let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in sprite
            .figures
            .iter()
            .flat_map(|f| f.subpaths.iter().flat_map(|c| c.points()))
        {
            for (axis, v) in p.to_array().into_iter().enumerate() {
                min[axis] = min[axis].min(v);
                max[axis] = max[axis].max(v);
            }
        }
        (min, max)
    }

    /// The centroid of a figure's outline, weighted by arc length.
    ///
    /// The point a figure turns about, when it turns onto itself at all.
    fn outline_centroid(sprite: &Sprite) -> [f64; 2] {
        let (mut moment, mut total) = ([0.0f64; 2], 0.0f64);
        for subpath in sprite.figures.iter().flat_map(|f| &f.subpaths) {
            let pts = subpath.points();
            for i in 0..pts.len() {
                let p = pts[i].to_array().map(f64::from);
                let q = pts[(i + 1) % pts.len()].to_array().map(f64::from);
                let len = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
                for axis in 0..2 {
                    moment[axis] += (p[axis] + q[axis]) / 2.0 * len;
                }
                total += len;
            }
        }
        moment.map(|m| m / total.max(1e-12))
    }

    #[test]
    fn every_figure_decodes_into_the_unit_box() {
        assert_eq!(count(), SPRITE_NAMES.len(), "blob and name table disagree");
        assert!(count() > 0, "no figures were baked");

        for (id, sprite) in all().iter().enumerate() {
            assert!(
                sprite.figures.iter().any(|f| !f.subpaths.is_empty()),
                "{} (id {id}) baked to no contour points",
                sprite.name
            );
            let (min, max) = bounds(sprite);
            let reach = min
                .into_iter()
                .chain(max)
                .fold(0.0f32, |a, v| a.max(v.abs()));
            // Normalisation fills the box in the figure's longest direction,
            // so the figure sits inside it and touches it.
            assert!(
                (reach - 1.0).abs() < 1e-3,
                "{} (id {id}) reaches {reach}, not the edge of the box",
                sprite.name
            );
        }
    }

    /// The figures whose bounding box is not centred on the point they turn
    /// about, and which are therefore placed on that point instead.
    ///
    /// Only an odd rotational order can want this. An even one contains the
    /// half turn, which maps the bounding box onto itself and so puts its
    /// centre on the centre of rotation.
    const RECENTRED: [&str; 5] = [
        "pinwheels/five_pointed",
        "stars/five_pointed",
        "blossoms/five_petal",
        "blossoms/six_petal",
        "emblems/biohazard",
    ];

    /// `blossoms/five_petal_open` is not in that list and looks as though it
    /// should be: it has five petals and sits about 0.068 high. Its top petal
    /// is drawn differently from the other four, so it is not five-fold, and
    /// it leaves a fifth of its angular energy unexplained by any rotation —
    /// against 0.083 for the worst figure that is accepted. Admitting it means
    /// admitting `hands/victory` too, so it keeps its wobble.

    #[test]
    fn a_figure_that_turns_onto_itself_sits_on_the_point_it_turns_about() {
        for (id, sprite) in all().iter().enumerate() {
            let (min, max) = bounds(sprite);
            let box_offset = (0..2)
                .map(|axis| (min[axis] + max[axis]).abs() / 2.0)
                .fold(0.0f32, f32::max);
            let centre = outline_centroid(sprite);
            let turn_offset = (centre[0] * centre[0] + centre[1] * centre[1]).sqrt();

            if RECENTRED.contains(&sprite.name) {
                assert!(
                    turn_offset < 0.005,
                    "{} (id {id}) sits {turn_offset} off the point it turns about",
                    sprite.name
                );
                assert!(
                    box_offset > 0.005,
                    "{} (id {id}) needs no recentring: its box is centred to {box_offset}",
                    sprite.name
                );
            } else {
                assert!(
                    box_offset < 1e-3,
                    "{} (id {id}) was moved off its bounding box by {box_offset}",
                    sprite.name
                );
            }
        }
    }

    #[test]
    fn the_families_tile_the_library() {
        let mut next = 0u16;
        for family in families() {
            assert!(family.len > 0, "the {} family is empty", family.name);
            assert_eq!(
                family.first, next,
                "the {} family does not follow the one before it",
                family.name
            );
            next += family.len;
        }
        assert_eq!(usize::from(next), count(), "the families miss figures");

        for id in 0..count() as u16 {
            let placement = placement(id).unwrap_or_else(|| panic!("figure {id} has no family"));
            let family = families()[usize::from(placement.family)];
            assert_eq!(family.member(placement.index), id, "figure {id} round trip");
        }
        assert_eq!(
            placement(count() as u16),
            None,
            "past the end of the library"
        );
    }
}
