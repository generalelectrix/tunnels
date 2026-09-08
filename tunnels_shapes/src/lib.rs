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

use std::sync::LazyLock;

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
    pub subpaths: Vec<Vec<[f32; 2]>>,
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

    fn point(&mut self) -> Option<[f32; 2]> {
        let bytes = self.take(8)?;
        Some([
            f32::from_le_bytes(bytes[..4].try_into().ok()?),
            f32::from_le_bytes(bytes[4..].try_into().ok()?),
        ])
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
            subpaths.push(points);
        }
        figures.push(Figure { rule, subpaths });
    }
    Some(figures)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn every_figure_decodes_normalised_into_the_unit_box() {
        assert_eq!(count(), SPRITE_NAMES.len(), "blob and name table disagree");
        assert!(count() > 0, "no figures were baked");

        for (id, sprite) in all().iter().enumerate() {
            let points: Vec<[f32; 2]> = sprite
                .figures
                .iter()
                .flat_map(|f| f.subpaths.iter().flatten().copied())
                .collect();
            assert!(
                !points.is_empty(),
                "{} (id {id}) baked to no contour points",
                sprite.name
            );

            let mut min = [f32::MAX; 2];
            let mut max = [f32::MIN; 2];
            for p in &points {
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
            for axis in 0..2 {
                assert!(
                    min[axis] >= -1.001 && max[axis] <= 1.001,
                    "{} (id {id}) reaches {min:?}..{max:?}, outside the unit box",
                    sprite.name
                );
            }
            // Normalisation scales the longest side to the full box and
            // centres it, so one axis fills [-1, 1] and both are centred.
            let longest = (max[0] - min[0]).max(max[1] - min[1]);
            assert!(
                (longest - 2.0).abs() < 1e-3,
                "{} (id {id}) spans {longest}, not the full box",
                sprite.name
            );
            for axis in 0..2 {
                assert!(
                    (min[axis] + max[axis]).abs() < 1e-3,
                    "{} (id {id}) is off centre on axis {axis}: {min:?}..{max:?}",
                    sprite.name
                );
            }
        }
    }
}
