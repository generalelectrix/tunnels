//! Reading the Python generator's output back, and writing the Rust generator's
//! output in the same form, so the two can be set against each other.

use std::fmt::Write as _;
use std::path::PathBuf;
use tunnels_shapes::geom::Figure;

/// One subpath of an SVG `d` attribute, in the two forms the generator emits.
#[derive(Debug, Clone, PartialEq)]
pub enum Subpath {
    /// A closed polyline: `M`, then `L` per point, then `Z`.
    Poly(Vec<(f64, f64)>),
    /// A full circle written as two half-arcs.
    Circle { cx: f64, cy: f64, r: f64 },
}

/// Where the Python generator's SVGs live.
///
/// They are large and belong to the curation checkout rather than to this one,
/// so their location is given rather than assumed.
pub fn python_svg_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("TUNNELS_SHAPES_PYTHON_SVGS") {
        Some(path) => PathBuf::from(path),
        None => {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tunnels-curate/svg_demo/shapes")
        }
    };
    dir.is_dir().then_some(dir)
}

/// The `d` attribute of the single path in an SVG the generator wrote.
pub fn read_path_data(svg: &str) -> Result<&str, String> {
    let rest = svg
        .split_once(" d=\"")
        .ok_or_else(|| "no path data".to_string())?
        .1;
    rest.split_once('"')
        .map(|(d, _)| d)
        .ok_or_else(|| "unterminated path data".to_string())
}

/// Split an SVG `d` attribute into its subpaths.
pub fn parse_path_data(d: &str) -> Result<Vec<Subpath>, String> {
    let mut tokens = d.split_whitespace();
    let mut subpaths = Vec::new();
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut arcs: Vec<(f64, (f64, f64))> = Vec::new();

    let number = |tokens: &mut std::str::SplitWhitespace| -> Result<f64, String> {
        let token = tokens.next().ok_or_else(|| "truncated".to_string())?;
        token.parse::<f64>().map_err(|e| format!("{token:?}: {e}"))
    };

    while let Some(command) = tokens.next() {
        match command {
            "M" => {
                points = vec![(number(&mut tokens)?, number(&mut tokens)?)];
                arcs.clear();
            }
            "L" => points.push((number(&mut tokens)?, number(&mut tokens)?)),
            "A" => {
                let radius = number(&mut tokens)?;
                for _ in 0..4 {
                    number(&mut tokens)?;
                }
                arcs.push((radius, (number(&mut tokens)?, number(&mut tokens)?)));
            }
            "Z" => {
                if arcs.is_empty() {
                    subpaths.push(Subpath::Poly(std::mem::take(&mut points)));
                } else {
                    let start = *points
                        .first()
                        .ok_or_else(|| "arc without start".to_string())?;
                    let r = arcs[0].0;
                    subpaths.push(Subpath::Circle {
                        cx: start.0 + r,
                        cy: start.1,
                        r,
                    });
                    arcs.clear();
                    points.clear();
                }
            }
            other => return Err(format!("unexpected command {other:?}")),
        }
    }
    Ok(subpaths)
}

/// Write a figure's contours the way the Python generator writes its polylines.
///
/// Coordinates are rounded to two decimals, which is the precision the SVGs
/// carry, so an identical figure produces an identical string.
pub fn format_figure(figure: &Figure) -> String {
    let mut out = String::new();
    for contour in &figure.contours {
        if !out.is_empty() {
            out.push(' ');
        }
        for (i, p) in contour.points().iter().enumerate() {
            let command = if i == 0 { "M" } else { " L" };
            let _ = write!(out, "{command} {:.2} {:.2}", p.x, p.y);
        }
        out.push_str(" Z");
    }
    out
}

/// A figure's shape recorded compactly enough to check into the repository.
///
/// The SVGs the Python wrote are tens of megabytes; this is what survives of one
/// when only the question "is the Rust output still the same figure" is asked of
/// it.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// A digest of the exact path data, for a figure written as polylines.
    Poly(u128),
    /// Centre and radius per circle, for a figure written as arcs.
    ///
    /// Arcs carry no points to digest, and the polyline standing in for one is a
    /// choice this port makes rather than something the Python fixes.
    Circles(Vec<(f64, f64, f64)>),
}

impl Recorded {
    /// Reduce one figure's path data to what the manifest keeps.
    pub fn of(d: &str) -> Result<Self, String> {
        let subpaths = parse_path_data(d)?;
        if subpaths.iter().all(|s| matches!(s, Subpath::Poly(_))) {
            return Ok(Self::Poly(digest(d)));
        }
        subpaths
            .iter()
            .map(|s| match s {
                Subpath::Circle { cx, cy, r } => Ok((*cx, *cy, *r)),
                Subpath::Poly(_) => Err("polylines mixed with arcs".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self::Circles)
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Poly(d) => format!("poly\t{d:032x}"),
            Self::Circles(circles) => {
                let body: Vec<String> = circles
                    .iter()
                    .map(|(cx, cy, r)| format!("{cx:.2},{cy:.2},{r:.2}"))
                    .collect();
                format!("circles\t{}", body.join(";"))
            }
        }
    }

    pub fn decode(kind: &str, body: &str) -> Result<Self, String> {
        match kind {
            "poly" => u128::from_str_radix(body, 16)
                .map(Self::Poly)
                .map_err(|e| e.to_string()),
            "circles" => body
                .split(';')
                .map(|circle| {
                    let mut parts = circle.split(',').map(str::parse::<f64>);
                    let mut next = || parts.next().transpose().ok().flatten().ok_or("bad circle");
                    Ok((next()?, next()?, next()?))
                })
                .collect::<Result<Vec<_>, &str>>()
                .map(Self::Circles)
                .map_err(str::to_string),
            other => Err(format!("unknown record kind {other:?}")),
        }
    }
}

/// FNV-1a over the bytes of a path, wide enough that a collision is not a risk
/// worth a dependency to avoid.
pub fn digest(s: &str) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    s.bytes().fold(OFFSET, |hash, byte| {
        (hash ^ byte as u128).wrapping_mul(PRIME)
    })
}
