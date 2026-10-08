//! What a comparison against a golden accepts, what it rejects, and what it
//! leaves behind when it rejects.

use std::path::{Path, PathBuf};

use golden_image::{Goldens, Tolerance};
use image::{Rgba, RgbaImage};

/// A solid image of the given colour, the size every image in this suite is.
fn solid(colour: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(4, 4, Rgba(colour))
}

/// A directory of this suite's own, named for the case using it, so one case
/// cannot read or overwrite another's images.
fn scratch(case: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(case);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `f`, returning the message it panicked with.
fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let payload = std::panic::catch_unwind(f).expect_err("expected a panic");
    std::panic::set_hook(previous);

    match payload.downcast_ref::<String>() {
        Some(message) => message.clone(),
        None => payload
            .downcast_ref::<&str>()
            .expect("panic payload was neither a String nor a &str")
            .to_string(),
    }
}

/// A golden written into `dir` under the given name.
fn write_golden(dir: &Path, name: &str, colour: [u8; 4]) {
    solid(colour).save(dir.join(name)).unwrap();
}

/// A difference on a channel is forgiven up to the tolerance and no further,
/// and a count of forgiven pixels is held to its own budget.
#[test]
fn a_tolerance_bounds_both_the_channel_and_the_count() {
    let fixtures = scratch("tolerance_fixtures");
    let artifacts = scratch("tolerance_artifacts");
    write_golden(&fixtures, "flat.png", [10, 20, 30, 255]);
    let held_to = |tolerance| Goldens::new(fixtures.clone(), artifacts.clone(), tolerance);

    let goldens = held_to(Tolerance::per_channel(2));

    goldens.compare(&solid([10, 20, 30, 255]), "flat.png");
    goldens.compare(&solid([12, 18, 32, 255]), "flat.png");

    let message = panic_message(|| goldens.compare(&solid([13, 20, 30, 255]), "flat.png"));
    assert!(
        message.contains("Image mismatch for flat.png") && message.contains("16 pixels differ"),
        "a difference of 3 on a tolerance of 2 was reported as {message:?}"
    );

    // One pixel past tolerance, against a budget of one and then of none.
    let mut one_off = solid([10, 20, 30, 255]);
    one_off.put_pixel(0, 0, Rgba([13, 20, 30, 255]));
    held_to(Tolerance::per_channel(2).allowing(1)).compare(&one_off, "flat.png");

    let message = panic_message(|| goldens.compare(&one_off, "flat.png"));
    assert!(
        message.contains("1 pixels differ") && message.contains("max allowed: 0"),
        "one pixel past a budget of none was reported as {message:?}"
    );
}

/// Two images of different sizes are not a difference to be counted.
#[test]
fn a_size_difference_is_reported_as_itself() {
    let goldens = Goldens::new(
        scratch("size_fixtures"),
        scratch("size_artifacts"),
        Tolerance::EXACT,
    );
    let message = panic_message(|| {
        goldens.assert_match(
            &RgbaImage::new(4, 4),
            &RgbaImage::new(4, 5),
            Tolerance::EXACT,
            "resized",
        )
    });
    assert!(
        message.contains("Image dimensions differ for resized"),
        "a size difference was reported as {message:?}"
    );
}

/// An absent golden asks to be generated rather than reporting a difference.
#[test]
fn an_absent_golden_asks_to_be_generated() {
    let goldens = Goldens::new(
        scratch("absent_fixtures"),
        scratch("absent_artifacts"),
        Tolerance::EXACT,
    );
    let message = panic_message(|| goldens.compare(&solid([0, 0, 0, 255]), "never_rendered.png"));
    assert!(
        message.contains("Missing fixture never_rendered.png")
            && message.contains("UPDATE_FIXTURES=1"),
        "an absent golden was reported as {message:?}"
    );
}

/// A failure leaves the render, the golden, and the difference between them in
/// the artifact directory, with the difference picking out the pixels that
/// differ.
#[test]
fn a_failure_leaves_the_three_images_behind() {
    let fixtures = scratch("artifact_fixtures");
    let artifacts = scratch("artifact_artifacts");
    write_golden(&fixtures, "flat.png", [10, 20, 30, 255]);

    let goldens = Goldens::new(fixtures, artifacts.clone(), Tolerance::EXACT);
    let mut actual = solid([10, 20, 30, 255]);
    actual.put_pixel(1, 2, Rgba([255, 255, 255, 255]));
    let message = panic_message(|| goldens.compare(&actual, "flat.png"));
    assert!(
        message.contains(&artifacts.display().to_string()),
        "the failure did not say where it put its images: {message:?}"
    );

    for suffix in ["actual", "expected", "diff"] {
        let path = artifacts.join(format!("flat.{suffix}.png"));
        assert!(path.is_file(), "no {suffix} image at {}", path.display());
    }

    let diff = image::open(artifacts.join("flat.diff.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(
        *diff.get_pixel(1, 2),
        Rgba([255, 0, 255, 255]),
        "the pixel that differed was not picked out"
    );
    assert_eq!(
        *diff.get_pixel(0, 0),
        Rgba([2, 5, 7, 255]),
        "a pixel that matched was not dimmed to a quarter of the golden"
    );
}
