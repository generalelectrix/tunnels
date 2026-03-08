mod software_graphics;

use std::path::Path;
use std::sync::Arc;

use graphics::Graphics;
use software_graphics::RenderBuffer;
use tunnelclient::config::ClientConfig;
use tunnelclient::draw::Draw;
use tunnels_lib::{Shape, Snapshot, Timestamp};

const WIDTH: u32 = 512;
const HEIGHT: u32 = 512;

fn test_config() -> ClientConfig {
    ClientConfig::new(
        0,
        "test".to_string(),
        (WIDTH, HEIGHT),
        false,
        false,
        None,
        false,
    )
}

fn render_snapshot(snapshot: &Snapshot, cfg: &ClientConfig) -> image::RgbaImage {
    let mut buffer = RenderBuffer::new(WIDTH, HEIGHT);
    // Clear to black
    buffer.clear_color([0.0, 0.0, 0.0, 1.0]);

    // Use identity transform so triangulated vertices stay in pixel coordinates.
    // The draw code in draw.rs computes pixel positions directly (x * resolution + center),
    // and the triangulation applies the context transform to produce output vertices.
    // With OpenGL, abs_transform maps pixels to NDC for the GPU. For our software
    // rasterizer, we want vertices in pixel space, so we use identity.
    let context = graphics::Context::new();
    snapshot.draw(&context, &mut buffer, cfg);

    buffer.into_image()
}

fn compare_to_fixture(actual: &image::RgbaImage, fixture_name: &str) {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture_name);

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

    assert_images_match(actual, &expected, 2);
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
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "elliptical_tunnel.png");
}

#[test]
fn elliptical_tunnel_dot_mode() {
    let snapshot = tunnels::tunnel::fixture::elliptical_tunnel_dot_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "elliptical_tunnel_dot.png");
}
