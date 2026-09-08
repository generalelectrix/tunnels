//! The parameters of the figures that survived curation.
//!
//! Each preset carries the name it was judged under, so a preset and the verdict
//! on it can be matched up.
//!
//! Names are not unique descriptions of parameters. The library was swept twice:
//! the first sweep named figures by the count they carried, the second by the
//! parameters that produced them, and where the second reached parameters the
//! first had already written, the figure kept the earlier name and gained a
//! second one. These pairs are therefore the same figure under two names, and
//! are aliases rather than a mistake — both are kept so every judged name is
//! accounted for:
//!
//! | early name | later name |
//! |---|---|
//! | `spiro_hypo_{5,7,9,11}` | `spiro_hypo_{10_4_6,14_5_8,9_4_7,11_4_8}` |
//! | `spiro_epi_{5,7,12,9}` | `spiro_epi_{10_4_6,7_3_5,12_5_7,9_2_5}` |
//! | `guilloche_{4,5,6,7}` | `guilloche_{9_4_7_4,14_5_8_5,12_5_7_6,10_4_6_7}` |
//! | `twistring_{5,6,8,12}` | `twistring_{5_13,6_14,8_14,12_12}` |
//! | `phyllo_{90,150,240,400}` | `phyllo_{90_34,150_27,240_22,400_17}` |
//! | `moire_rings_{10,16,22}` | `moire_rings_{10_1,16_2,22_2}` |
//! | `starlattice_{2,3,4}` | `starlattice_{2_70,3_70,4_70}` |
//!
//! Families that were rejected entire are absent — a family earns its place only
//! if it makes an interior a ring of marks cannot.

use crate::families::*;
use std::f64::consts::PI;

/// One curated figure: the name it was judged under and how to build it.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub params: ShapeParams,
}

impl Preset {
    fn new(name: impl Into<String>, params: ShapeParams) -> Self {
        Self {
            name: name.into(),
            params,
        }
    }
}

/// The curated parameters belonging to one family.
pub fn of_family(family: ShapeFamily) -> Vec<ShapeParams> {
    all()
        .into_iter()
        .map(|preset| preset.params)
        .filter(|params| params.family() == family)
        .collect()
}

/// Every curated figure.
pub fn all() -> Vec<Preset> {
    let mut p = Vec::new();
    star_polygons(&mut p);
    roses(&mut p);
    spirographs(&mut p);
    guilloches(&mut p);
    twist_rings(&mut p);
    moires(&mut p);
    phyllotaxes(&mut p);
    star_lattices(&mut p);
    pinwheels(&mut p);
    modular_chords(&mut p);
    string_arts(&mut p);
    harmonographs(&mut p);
    maurer_roses(&mut p);
    truchets(&mut p);
    lissajous_figures(&mut p);
    cycloid_rosettes(&mut p);
    chunky_radial(&mut p);
    p
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
        out.push(Preset::new(
            format!("star_{points}_{step}"),
            ShapeParams::StarPolygon(StarPolygon::new(points, step)),
        ));
    }
}

fn roses(out: &mut Vec<Preset>) {
    const DENSE: &[(u32, u32)] = &[
        (2, 1),
        (3, 2),
        (4, 1),
        (5, 2),
        (5, 3),
        (5, 4),
        (6, 1),
        (7, 2),
        (7, 3),
        (7, 4),
        (7, 5),
        (8, 3),
        (8, 5),
        (9, 2),
        (9, 4),
        (9, 5),
        (11, 4),
        (11, 6),
        (12, 5),
        (13, 6),
    ];
    const FEW_PETALS: &[(u32, u32)] = &[
        (3, 1),
        (5, 1),
        (4, 3),
        (6, 5),
        (7, 6),
        (8, 7),
        (5, 6),
        (7, 8),
    ];
    for (params, width) in [(DENSE, 15.0), (FEW_PETALS, 22.0)] {
        for &(petals, divisor) in params {
            out.push(Preset::new(
                format!("rose_{petals}_{divisor}"),
                ShapeParams::Rose(Rose::new(petals, divisor, width)),
            ));
        }
    }
}

fn spirographs(out: &mut Vec<Preset>) {
    const HYPO: &[(u32, u32, u32)] = &[
        (10, 4, 6),
        (14, 5, 8),
        (9, 4, 7),
        (11, 4, 8),
        (13, 5, 9),
        (16, 7, 10),
        (15, 4, 9),
        (12, 7, 11),
        (17, 6, 12),
        (8, 3, 6),
        (19, 8, 13),
        (21, 8, 14),
    ];
    const EPI: &[(u32, u32, u32)] = &[
        (10, 4, 6),
        (7, 3, 5),
        (12, 5, 7),
        (9, 2, 5),
        (11, 3, 7),
        (13, 4, 8),
        (15, 7, 9),
        (8, 5, 6),
        (14, 3, 9),
        (17, 5, 11),
        (6, 5, 4),
        (16, 9, 10),
    ];
    const HYPO_FEW: &[(u32, u32, u32)] = &[
        (3, 1, 2),
        (4, 1, 2),
        (5, 1, 3),
        (5, 2, 3),
        (6, 1, 3),
        (7, 2, 4),
        (4, 3, 2),
        (7, 3, 3),
        (5, 3, 4),
        (6, 5, 3),
    ];
    const EPI_FEW: &[(u32, u32, u32)] = &[
        (3, 1, 2),
        (4, 1, 2),
        (5, 1, 2),
        (5, 2, 3),
        (6, 1, 3),
        (7, 2, 3),
        (3, 2, 2),
        (5, 4, 3),
        (7, 5, 4),
        (4, 3, 3),
    ];
    // The first sweep named four of each by lobe count alone, before the
    // parameters went into the name.
    const HYPO_EARLY: &[(&str, u32, u32, u32)] = &[
        ("spiro_hypo_5", 10, 4, 6),
        ("spiro_hypo_7", 14, 5, 8),
        ("spiro_hypo_9", 9, 4, 7),
        ("spiro_hypo_11", 11, 4, 8),
    ];
    const EPI_EARLY: &[(&str, u32, u32, u32)] = &[
        ("spiro_epi_5", 10, 4, 6),
        ("spiro_epi_7", 7, 3, 5),
        ("spiro_epi_12", 12, 5, 7),
        ("spiro_epi_9", 9, 2, 5),
    ];

    for (name, fixed, rolling, pen) in HYPO_EARLY.iter().copied() {
        out.push(Preset::new(
            name,
            ShapeParams::Spirograph(Spirograph::new(fixed, rolling, pen, Roll::Inside, 17.0)),
        ));
    }
    for (name, fixed, rolling, pen) in EPI_EARLY.iter().copied() {
        out.push(Preset::new(
            name,
            ShapeParams::Spirograph(Spirograph::new(fixed, rolling, pen, Roll::Outside, 17.0)),
        ));
    }
    for (params, roll, label, width) in [
        (HYPO, Roll::Inside, "hypo", 17.0),
        (EPI, Roll::Outside, "epi", 17.0),
        (HYPO_FEW, Roll::Inside, "hypo", 24.0),
        (EPI_FEW, Roll::Outside, "epi", 24.0),
    ] {
        for &(fixed, rolling, pen) in params {
            out.push(Preset::new(
                format!("spiro_{label}_{fixed}_{rolling}_{pen}"),
                ShapeParams::Spirograph(Spirograph::new(fixed, rolling, pen, roll, width)),
            ));
        }
    }
}

fn guilloches(out: &mut Vec<Preset>) {
    const DENSE: &[(u32, u32, u32, u32)] = &[
        (9, 4, 7, 4),
        (14, 5, 8, 5),
        (12, 5, 7, 6),
        (10, 4, 6, 7),
        (11, 4, 8, 3),
        (13, 5, 9, 4),
        (16, 7, 10, 5),
        (15, 4, 9, 6),
        (8, 3, 6, 8),
        (17, 6, 12, 3),
    ];
    const FEW_COPIES: &[(u32, u32, u32, u32)] = &[
        (5, 2, 3, 2),
        (7, 3, 4, 2),
        (9, 4, 5, 3),
        (5, 2, 3, 3),
        (7, 2, 4, 2),
        (6, 1, 3, 3),
    ];
    const EARLY: &[(&str, u32, u32, u32, u32)] = &[
        ("guilloche_4", 9, 4, 7, 4),
        ("guilloche_5", 14, 5, 8, 5),
        ("guilloche_6", 12, 5, 7, 6),
        ("guilloche_7", 10, 4, 6, 7),
    ];

    let build = |fixed, rolling, pen, copies, width| {
        ShapeParams::Guilloche(Guilloche::new(
            Spirograph::new(fixed, rolling, pen, Roll::Inside, width),
            copies,
        ))
    };
    for (name, fixed, rolling, pen, copies) in EARLY.iter().copied() {
        out.push(Preset::new(name, build(fixed, rolling, pen, copies, 11.0)));
    }
    for (params, width) in [(DENSE, 11.0), (FEW_COPIES, 22.0)] {
        for &(fixed, rolling, pen, copies) in params {
            out.push(Preset::new(
                format!("guilloche_{fixed}_{rolling}_{pen}_{copies}"),
                build(fixed, rolling, pen, copies, width),
            ));
        }
    }
}

fn twist_rings(out: &mut Vec<Preset>) {
    const DENSE: &[(u32, u32, f64)] = &[
        (3, 12, 0.075),
        (4, 13, 0.055),
        (5, 13, 0.045),
        (6, 14, 0.037),
        (7, 14, 0.032),
        (8, 14, 0.028),
        (9, 13, 0.024),
        (10, 12, 0.021),
        (12, 12, 0.018),
        (16, 11, 0.013),
        (5, 20, 0.030),
        (8, 20, 0.019),
        (6, 9, 0.058),
        (12, 18, 0.012),
    ];
    const FEW_RINGS: &[(u32, u32, f64)] = &[
        (3, 3, 0.20),
        (4, 3, 0.16),
        (5, 3, 0.13),
        (6, 3, 0.11),
        (4, 4, 0.14),
        (6, 4, 0.10),
        (3, 5, 0.14),
        (5, 5, 0.10),
        (8, 4, 0.08),
        (7, 3, 0.09),
    ];
    const EARLY: &[(&str, u32, u32, f64)] = &[
        ("twistring_5", 5, 13, 0.045),
        ("twistring_6", 6, 14, 0.037),
        ("twistring_8", 8, 14, 0.028),
        ("twistring_12", 12, 12, 0.018),
    ];
    for (name, sides, rings, step) in EARLY.iter().copied() {
        out.push(Preset::new(
            name,
            ShapeParams::TwistRings(TwistRings::new(sides, rings, step, 11.0)),
        ));
    }
    for (params, width) in [(DENSE, 11.0), (FEW_RINGS, 22.0)] {
        for &(sides, rings, step) in params {
            out.push(Preset::new(
                format!("twistring_{sides}_{rings}"),
                ShapeParams::TwistRings(TwistRings::new(sides, rings, step, width)),
            ));
        }
    }
}

fn moires(out: &mut Vec<Preset>) {
    const GRID: &[(u32, f64)] = &[
        (8, 0.20),
        (10, 0.16),
        (12, 0.13),
        (14, 0.11),
        (16, 0.10),
        (20, 0.09),
        (24, 0.08),
        (28, 0.07),
        (34, 0.06),
        (48, 0.04),
    ];
    const WEAVE: &[(u32, f64)] = &[
        (6, 0.24),
        (8, 0.19),
        (10, 0.15),
        (12, 0.13),
        (16, 0.10),
        (20, 0.08),
        (26, 0.06),
        (32, 0.05),
        (40, 0.04),
    ];
    for (kind, params) in [(MoireKind::Grid, GRID), (MoireKind::Weave, WEAVE)] {
        for &(bars, angle) in params {
            out.push(Preset::new(
                format!("{}_{bars}", kind.name()),
                ShapeParams::MoireBars(MoireBars::new(kind, bars, angle)),
            ));
        }
    }

    const RINGS: &[(u32, u32, f64)] = &[
        (8, 1, 9.0),
        (10, 1, 9.0),
        (13, 1, 9.0),
        (16, 2, 9.0),
        (19, 2, 9.0),
        (22, 2, 9.0),
        (26, 3, 9.0),
        (3, 1, 20.0),
        (4, 1, 20.0),
        (5, 1, 20.0),
        (6, 1, 20.0),
        (5, 2, 20.0),
        (7, 2, 20.0),
    ];
    const RINGS_EARLY: &[(&str, u32, u32)] = &[
        ("moire_rings_10", 10, 1),
        ("moire_rings_16", 16, 2),
        ("moire_rings_22", 22, 2),
    ];
    for (name, rings, offset) in RINGS_EARLY.iter().copied() {
        out.push(Preset::new(
            name,
            ShapeParams::MoireRings(MoireRings::new(rings, offset, 9.0)),
        ));
    }
    for &(rings, offset, width) in RINGS {
        out.push(Preset::new(
            format!("moire_rings_{rings}_{offset}"),
            ShapeParams::MoireRings(MoireRings::new(rings, offset, width)),
        ));
    }
}

fn phyllotaxes(out: &mut Vec<Preset>) {
    const PARAMS: &[(u32, f64)] = &[
        (60, 42.0),
        (90, 34.0),
        (120, 30.0),
        (150, 27.0),
        (200, 24.0),
        (240, 22.0),
        (300, 19.0),
        (400, 17.0),
        (180, 32.0),
        (260, 15.0),
        (24, 66.0),
        (30, 60.0),
        (36, 55.0),
        (45, 50.0),
        (55, 46.0),
        (75, 38.0),
    ];
    const EARLY: &[(&str, u32, f64)] = &[
        ("phyllo_90", 90, 34.0),
        ("phyllo_150", 150, 27.0),
        ("phyllo_240", 240, 22.0),
        ("phyllo_400", 400, 17.0),
    ];
    for (name, florets, dot) in EARLY.iter().copied() {
        out.push(Preset::new(
            name,
            ShapeParams::Phyllotaxis(Phyllotaxis::new(florets, dot)),
        ));
    }
    for &(florets, dot) in PARAMS {
        out.push(Preset::new(
            format!("phyllo_{florets}_{}", dot as u32),
            ShapeParams::Phyllotaxis(Phyllotaxis::new(florets, dot)),
        ));
    }
}

fn star_lattices(out: &mut Vec<Preset>) {
    const PARAMS: &[(u32, f64, f64)] = &[
        (2, 14.0, 0.70),
        (3, 11.0, 0.70),
        (4, 9.0, 0.70),
        (2, 20.0, 0.55),
        (3, 15.0, 0.55),
        (5, 8.0, 0.66),
        (3, 9.0, 0.85),
        (4, 7.0, 0.85),
        (1, 26.0, 0.70),
        (1, 34.0, 0.55),
        (1, 20.0, 0.85),
    ];
    const EARLY: &[(&str, u32, f64)] = &[
        ("starlattice_2", 2, 14.0),
        ("starlattice_3", 3, 11.0),
        ("starlattice_4", 4, 9.0),
    ];
    for (name, tiles, width) in EARLY.iter().copied() {
        out.push(Preset::new(
            name,
            ShapeParams::StarLattice(StarLattice::new(tiles, width, 0.70)),
        ));
    }
    for &(tiles, width, reach) in PARAMS {
        out.push(Preset::new(
            format!("starlattice_{tiles}_{}", (reach * 100.0) as u32),
            ShapeParams::StarLattice(StarLattice::new(tiles, width, reach)),
        ));
    }
}

fn pinwheels(out: &mut Vec<Preset>) {
    type LayerSpec = (u32, f64, f64, f64, f64);
    const NESTS: &[(&str, &[LayerSpec])] = &[
        (
            "3_5_8",
            &[
                (3, 50.0, 200.0, 2.6, 30.0),
                (5, 180.0, 330.0, 2.2, 22.0),
                (8, 310.0, 470.0, 1.8, 16.0),
            ],
        ),
        (
            "5_8_13",
            &[
                (5, 50.0, 210.0, 2.4, 26.0),
                (8, 190.0, 340.0, 2.0, 18.0),
                (13, 320.0, 470.0, 1.6, 12.0),
            ],
        ),
        (
            "4_7_11",
            &[
                (4, 50.0, 200.0, 2.8, 28.0),
                (7, 180.0, 330.0, 2.3, 20.0),
                (11, 310.0, 470.0, 1.7, 14.0),
            ],
        ),
        (
            "3_7",
            &[(3, 60.0, 270.0, 3.0, 34.0), (7, 250.0, 470.0, 2.0, 20.0)],
        ),
        (
            "5_11",
            &[(5, 60.0, 280.0, 2.6, 30.0), (11, 260.0, 470.0, 1.8, 15.0)],
        ),
        (
            "7_12",
            &[(7, 60.0, 280.0, 2.4, 26.0), (12, 260.0, 470.0, 1.6, 14.0)],
        ),
        (
            "2_5_9",
            &[
                (2, 50.0, 200.0, 2.8, 32.0),
                (5, 180.0, 330.0, 2.3, 22.0),
                (9, 310.0, 470.0, 1.7, 15.0),
            ],
        ),
        (
            "3_4_11",
            &[
                (3, 50.0, 200.0, 2.7, 30.0),
                (4, 180.0, 330.0, 2.2, 22.0),
                (11, 310.0, 470.0, 1.6, 14.0),
            ],
        ),
        (
            "4_9_14",
            &[
                (4, 50.0, 210.0, 2.5, 28.0),
                (9, 190.0, 340.0, 2.0, 18.0),
                (14, 320.0, 470.0, 1.5, 12.0),
            ],
        ),
        (
            "5_7_16",
            &[
                (5, 50.0, 210.0, 2.4, 26.0),
                (7, 190.0, 340.0, 1.9, 18.0),
                (16, 320.0, 470.0, 1.4, 11.0),
            ],
        ),
        (
            "6_11_17",
            &[
                (6, 50.0, 210.0, 2.2, 24.0),
                (11, 190.0, 340.0, 1.8, 16.0),
                (17, 320.0, 470.0, 1.3, 10.0),
            ],
        ),
        (
            "3_8_13",
            &[
                (3, 50.0, 200.0, 2.9, 30.0),
                (8, 180.0, 330.0, 2.1, 20.0),
                (13, 310.0, 470.0, 1.6, 13.0),
            ],
        ),
        (
            "2_9",
            &[(2, 60.0, 280.0, 3.1, 36.0), (9, 260.0, 470.0, 1.8, 17.0)],
        ),
        (
            "4_13",
            &[(4, 60.0, 280.0, 2.7, 32.0), (13, 260.0, 470.0, 1.5, 13.0)],
        ),
        (
            "6_17",
            &[(6, 60.0, 280.0, 2.5, 28.0), (17, 260.0, 470.0, 1.3, 10.0)],
        ),
        (
            "5_9_14",
            &[
                (5, 50.0, 210.0, 2.3, 26.0),
                (9, 190.0, 340.0, 1.9, 17.0),
                (14, 320.0, 470.0, 1.4, 11.0),
            ],
        ),
        (
            "2_3",
            &[(2, 60.0, 270.0, 3.0, 44.0), (3, 250.0, 470.0, 2.2, 30.0)],
        ),
        (
            "3_4",
            &[(3, 60.0, 270.0, 2.8, 40.0), (4, 250.0, 470.0, 2.1, 28.0)],
        ),
        (
            "3_5",
            &[(3, 60.0, 270.0, 2.7, 40.0), (5, 250.0, 470.0, 2.0, 26.0)],
        ),
        (
            "2_5",
            &[(2, 60.0, 280.0, 3.1, 46.0), (5, 260.0, 470.0, 1.9, 26.0)],
        ),
        (
            "4_5",
            &[(4, 60.0, 270.0, 2.5, 36.0), (5, 250.0, 470.0, 1.9, 26.0)],
        ),
        (
            "2_3_5",
            &[
                (2, 50.0, 190.0, 3.0, 40.0),
                (3, 170.0, 330.0, 2.4, 30.0),
                (5, 310.0, 470.0, 1.8, 22.0),
            ],
        ),
        (
            "3_4_7",
            &[
                (3, 50.0, 190.0, 2.8, 38.0),
                (4, 170.0, 330.0, 2.2, 28.0),
                (7, 310.0, 470.0, 1.7, 19.0),
            ],
        ),
    ];
    for &(name, layers) in NESTS {
        let layers = layers
            .iter()
            .map(|&(blades, inner, outer, sweep, width)| {
                pinwheel::Layer::new(blades, inner, outer, sweep, width)
            })
            .collect();
        out.push(Preset::new(
            format!("pinwheel_nest_{name}"),
            ShapeParams::PinwheelNest(PinwheelNest::new(layers)),
        ));
    }
}

fn modular_chords(out: &mut Vec<Preset>) {
    const DENSE: &[(u32, u32)] = &[
        (90, 2),
        (120, 2),
        (120, 3),
        (150, 3),
        (160, 5),
        (180, 7),
        (200, 5),
        (144, 13),
        (150, 51),
        (180, 29),
        (210, 11),
        (96, 17),
        (128, 33),
        (220, 79),
    ];
    const FEW_POINTS: &[(u32, u32)] = &[
        (24, 2),
        (30, 2),
        (36, 2),
        (30, 3),
        (40, 3),
        (48, 5),
        (36, 7),
        (60, 2),
        (60, 11),
        (45, 4),
        (28, 3),
        (56, 9),
    ];
    for (params, width) in [(DENSE, 5), (FEW_POINTS, 11)] {
        for &(points, multiplier) in params {
            out.push(Preset::new(
                format!("modmult_{points}_{multiplier}"),
                ShapeParams::ModularChords(ModularChords::new(points, multiplier, width)),
            ));
        }
    }
}

fn string_arts(out: &mut Vec<Preset>) {
    const PARAMS: &[(u32, u32, f64)] = &[
        (4, 22, 8.0),
        (5, 18, 8.0),
        (6, 16, 7.0),
        (8, 13, 7.0),
        (4, 34, 5.0),
        (5, 30, 5.0),
        (7, 20, 6.0),
        (6, 26, 6.0),
        (9, 16, 6.0),
        (10, 14, 6.0),
        (4, 8, 16.0),
        (5, 7, 15.0),
        (6, 6, 14.0),
        (8, 5, 13.0),
        (4, 12, 12.0),
        (5, 10, 12.0),
        (6, 9, 11.0),
        (10, 5, 12.0),
    ];
    for &(sides, per_corner, width) in PARAMS {
        out.push(Preset::new(
            format!("stringart_{sides}_{per_corner}"),
            ShapeParams::StringArt(StringArt::new(sides, per_corner, width)),
        ));
    }
}

fn harmonographs(out: &mut Vec<Preset>) {
    type Spec = (&'static str, (f64, f64), (f64, f64), (f64, f64), [f64; 4]);
    const WOVEN: &[Spec] = &[
        (
            "2_3",
            (2.0, 2.01),
            (3.0, 3.01),
            (0.0, 1.1),
            [0.012, 0.010, 0.011, 0.013],
        ),
        (
            "3_4",
            (3.0, 3.02),
            (4.0, 4.01),
            (0.7, 0.3),
            [0.010, 0.012, 0.009, 0.011],
        ),
        (
            "4_5",
            (4.0, 4.01),
            (5.0, 5.02),
            (1.4, 0.8),
            [0.011, 0.009, 0.012, 0.010],
        ),
        (
            "5_6",
            (5.0, 5.02),
            (6.0, 6.01),
            (0.4, 1.6),
            [0.013, 0.011, 0.010, 0.012],
        ),
        (
            "2_5",
            (2.0, 2.02),
            (5.0, 5.01),
            (1.0, 0.5),
            [0.009, 0.011, 0.012, 0.010],
        ),
        (
            "3_5",
            (3.0, 3.01),
            (5.0, 5.03),
            (0.2, 1.3),
            [0.012, 0.010, 0.011, 0.009],
        ),
        (
            "3_7",
            (3.0, 3.02),
            (7.0, 7.01),
            (1.7, 0.6),
            [0.010, 0.013, 0.009, 0.012],
        ),
        (
            "4_7",
            (4.0, 4.03),
            (7.0, 7.02),
            (0.9, 1.9),
            [0.011, 0.010, 0.013, 0.011],
        ),
    ];
    const BRIEF: &[Spec] = &[
        (
            "1_2s",
            (1.0, 1.01),
            (2.0, 2.01),
            (0.0, 0.9),
            [0.030, 0.026, 0.028, 0.032],
        ),
        (
            "2_3s",
            (2.0, 2.01),
            (3.0, 3.02),
            (0.6, 0.2),
            [0.028, 0.032, 0.026, 0.030],
        ),
        (
            "3_2s",
            (3.0, 3.02),
            (2.0, 2.01),
            (1.2, 0.7),
            [0.032, 0.028, 0.030, 0.026],
        ),
        (
            "1_3s",
            (1.0, 1.02),
            (3.0, 3.01),
            (0.3, 1.4),
            [0.026, 0.030, 0.032, 0.028],
        ),
        (
            "3_4s",
            (3.0, 3.01),
            (4.0, 4.02),
            (0.8, 0.4),
            [0.030, 0.026, 0.028, 0.032],
        ),
    ];
    for (specs, turns, steps, width) in [(WOVEN, 30.0, 3000, 7.0), (BRIEF, 11.0, 1400, 16.0)] {
        for &(name, fx, fy, phase, damp) in specs {
            out.push(Preset::new(
                format!("harmo_{name}"),
                ShapeParams::Harmonograph(Harmonograph {
                    x: harmonograph::Axis {
                        frequencies: fx,
                        damping: (damp[0], damp[1]),
                        phase: phase.0,
                    },
                    y: harmonograph::Axis {
                        frequencies: fy,
                        damping: (damp[2], damp[3]),
                        phase: phase.1,
                    },
                    turns,
                    steps,
                    width,
                }),
            ));
        }
    }
}

fn maurer_roses(out: &mut Vec<Preset>) {
    const PARAMS: &[(u32, u32)] = &[
        (2, 39),
        (3, 47),
        (3, 109),
        (4, 127),
        (5, 97),
        (5, 53),
        (7, 19),
        (7, 113),
        (9, 43),
        (11, 67),
        (13, 53),
        (15, 71),
    ];
    for &(petals, step) in PARAMS {
        out.push(Preset::new(
            format!("maurer_{petals}_{step}"),
            ShapeParams::MaurerRose(MaurerRose::new(petals, step, 5.0)),
        ));
    }
}

fn truchets(out: &mut Vec<Preset>) {
    const PARAMS: &[(u32, f64)] = &[
        (3, 26.0),
        (4, 20.0),
        (5, 17.0),
        (6, 15.0),
        (8, 11.0),
        (10, 9.0),
        (12, 8.0),
        (16, 6.0),
        (2, 36.0),
    ];
    for &(tiles, width) in PARAMS {
        out.push(Preset::new(
            format!("truchet_{tiles}"),
            ShapeParams::Truchet(Truchet::new(tiles, width)),
        ));
    }
}

fn lissajous_figures(out: &mut Vec<Preset>) {
    const DENSE: &[(u32, u32, u32)] = &[
        (3, 2, 2),
        (5, 4, 3),
        (7, 5, 4),
        (9, 8, 2),
        (5, 3, 6),
        (7, 4, 2),
        (8, 5, 3),
        (9, 7, 4),
        (11, 8, 2),
        (11, 9, 2),
        (13, 8, 4),
    ];
    const LOW_RATIO: &[(u32, u32, u32)] = &[
        (2, 1, 3),
        (3, 1, 2),
        (3, 2, 3),
        (4, 3, 3),
        (5, 2, 3),
        (5, 3, 2),
        (4, 1, 3),
        (5, 4, 6),
    ];
    for (params, width) in [(DENSE, 12.0), (LOW_RATIO, 22.0)] {
        for &(a, b, divisor) in params {
            out.push(Preset::new(
                format!("liss_{a}_{b}_{divisor}"),
                ShapeParams::Lissajous(Lissajous::new(a, b, PI / divisor as f64, width)),
            ));
        }
    }
}

fn cycloid_rosettes(out: &mut Vec<Preset>) {
    use CycloidKind::*;
    const DENSE: &[(CycloidKind, u32)] = &[
        (Astroid, 5),
        (Astroid, 7),
        (Astroid, 9),
        (Deltoid, 5),
        (Deltoid, 7),
        (Deltoid, 11),
        (Cardioid, 6),
        (Cardioid, 8),
        (Cardioid, 12),
        (Nephroid, 5),
        (Nephroid, 7),
        (Nephroid, 9),
    ];
    const FEW_COPIES: &[(CycloidKind, u32)] = &[
        (Astroid, 2),
        (Astroid, 3),
        (Deltoid, 2),
        (Deltoid, 3),
        (Cardioid, 2),
        (Cardioid, 3),
        (Cardioid, 4),
        (Nephroid, 2),
        (Nephroid, 3),
        (Nephroid, 4),
    ];
    for (params, width) in [(DENSE, 9.0), (FEW_COPIES, 20.0)] {
        for &(kind, copies) in params {
            out.push(Preset::new(
                format!("cyc_{}_{copies}", kind.name()),
                ShapeParams::CycloidRosette(CycloidRosette::new(kind, copies, width)),
            ));
        }
    }
}

fn chunky_radial(out: &mut Vec<Preset>) {
    out.push(Preset::new(
        "rings_3",
        ShapeParams::ConcentricRings(ConcentricRings::new(vec![
            470.0, 380.0, 290.0, 200.0, 110.0, 40.0,
        ])),
    ));
    out.push(Preset::new(
        "rings_wide",
        ShapeParams::ConcentricRings(ConcentricRings::new(vec![470.0, 300.0, 190.0, 70.0])),
    ));
    out.push(Preset::new(
        "hex_rings",
        ShapeParams::PolygonRings(PolygonRings::new(
            6,
            vec![470.0, 390.0, 310.0, 230.0, 150.0, 70.0],
        )),
    ));
    out.push(Preset::new(
        "oct_rings",
        ShapeParams::PolygonRings(PolygonRings::new(8, vec![470.0, 360.0, 250.0, 140.0])),
    ));
    for (name, petals, ring, petal) in [
        ("mandala_petal_6", 6, 260.0, 200.0),
        ("mandala_petal_8", 8, 260.0, 200.0),
        ("mandala_petal_12", 12, 300.0, 170.0),
    ] {
        out.push(Preset::new(
            name,
            ShapeParams::PetalMandala(PetalMandala::new(petals, ring, petal)),
        ));
    }
    for (name, count, direction) in [
        ("slats_8", 8, bars::Direction::Horizontal),
        ("slats_16", 16, bars::Direction::Horizontal),
        ("slats_v_8", 8, bars::Direction::Vertical),
    ] {
        out.push(Preset::new(
            name,
            ShapeParams::Slats(Slats::new(count, direction, 0.5)),
        ));
    }
    for (name, count) in [("grid_6", 6), ("grid_12", 12)] {
        out.push(Preset::new(name, ShapeParams::Grid(Grid::new(count, 0.35))));
    }
    out.push(Preset::new(
        "frames_4",
        ShapeParams::Frames(Frames::new(4, 45.0)),
    ));
    out.push(Preset::new(
        "frames_6",
        ShapeParams::Frames(Frames::new(6, 30.0)),
    ));
}
