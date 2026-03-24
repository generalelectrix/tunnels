mod software_graphics;

use std::path::Path;
use std::sync::Arc;

use software_graphics::RenderBuffer;
use tunnelclient::config::ClientConfig;
use tunnelclient::draw::Draw;
use tunnelclient::render::RenderTarget;
use tunnels_lib::{Shape, Snapshot, Timestamp};

const WIDTH: u32 = 512;
const HEIGHT: u32 = 512;
const WIDE_WIDTH: u32 = 768;

fn test_config_sized(width: u32, height: u32) -> ClientConfig {
    ClientConfig::new(
        0,
        "test".to_string(),
        (width, height),
        false,
        false,
        None,
        false,
    )
}

fn test_config() -> ClientConfig {
    test_config_sized(WIDTH, HEIGHT)
}

fn render_snapshot_sized(
    snapshot: &Snapshot,
    cfg: &ClientConfig,
    width: u32,
    height: u32,
) -> image::RgbaImage {
    let mut buffer = RenderBuffer::new(width, height);
    buffer.clear([0.0, 0.0, 0.0, 1.0]);
    snapshot.draw(&mut buffer, cfg);
    buffer.into_image()
}

fn render_snapshot(snapshot: &Snapshot, cfg: &ClientConfig) -> image::RgbaImage {
    render_snapshot_sized(snapshot, cfg, WIDTH, HEIGHT)
}

fn compare_to_fixture(actual: &image::RgbaImage, fixture_name: &str) {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let fixture_path = fixtures_dir.join(fixture_name);

    if std::env::var("UPDATE_FIXTURES").is_ok() {
        actual.save(&fixture_path).unwrap();
        eprintln!("Updated fixture: {}", fixture_path.display());
        return;
    }

    let expected = image::open(&fixture_path)
        .unwrap_or_else(|_| {
            panic!(
                "Missing fixture {}. Run with UPDATE_FIXTURES=1 to generate.",
                fixture_name
            )
        })
        .to_rgba8();

    let tolerance = 2u8;
    let mismatches: usize = actual
        .pixels()
        .zip(expected.pixels())
        .filter(|(a, e)| {
            a.0.iter()
                .zip(e.0.iter())
                .any(|(ac, ec)| ac.abs_diff(*ec) > tolerance)
        })
        .count();

    if mismatches > 0 {
        // Save actual render and visual diff for review
        let stem = fixture_name.trim_end_matches(".png");
        let diff_dir = fixtures_dir.join("diffs");
        std::fs::create_dir_all(&diff_dir).ok();

        let actual_path = diff_dir.join(format!("{stem}_actual.png"));
        let diff_path = diff_dir.join(format!("{stem}_diff.png"));

        actual.save(&actual_path).unwrap();
        let diff_img = generate_diff_image(actual, &expected, tolerance);
        diff_img.save(&diff_path).unwrap();

        panic!(
            "Image mismatch: {} pixels differ (out of {}). \
             Visual diff saved to:\n  actual: {}\n  diff:   {}\n\
             Run with UPDATE_FIXTURES=1 to accept new renders.",
            mismatches,
            actual.width() * actual.height(),
            actual_path.display(),
            diff_path.display(),
        );
    }
}

fn assert_images_match(actual: &image::RgbaImage, expected: &image::RgbaImage, tolerance: u8) {
    assert_images_match_with_limit(actual, expected, tolerance, 0);
}

fn assert_images_match_with_limit(
    actual: &image::RgbaImage,
    expected: &image::RgbaImage,
    tolerance: u8,
    max_mismatches: usize,
) {
    assert_eq!(
        actual.dimensions(),
        expected.dimensions(),
        "Image dimensions differ"
    );
    let mismatches: usize = actual
        .pixels()
        .zip(expected.pixels())
        .filter(|(a, e)| {
            a.0.iter()
                .zip(e.0.iter())
                .any(|(ac, ec)| ac.abs_diff(*ec) > tolerance)
        })
        .count();

    if mismatches > max_mismatches {
        panic!(
            "Image mismatch: {} pixels differ (out of {}, max allowed: {}). \
             Run with UPDATE_FIXTURES=1 to update.",
            mismatches,
            actual.width() * actual.height(),
            max_mismatches,
        );
    }
}

/// Generate a visual diff image.
/// - Black: identical pixels (within tolerance)
/// - White: edge coverage difference (one side is near-black, or same hue different brightness)
/// - Bright pink: actual hue shift (the color itself changed)
fn generate_diff_image(
    actual: &image::RgbaImage,
    expected: &image::RgbaImage,
    tolerance: u8,
) -> image::RgbaImage {
    let (w, h) = actual.dimensions();
    let mut diff = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let a = actual.get_pixel(x, y);
            let e = expected.get_pixel(x, y);
            let matches =
                a.0.iter()
                    .zip(e.0.iter())
                    .all(|(ac, ec)| ac.abs_diff(*ec) <= tolerance);
            if matches {
                diff.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            } else if is_edge_difference(&a.0, &e.0) {
                diff.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            } else {
                diff.put_pixel(x, y, image::Rgba([255, 0, 255, 255]));
            }
        }
    }
    diff
}

const NEAR_BLACK_THRESHOLD: f32 = 10.0;

/// An "edge difference" is either:
/// - One pixel is near-black and the other isn't (edge coverage shift)
/// - Both are colored but with the same hue (brightness-only change)
fn is_edge_difference(a: &[u8; 4], b: &[u8; 4]) -> bool {
    let a_sum = a[0] as f32 + a[1] as f32 + a[2] as f32;
    let b_sum = b[0] as f32 + b[1] as f32 + b[2] as f32;

    // Either side is near-black → edge coverage difference
    if a_sum < NEAR_BLACK_THRESHOLD || b_sum < NEAR_BLACK_THRESHOLD {
        return true;
    }

    // Both have color — check if the hue is the same (ratio comparison)
    let threshold = 0.1;
    for i in 0..3 {
        let a_ratio = a[i] as f32 / a_sum;
        let b_ratio = b[i] as f32 / b_sum;
        if (a_ratio - b_ratio).abs() > threshold {
            return false;
        }
    }
    true
}

fn test_arc(start: f64, stop: f64, hue: f64, radius: f64) -> Shape {
    Shape {
        render_mode: Default::default(),
        level: 1.0,
        thickness: 0.1,
        hue,
        sat: 1.0,
        val: 1.0,
        x: 0.0,
        y: 0.0,
        rad_x: radius,
        rad_y: radius,
        start,
        stop,
        rot_angle: 0.0,
        spin_angle: 0.0,
    }
}

#[test]
fn single_arc() {
    let snapshot = Snapshot {
        frame_number: 0,
        time: Timestamp(0),
        layers: vec![Arc::new(vec![test_arc(0.0, 0.25, 0.0, 0.4)])],
    };
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "single_arc.png");
}

#[test]
fn concentric_rings() {
    let layers = vec![Arc::new(vec![
        test_arc(0.0, 1.0, 0.0, 0.2),
        test_arc(0.0, 1.0, 0.33, 0.35),
        test_arc(0.0, 1.0, 0.66, 0.5),
    ])];
    let snapshot = Snapshot {
        frame_number: 0,
        time: Timestamp(0),
        layers,
    };
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "concentric_rings.png");
}

#[test]
fn rotated_arc() {
    let mut seg = test_arc(0.0, 0.5, 0.6, 0.3);
    seg.rot_angle = 0.125; // 45 degrees
    let snapshot = Snapshot {
        frame_number: 0,
        time: Timestamp(0),
        layers: vec![Arc::new(vec![seg])],
    };
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "rotated_arc.png");
}

#[test]
fn flipped_horizontal() {
    use tunnelclient::draw::{Transform, TransformDirection};

    let mut seg = test_arc(0.0, 0.25, 0.0, 0.4);
    seg.x = 0.3; // offset from center so flip is visually distinct

    let snapshot = Snapshot {
        frame_number: 0,
        time: Timestamp(0),
        layers: vec![Arc::new(vec![seg])],
    };

    // Render without flip and compare to fixture.
    let unflipped = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&unflipped, "flipped_horizontal_unflipped.png");

    // Render with flip_horizontal transform enabled.
    let flipped_cfg = ClientConfig::new(
        0,
        "test".to_string(),
        (WIDTH, HEIGHT),
        false,
        false,
        Some(Transform::Flip(TransformDirection::Horizontal)),
        false,
    );
    let flipped = render_snapshot(&snapshot, &flipped_cfg);

    // Flip the unflipped image in memory and assert it matches the rendered flip.
    // Allow a small number of edge-pixel mismatches: the triangle rasterizer makes
    // slightly different containment decisions when geometry is mirrored vs when
    // the final image is flipped, due to sub-pixel rounding at arc edges.
    let expected = image::imageops::flip_horizontal(&unflipped);
    assert_images_match_with_limit(&flipped, &expected, 2, 100);
}

#[test]
fn solid_ring() {
    let snapshot = tunnels::tunnel::fixture::solid_ring_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "solid_ring.png");
}

#[test]
fn default_tunnel() {
    let snapshot = tunnels::tunnel::fixture::default_tunnel_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "default_tunnel.png");
}

#[test]
fn stress_tunnel() {
    let snapshot = tunnels::tunnel::fixture::stress_tunnel_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel.png");
}

#[test]
fn stress_tunnel_evolved() {
    let snapshot = tunnels::tunnel::fixture::stress_tunnel_evolved_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel_evolved.png");
}

#[test]
fn default_tunnel_dot_mode() {
    let snapshot = tunnels::tunnel::fixture::default_tunnel_dot_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "default_tunnel_dot.png");
}

#[test]
fn stress_tunnel_dot_mode() {
    let snapshot = tunnels::tunnel::fixture::stress_tunnel_dot_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel_dot.png");
}

#[test]
fn elliptical_tunnel() {
    let snapshot = tunnels::tunnel::fixture::elliptical_tunnel_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "elliptical_tunnel.png");
}

#[test]
fn elliptical_tunnel_dot_mode() {
    let snapshot = tunnels::tunnel::fixture::elliptical_tunnel_dot_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "elliptical_tunnel_dot.png");
}

#[test]
fn saucer_few_thin() {
    let snapshot = tunnels::tunnel::fixture::saucer_few_thin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_few_thin.png");
}

#[test]
fn saucer_many_thick() {
    let snapshot = tunnels::tunnel::fixture::saucer_many_thick_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_many_thick.png");
}

#[test]
fn saucer_wide_ellipse() {
    let snapshot = tunnels::tunnel::fixture::saucer_wide_ellipse_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "saucer_wide_ellipse.png");
}

#[test]
fn saucer_tall_ellipse() {
    let snapshot = tunnels::tunnel::fixture::saucer_tall_ellipse_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_tall_ellipse.png");
}

#[test]
fn saucer_few_thin_spin() {
    let snapshot = tunnels::tunnel::fixture::saucer_few_thin_spin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_few_thin_spin.png");
}

#[test]
fn saucer_many_thick_spin() {
    let snapshot = tunnels::tunnel::fixture::saucer_many_thick_spin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_many_thick_spin.png");
}

#[test]
fn saucer_wide_ellipse_spin() {
    let snapshot = tunnels::tunnel::fixture::saucer_wide_ellipse_spin_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "saucer_wide_ellipse_spin.png");
}

#[test]
fn saucer_tall_ellipse_spin() {
    let snapshot = tunnels::tunnel::fixture::saucer_tall_ellipse_spin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_tall_ellipse_spin.png");
}

#[test]
fn empty_snapshot() {
    let snapshot = Snapshot {
        frame_number: 0,
        time: Timestamp(0),
        layers: vec![],
    };
    let image = render_snapshot(&snapshot, &test_config());
    // Should be solid black
    for pixel in image.pixels() {
        assert_eq!(
            pixel.0,
            [0, 0, 0, 255],
            "Empty snapshot should be solid black"
        );
    }
}

#[test]
fn composited_layers() {
    let snapshot = tunnels::tunnel::fixture::composited_layers_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "composited_layers.png");
}

#[test]
fn mask_over_stress() {
    let snapshot = tunnels::tunnel::fixture::mask_over_stress_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "mask_over_stress.png");
}
