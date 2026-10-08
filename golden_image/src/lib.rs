//! Comparison of a rendered image against a golden checked in beside the test
//! that renders it.
//!
//! A render is held to its golden to within a tolerance rather than exactly,
//! because a rasteriser decides pixel coverage at a shape's edge by arithmetic
//! that a change anywhere upstream can round the other way. Behind a failure
//! the comparison writes what was rendered, what was expected, and where they
//! differ, since a count of differing pixels says that a comparison failed but
//! not how.

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};

/// The environment variable that makes a comparison overwrite its golden
/// instead of asserting against it.
const UPDATE_VAR: &str = "UPDATE_FIXTURES";

/// How far a render may stray from its golden and still count as a match.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    /// The largest difference on any one channel that still reads as the same
    /// pixel.
    pub channel: u8,
    /// How many pixels may exceed `channel` and still read as the same image.
    pub pixels: usize,
}

impl Tolerance {
    /// Every channel of every pixel equal.
    pub const EXACT: Self = Self {
        channel: 0,
        pixels: 0,
    };

    /// Every pixel equal to within `channel` on each of its channels.
    pub const fn per_channel(channel: u8) -> Self {
        Self { channel, pixels: 0 }
    }

    /// The same tolerance, additionally permitting `pixels` pixels to exceed it.
    pub const fn allowing(self, pixels: usize) -> Self {
        Self { pixels, ..self }
    }

    /// Whether two pixels are the same within this tolerance.
    fn matches(&self, a: &Rgba<u8>, e: &Rgba<u8>) -> bool {
        a.0.iter()
            .zip(e.0.iter())
            .all(|(ac, ec)| ac.abs_diff(*ec) <= self.channel)
    }
}

/// A directory of golden images, and the tolerance a render is held to against
/// one of them.
///
/// Both directories are explicit rather than derived: a golden lives beside
/// the test that renders it, a different directory for every suite of them,
/// and the directory a test target may write to is named by an environment
/// variable that Cargo expands at the compile time of the crate reading it.
pub struct Goldens {
    fixture_dir: PathBuf,
    artifact_dir: PathBuf,
    tolerance: Tolerance,
}

impl Goldens {
    /// Goldens kept in `fixture_dir`, held to `tolerance`, writing the images
    /// behind a failure into `artifact_dir`.
    ///
    /// Artifacts are named after the golden they belong to, so an
    /// `artifact_dir` shared by two suites that use one name between them
    /// leaves only whichever wrote it last.
    pub fn new(fixture_dir: PathBuf, artifact_dir: PathBuf, tolerance: Tolerance) -> Self {
        Self {
            fixture_dir,
            artifact_dir,
            tolerance,
        }
    }

    /// Assert that `actual` matches the golden of the given name, or overwrite
    /// that golden when the environment asks for an update.
    ///
    /// # Panics
    ///
    /// If the images differ by more than the tolerance, or if the named golden
    /// is absent.
    pub fn compare(&self, actual: &RgbaImage, fixture_name: &str) {
        let fixture_path = self.fixture_dir.join(fixture_name);

        if std::env::var(UPDATE_VAR).is_ok() {
            if let Some(parent) = fixture_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            actual.save(&fixture_path).unwrap();
            eprintln!("Updated fixture: {}", fixture_path.display());
            return;
        }

        let expected = image::open(&fixture_path)
            .unwrap_or_else(|_| {
                panic!("Missing fixture {fixture_name}. Run with {UPDATE_VAR}=1 to generate.")
            })
            .to_rgba8();

        self.assert_match(actual, &expected, self.tolerance, fixture_name);
    }

    /// Assert that two images match to within the given tolerance, writing the
    /// images behind a failure under this harness's artifact directory.
    ///
    /// # Panics
    ///
    /// If the images differ in size, or by more than the tolerance.
    pub fn assert_match(
        &self,
        actual: &RgbaImage,
        expected: &RgbaImage,
        tolerance: Tolerance,
        name: &str,
    ) {
        assert_eq!(
            actual.dimensions(),
            expected.dimensions(),
            "Image dimensions differ for {name}"
        );
        let mismatches = actual
            .pixels()
            .zip(expected.pixels())
            .filter(|(a, e)| !tolerance.matches(a, e))
            .count();

        if mismatches > tolerance.pixels {
            let dir = write_comparison(&self.artifact_dir, actual, expected, tolerance, name);
            panic!(
                "Image mismatch for {}: {} pixels differ (out of {}, max allowed: {}). \
                 Rendered, expected and difference images written to {}. \
                 Run with {}=1 to update.",
                name,
                mismatches,
                actual.width() * actual.height(),
                tolerance.pixels,
                dir.display(),
                UPDATE_VAR,
            );
        }
    }
}

/// Write what was rendered, what was expected, and where they differ into
/// `dir`, and return where they were put.
///
/// An image is the only form the answer to a failed comparison takes: whether
/// a shape moved, whether a colour shifted, or whether a handful of edge pixels
/// landed on the other side of a triangle boundary are three different
/// failures behind the same count of differing pixels.
///
/// The difference is drawn as the expected image dimmed to a quarter, with the
/// pixels that differ picked out in magenta, so a change reads against the
/// shape it happened to rather than against nothing.
fn write_comparison(
    dir: &Path,
    actual: &RgbaImage,
    expected: &RgbaImage,
    tolerance: Tolerance,
    name: &str,
) -> PathBuf {
    let stem = name.strip_suffix(".png").unwrap_or(name);
    if std::fs::create_dir_all(dir).is_err() {
        return dir.to_path_buf();
    }

    let mut diff = expected.clone();
    for (px, (a, e)) in diff
        .pixels_mut()
        .zip(actual.pixels().zip(expected.pixels()))
    {
        *px = if tolerance.matches(a, e) {
            Rgba([e.0[0] / 4, e.0[1] / 4, e.0[2] / 4, 255])
        } else {
            Rgba([255, 0, 255, 255])
        };
    }

    for (suffix, img) in [("actual", actual), ("expected", expected), ("diff", &diff)] {
        let _ = img.save(dir.join(format!("{stem}.{suffix}.png")));
    }
    dir.to_path_buf()
}
