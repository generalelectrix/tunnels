//! The parameters of the figures that survived curation.
//!
//! Each preset carries the name the curation pass judged it under, so a preset
//! and the verdict on it can be matched up.

use crate::families::{ShapeParams, StarPolygon};

/// One curated figure: the name it was judged under and how to build it.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub params: ShapeParams,
}

/// Every curated figure.
pub fn all() -> Vec<Preset> {
    let mut presets = Vec::new();
    star_polygons(&mut presets);
    presets
}

/// A step of two was judged an open star a ring of marks already cuts, which is
/// why no preset carries one.
fn star_polygons(out: &mut Vec<Preset>) {
    const PARAMS: &[(u32, u32)] = &[
        (7, 3),
        (8, 3),
        (9, 4),
        (10, 3),
        (11, 3),
        (11, 4),
        (11, 5),
        (12, 5),
        (13, 3),
        (13, 4),
        (13, 5),
        (13, 6),
        (14, 3),
        (14, 5),
        (15, 4),
        (16, 5),
        (16, 7),
        (17, 5),
        (17, 7),
        (19, 8),
    ];
    for &(points, step) in PARAMS {
        out.push(Preset {
            name: format!("star_{points}_{step}"),
            params: ShapeParams::StarPolygon(StarPolygon::new(points, step)),
        });
    }
}
