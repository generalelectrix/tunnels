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
    pub fn member(self, index: u16) -> u16 {
        self.first + index.min(self.len.saturating_sub(1))
    }
}

/// Where a figure sits in the library: which shelf, and how far along it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Slot {
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

/// The family at this position in that order, or `None` past the end of it.
pub fn family(index: u16) -> Option<SpriteFamily> {
    SPRITE_FAMILIES.get(usize::from(index)).copied()
}

/// Which family a figure belongs to, and how far into it the figure sits.
pub fn slot(id: u16) -> Option<Slot> {
    SPRITE_FAMILIES
        .iter()
        .position(|f| id >= f.first && id < f.first + f.len)
        .map(|family| Slot {
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
    ///
    /// A count of petals is not a rotational order, and `blossoms/six_petal`
    /// is where the two part company: it reads as six petals and misses both
    /// the sixth turn and the half turn, which is why it is here alongside the
    /// five-fold figures. `blossoms/snowflake` is the six-armed figure that
    /// does carry its turn, and its box is centred.
    ///
    /// `blossoms/five_petal_open` looks as though it belongs here too — five
    /// petals, and an outline centroid well off the origin. Its top petal is
    /// drawn unlike the other four, so it misses the fifth turn by more than
    /// any figure accepted here, and that centroid is not a centre of
    /// rotation. Moving the figure onto it would take a centred box off
    /// centre for nothing.
    const RECENTRED: [&str; 5] = [
        "pinwheels/five_pointed",
        "stars/five_pointed",
        "blossoms/five_petal",
        "blossoms/six_petal",
        "emblems/biohazard",
    ];

    /// The figure of this name, which the library is expected to carry.
    fn named(name: &str) -> &'static Sprite {
        all()
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("the library carries no {name}"))
    }

    /// How far a figure sits from its own image under a turn of `turns`:
    /// the furthest any of its points lands from the nearest point of it.
    ///
    /// Zero for a figure the turn maps exactly onto itself. Flattening a
    /// contour into line segments moves the sample points around the outline,
    /// so a figure that does carry a turn still measures a little above zero.
    fn rotation_residual(sprite: &Sprite, turns: f64) -> f64 {
        let points: Vec<[f64; 2]> = sprite
            .figures
            .iter()
            .flat_map(|f| f.subpaths.iter().flat_map(|c| c.points()))
            .map(|p| p.to_array().map(f64::from))
            .collect();
        let (sin, cos) = (turns * std::f64::consts::TAU).sin_cos();
        points
            .iter()
            .map(|p| {
                let turned = [cos * p[0] - sin * p[1], sin * p[0] + cos * p[1]];
                points
                    .iter()
                    .map(|q| (turned[0] - q[0]).powi(2) + (turned[1] - q[1]).powi(2))
                    .fold(f64::MAX, f64::min)
                    .sqrt()
            })
            .fold(0.0, f64::max)
    }

    /// How far a figure may sit from its own turned image and still be taken
    /// to turn onto itself.
    ///
    /// A tolerance and not a property of the geometry: flattening a contour
    /// into line segments moves the sample points around the outline, so even
    /// a figure a turn maps exactly onto itself measures above zero, by an
    /// amount depending on how finely it was flattened. This sits between the
    /// two populations it has to separate — the figures below it read as
    /// turning onto themselves, and every re-centred figure is above it. Only
    /// one direction of the comparison is worth making: a figure above it may
    /// still be symmetric about some centre that is not the origin.
    const LANDS_BACK: f64 = 0.1;

    /// The half turn pins a figure's bounding box to its centre of rotation,
    /// so a figure that carries the half turn can never want re-centring.
    /// Every re-centred figure therefore has to miss its own half-turn image,
    /// including the one whose name counts an even number of petals.
    #[test]
    fn nothing_placed_on_its_centre_of_rotation_carries_the_half_turn() {
        for name in RECENTRED {
            let missed = rotation_residual(named(name), 0.5);
            assert!(
                missed > LANDS_BACK,
                "{name} sits only {missed} from its own half-turn image"
            );
        }

        // The one whose name says otherwise. `blossoms/snowflake` has six arms
        // and does land back on itself under a sixth turn, which is what makes
        // the blossom's failure a fact about the figure rather than about how
        // finely the library flattens six-fold things.
        assert!(
            rotation_residual(named("blossoms/snowflake"), 1.0 / 6.0) < LANDS_BACK,
            "a six-armed figure that turns onto itself has to read that way"
        );
        let missed = rotation_residual(named("blossoms/six_petal"), 1.0 / 6.0);
        assert!(
            missed > LANDS_BACK,
            "blossoms/six_petal sits only {missed} from its own sixth-turn image, so it is six-fold after all"
        );

        // The near miss that is left where it lies: five petals and an outline
        // centroid off the origin, but the fifth turn lands it back less well
        // than it lands any of the five-fold figures that are accepted.
        let open = named("blossoms/five_petal_open");
        let missed = rotation_residual(open, 0.2);
        for name in [
            "pinwheels/five_pointed",
            "stars/five_pointed",
            "blossoms/five_petal",
        ] {
            let accepted = rotation_residual(named(name), 0.2);
            assert!(
                missed > accepted,
                "blossoms/five_petal_open misses the fifth turn by {missed}, no worse than the accepted {name} at {accepted}"
            );
        }
        let centre = outline_centroid(open);
        assert!(
            centre[0].hypot(centre[1]) > 0.005,
            "blossoms/five_petal_open would not be mistaken for a re-centred figure at all"
        );
    }

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
            let slot = slot(id).unwrap_or_else(|| panic!("figure {id} has no family"));
            let family = families()[usize::from(slot.family)];
            assert_eq!(family.member(slot.index), id, "figure {id} round trip");
        }
        assert_eq!(slot(count() as u16), None, "past the end of the library");
    }
}
