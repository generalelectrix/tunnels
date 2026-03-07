// Vendored software rasterizer for Piston's Graphics trait.
// Based on graphics_buffer (MIT license, https://github.com/kaikalii/graphics_buffer).
// Simplified: no rayon, no texture UV, no glyphs — just solid-color triangle rasterization.

mod software_graphics {
    use graphics::draw_state::DrawState;
    use graphics::types::Color;
    use graphics::{Graphics, ImageSize};
    use image::{Rgba, RgbaImage};

    pub struct RenderBuffer {
        inner: RgbaImage,
    }

    impl RenderBuffer {
        pub fn new(width: u32, height: u32) -> Self {
            RenderBuffer {
                inner: RgbaImage::new(width, height),
            }
        }

        pub fn into_image(self) -> RgbaImage {
            self.inner
        }
    }

    impl ImageSize for RenderBuffer {
        fn get_size(&self) -> (u32, u32) {
            self.inner.dimensions()
        }
    }

    fn color_f32_rgba(color: &[f32; 4]) -> Rgba<u8> {
        Rgba([
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            (color[3] * 255.0) as u8,
        ])
    }

    fn color_rgba_f32(color: Rgba<u8>) -> [f32; 4] {
        [
            f32::from(color[0]) / 255.0,
            f32::from(color[1]) / 255.0,
            f32::from(color[2]) / 255.0,
            f32::from(color[3]) / 255.0,
        ]
    }

    fn layer_color(over: &[f32; 4], under: &[f32; 4]) -> [f32; 4] {
        let over_weight = 1.0 - (1.0 - over[3]).powf(2.0);
        let under_weight = 1.0 - over_weight;
        [
            over_weight * over[0] + under_weight * under[0],
            over_weight * over[1] + under_weight * under[1],
            over_weight * over[2] + under_weight * under[2],
            (over[3].powf(2.0) + under[3].powf(2.0)).sqrt().min(1.0),
        ]
    }

    fn sign(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> f32 {
        (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])
    }

    fn triangle_contains(tri: &[[f32; 2]], point: [f32; 2]) -> bool {
        let b1 = sign(point, tri[0], tri[1]) < 0.0;
        let b2 = sign(point, tri[1], tri[2]) < 0.0;
        let b3 = sign(point, tri[2], tri[0]) < 0.0;
        b1 == b2 && b2 == b3
    }

    impl Graphics for RenderBuffer {
        type Texture = RenderBuffer;

        fn clear_color(&mut self, color: Color) {
            for (_, _, pixel) in self.inner.enumerate_pixels_mut() {
                *pixel = color_f32_rgba(&color);
            }
        }

        fn clear_stencil(&mut self, _value: u8) {}

        fn tri_list<F>(&mut self, _draw_state: &DrawState, color: &[f32; 4], mut f: F)
        where
            F: FnMut(&mut dyn FnMut(&[[f32; 2]])),
        {
            f(&mut |vertices| {
                for tri in vertices.chunks(3) {
                    if tri.len() < 3 {
                        continue;
                    }
                    // Bounding box
                    let mut tl = [f32::MAX, f32::MAX];
                    let mut br = [f32::MIN, f32::MIN];
                    for v in tri {
                        tl[0] = tl[0].min(v[0]);
                        tl[1] = tl[1].min(v[1]);
                        br[0] = br[0].max(v[0]);
                        br[1] = br[1].max(v[1]);
                    }
                    let x0 = tl[0].floor().max(0.0) as i32;
                    let y0 = tl[1].floor().max(0.0) as i32;
                    let x1 = br[0].ceil().min(self.inner.width() as f32) as i32;
                    let y1 = br[1].ceil().min(self.inner.height() as f32) as i32;

                    for x in x0..x1 {
                        for y in y0..y1 {
                            if triangle_contains(tri, [x as f32, y as f32]) {
                                let under =
                                    color_rgba_f32(*self.inner.get_pixel(x as u32, y as u32));
                                let blended = layer_color(color, &under);
                                self.inner
                                    .put_pixel(x as u32, y as u32, color_f32_rgba(&blended));
                            }
                        }
                    }
                }
            });
        }

        fn tri_list_uv<F>(
            &mut self,
            _draw_state: &DrawState,
            _color: &[f32; 4],
            _texture: &Self::Texture,
            _f: F,
        ) where
            F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]])),
        {
            unimplemented!("tri_list_uv not needed for arc rendering tests")
        }

        fn tri_list_c<F>(&mut self, _: &DrawState, _: F)
        where
            F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 4]])),
        {
            unimplemented!("tri_list_c not needed for arc rendering tests")
        }

        fn tri_list_uv_c<F>(&mut self, _: &DrawState, _: &Self::Texture, _: F)
        where
            F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]], &[[f32; 4]])),
        {
            unimplemented!("tri_list_uv_c not needed for arc rendering tests")
        }
    }
}

use std::path::Path;
use std::sync::Arc;

use graphics::Graphics;
use software_graphics::RenderBuffer;
use tunnelclient::config::ClientConfig;
use tunnelclient::draw::Draw;
use tunnels_lib::{ArcSegment, Snapshot, Timestamp};

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

fn dot_config() -> ClientConfig {
    let mut cfg = test_config();
    cfg.render_mode = tunnelclient::config::RenderMode::Dot;
    cfg
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

fn test_arc(start: f64, stop: f64, hue: f64, radius: f64) -> ArcSegment {
    ArcSegment {
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
    let snapshot = tunnels::tunnel::default_tunnel_snapshot_fixture();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "default_tunnel.png");
}

#[test]
fn stress_tunnel() {
    let snapshot = tunnels::tunnel::stress_tunnel_snapshot_fixture();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel.png");
}

#[test]
fn stress_tunnel_evolved() {
    let snapshot = tunnels::tunnel::stress_tunnel_evolved_snapshot_fixture();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel_evolved.png");
}

#[test]
fn default_tunnel_dot_mode() {
    let snapshot = tunnels::tunnel::default_tunnel_snapshot_fixture();
    let image = render_snapshot(&snapshot, &dot_config());
    compare_to_fixture(&image, "default_tunnel_dot.png");
}

#[test]
fn stress_tunnel_dot_mode() {
    let snapshot = tunnels::tunnel::stress_tunnel_snapshot_fixture();
    let image = render_snapshot(&snapshot, &dot_config());
    compare_to_fixture(&image, "stress_tunnel_dot.png");
}
