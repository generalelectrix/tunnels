mod software_graphics;

use std::path::Path;

use client_lib::config::ClientConfig;
use graphics::Graphics;
use software_graphics::RenderBuffer;
use tunnelclient::fill::Renderer;
use tunnels_lib::number::UnipolarFloat;
use tunnels_model::layer::{
    Hsva, Layer, LayerCollection, PaintMode, PhaseAxis, Placement, RenderMode, SegmentLayer,
    SegmentPath, ShapeGeometry,
};
use tunnels_model::tunnel::fixture;

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
        false,
    )
}

fn test_config() -> ClientConfig {
    test_config_sized(WIDTH, HEIGHT)
}

fn render_snapshot_sized(
    snapshot: &LayerCollection,
    cfg: &ClientConfig,
    width: u32,
    height: u32,
) -> image::RgbaImage {
    let mut buffer = RenderBuffer::new(width, height);
    // Clear to black
    buffer.clear_color([0.0, 0.0, 0.0, 1.0]);

    // Use identity transform so triangulated vertices stay in pixel coordinates.
    // The draw code in draw.rs computes pixel positions directly (x * resolution + center),
    // and the triangulation applies the context transform to produce output vertices.
    // With OpenGL, abs_transform maps pixels to NDC for the GPU. For our software
    // rasterizer, we want vertices in pixel space, so we use identity.
    let context = graphics::Context::new();
    Renderer::default().draw(snapshot, &context, &mut buffer, cfg);

    buffer.into_image()
}

fn render_snapshot(snapshot: &LayerCollection, cfg: &ClientConfig) -> image::RgbaImage {
    render_snapshot_sized(snapshot, cfg, WIDTH, HEIGHT)
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

fn compare_to_fixture_with_limit(
    actual: &image::RgbaImage,
    fixture_name: &str,
    max_mismatches: usize,
) {
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
            panic!("Missing fixture {fixture_name}. Run with UPDATE_FIXTURES=1 to generate.")
        })
        .to_rgba8();

    assert_images_match_with_limit(actual, &expected, 2, max_mismatches);
}

/// Compare a figure render against its golden, allowing a few edge pixels.
///
/// The software rasteriser makes different coverage decisions at a triangle
/// edge than the GL one does, and a figure has tens of thousands of triangles
/// where a tunnel has a hundred segments — so a change anywhere upstream that
/// moves one vertex onto an edge moves a handful of pixels. The budget is
/// small enough that a real change still fails.
fn compare_fill_to_fixture(actual: &image::RgbaImage, fixture_name: &str) {
    compare_to_fixture_with_limit(actual, fixture_name, 200);
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

/// Wrap shapes in a layer drawn with the default render mode and path shape,
/// every segment spanning `span` turns.
fn default_layer(span: f64, shapes: Vec<ShapeGeometry>) -> Layer {
    layer_in_mode(PaintMode::Normal, span, shapes)
}

/// A default-shaped layer that paints in the given mode, at full level.
fn layer_in_mode(mode: PaintMode, span: f64, shapes: Vec<ShapeGeometry>) -> Layer {
    layer_in_mode_at(mode, 1.0, span, shapes)
}

/// A default-shaped layer that paints in the given mode at the given level.
fn layer_in_mode_at(mode: PaintMode, level: f64, span: f64, shapes: Vec<ShapeGeometry>) -> Layer {
    Layer::Segments(SegmentLayer {
        render_mode: RenderMode::default(),
        segment_path: SegmentPath::Ellipse,
        mode,
        level,
        span,
        shapes,
    })
}

fn test_arc(start: f64, hue: f64, radius: f64) -> ShapeGeometry {
    ShapeGeometry {
        color: Hsva {
            hue,
            sat: 1.0,
            val: 1.0,
            level: 1.0,
        },
        placement: Placement {
            x: 0.0,
            y: 0.0,
            extent_x: radius,
            extent_y: radius,
            rot_angle: 0.0,
        },
        thickness: 0.1,
        start,
        spin_angle: 0.0,
    }
}

#[test]
fn single_arc() {
    let snapshot = vec![default_layer(0.25, vec![test_arc(0.0, 0.0, 0.4)])];
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "single_arc.png");
}

#[test]
fn concentric_rings() {
    let snapshot = vec![default_layer(
        1.0,
        vec![
            test_arc(0.0, 0.0, 0.2),
            test_arc(0.0, 0.33, 0.35),
            test_arc(0.0, 0.66, 0.5),
        ],
    )];
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "concentric_rings.png");
}

#[test]
fn rotated_arc() {
    let mut seg = test_arc(0.0, 0.6, 0.3);
    let span = 0.5;
    seg.placement.rot_angle = 0.125; // 45 degrees
    let snapshot = vec![default_layer(span, vec![seg])];
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "rotated_arc.png");
}

#[test]
fn flipped_horizontal() {
    use client_lib::transform::{Transform, TransformDirection};

    let mut seg = test_arc(0.0, 0.0, 0.4);
    let span = 0.25;
    seg.placement.x = 0.3; // offset from center so flip is visually distinct

    let snapshot = vec![default_layer(span, vec![seg])];

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
    let snapshot = tunnels_model::tunnel::fixture::default_tunnel_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "default_tunnel.png");
}

#[test]
fn stress_tunnel() {
    let snapshot = tunnels_model::tunnel::fixture::stress_tunnel_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel.png");
}

#[test]
fn stress_tunnel_evolved() {
    let snapshot = tunnels_model::tunnel::fixture::stress_tunnel_evolved_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel_evolved.png");
}

#[test]
fn default_tunnel_dot_mode() {
    let snapshot = tunnels_model::tunnel::fixture::default_tunnel_dot_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "default_tunnel_dot.png");
}

#[test]
fn stress_tunnel_dot_mode() {
    let snapshot = tunnels_model::tunnel::fixture::stress_tunnel_dot_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "stress_tunnel_dot.png");
}

#[test]
fn elliptical_tunnel() {
    let snapshot = tunnels_model::tunnel::fixture::elliptical_tunnel_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "elliptical_tunnel.png");
}

#[test]
fn elliptical_tunnel_dot_mode() {
    let snapshot = tunnels_model::tunnel::fixture::elliptical_tunnel_dot_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "elliptical_tunnel_dot.png");
}

#[test]
fn saucer_few_thin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_few_thin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_few_thin.png");
}

#[test]
fn saucer_many_thick() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_many_thick_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_many_thick.png");
}

#[test]
fn saucer_wide_ellipse() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_wide_ellipse_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "saucer_wide_ellipse.png");
}

#[test]
fn saucer_tall_ellipse() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_tall_ellipse_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_tall_ellipse.png");
}

#[test]
fn saucer_few_thin_spin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_few_thin_spin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_few_thin_spin.png");
}

#[test]
fn saucer_many_thick_spin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_many_thick_spin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_many_thick_spin.png");
}

#[test]
fn saucer_wide_ellipse_spin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_wide_ellipse_spin_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "saucer_wide_ellipse_spin.png");
}

#[test]
fn saucer_tall_ellipse_spin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_tall_ellipse_spin_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "saucer_tall_ellipse_spin.png");
}

// --- Ellipse path: arc spin ---

#[test]
fn arc_spin_many() {
    let snapshot = tunnels_model::tunnel::fixture::arc_spin_many_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "arc_spin_many.png");
}

#[test]
fn arc_spin_few() {
    let snapshot = tunnels_model::tunnel::fixture::arc_spin_few_snapshot();
    let image = render_snapshot(&snapshot, &test_config());
    compare_to_fixture(&image, "arc_spin_few.png");
}

#[test]
fn arc_spin_wide_ellipse() {
    let snapshot = tunnels_model::tunnel::fixture::arc_spin_wide_ellipse_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "arc_spin_wide_ellipse.png");
}

// --- Line path: full-tunnel snapshots ---

#[test]
fn default_tunnel_line() {
    let snapshot = tunnels_model::tunnel::fixture::default_tunnel_line_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "default_tunnel_line.png");
}

#[test]
fn default_tunnel_line_dot() {
    let snapshot = tunnels_model::tunnel::fixture::default_tunnel_line_dot_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "default_tunnel_line_dot.png");
}

#[test]
fn saucer_line_few_thin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_line_few_thin_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "saucer_line_few_thin.png");
}

#[test]
fn saucer_line_spin() {
    let snapshot = tunnels_model::tunnel::fixture::saucer_line_spin_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "saucer_line_spin.png");
}

#[test]
fn arc_line_spin() {
    let snapshot = tunnels_model::tunnel::fixture::arc_line_spin_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "arc_line_spin.png");
}

// --- Line path: direct Shape edge-wrapping tests ---

/// Helper to create a line-path shape with specific start and stop angles.
fn test_line_shape(start: f64) -> ShapeGeometry {
    ShapeGeometry {
        color: Hsva {
            hue: 0.0,
            sat: 1.0,
            val: 1.0,
            level: 1.0,
        },
        placement: Placement {
            x: 0.0,
            y: 0.0,
            extent_x: 0.4, // line half-length
            extent_y: 0.0, // on the line (no perpendicular offset)
            rot_angle: 0.0,
        },
        thickness: 0.1,
        start,
        spin_angle: 0.0,
    }
}

/// One line-path layer per (span, shapes) group, drawn in the order given.
fn snapshot_from_groups(
    render_mode: RenderMode,
    groups: Vec<(f64, Vec<ShapeGeometry>)>,
) -> LayerCollection {
    groups
        .into_iter()
        .map(|(span, shapes)| {
            Layer::Segments(SegmentLayer {
                render_mode,
                segment_path: SegmentPath::Line,
                mode: PaintMode::Normal,
                level: 1.0,
                span,
                shapes,
            })
        })
        .collect()
}

/// Arc segment that wraps past the right end of the line.
#[test]
fn line_arc_edge_wrap() {
    let render_mode = RenderMode::Arc;
    let snapshot = snapshot_from_groups(
        render_mode,
        vec![
            // A segment sitting squarely on the line (no wrapping).
            (0.1, vec![test_line_shape(0.3)]),
            // A segment that wraps past the right end.
            (0.15, vec![test_line_shape(0.9)]),
        ],
    );
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "line_arc_edge_wrap.png");
}

/// Dot near the edge of the line, cross-fading between ends.
#[test]
fn line_dot_edge_crossfade() {
    let render_mode = RenderMode::Dot;
    let seg_width = 1.0 / 12.0; // simulate 12 segments
    let snapshot = snapshot_from_groups(
        render_mode,
        vec![(
            seg_width,
            vec![
                // A dot in the middle of the line (no fading).
                test_line_shape(0.25),
                // A dot just entering the fade zone near the right end.
                test_line_shape(0.95 - seg_width / 2.0),
                // A dot midway through wrapping.
                test_line_shape(1.0 - seg_width / 4.0),
            ],
        )],
    );
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "line_dot_edge_crossfade.png");
}

/// Saucer near the edge of the line, cross-fading between ends.
#[test]
fn line_saucer_edge_crossfade() {
    let render_mode = RenderMode::Saucer;
    let seg_width = 1.0 / 12.0;
    let snapshot = snapshot_from_groups(
        render_mode,
        vec![(
            seg_width,
            vec![
                // A saucer in the middle of the line.
                test_line_shape(0.25),
                // A saucer just entering the fade zone near the right end.
                test_line_shape(0.95 - seg_width / 2.0),
                // A saucer midway through wrapping.
                test_line_shape(1.0 - seg_width / 4.0),
            ],
        )],
    );
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "line_saucer_edge_crossfade.png");
}

// --- Line path: aspect ratio animation (wiggle) ---

#[test]
fn line_aspect_ratio_anim_arc() {
    let snapshot = tunnels_model::tunnel::fixture::line_aspect_ratio_anim_arc_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "line_aspect_ratio_anim_arc.png");
}

#[test]
fn line_aspect_ratio_anim_dot() {
    let snapshot = tunnels_model::tunnel::fixture::line_aspect_ratio_anim_dot_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "line_aspect_ratio_anim_dot.png");
}

#[test]
fn line_aspect_ratio_anim_saucer() {
    let snapshot = tunnels_model::tunnel::fixture::line_aspect_ratio_anim_saucer_snapshot();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    let image = render_snapshot_sized(&snapshot, &cfg, WIDE_WIDTH, HEIGHT);
    compare_to_fixture(&image, "line_aspect_ratio_anim_saucer.png");
}

// --- Line path: saucer marquee evolution sequence ---

#[test]
fn saucer_line_marquee_sequence() {
    let snapshots = tunnels_model::tunnel::fixture::saucer_line_marquee_sequence();
    let cfg = test_config_sized(WIDE_WIDTH, HEIGHT);
    for (i, snapshot) in snapshots.iter().enumerate() {
        let image = render_snapshot_sized(snapshot, &cfg, WIDE_WIDTH, HEIGHT);
        compare_to_fixture(&image, &format!("saucer_line_marquee_f{i}.png"));
    }
}

/// The golden fill images below draw these figures, and a figure added to the
/// library renumbers everything after it. Naming them here is what turns that
/// into a failing test rather than a golden image quietly becoming a picture of
/// something else.
#[test]
fn the_fixtures_draw_the_figures_they_were_taken_of() {
    for (id, name) in [
        (fixture::SNOWFLAKE, "blossoms/snowflake"),
        (fixture::BULLSEYE, "emblems/bullseye"),
        (fixture::UMBRELLA, "novelty/umbrella"),
        (fixture::PINWHEEL, "pinwheels/six_pointed"),
    ] {
        let sprite = tunnels_sprites::sprite(id.0)
            .unwrap_or_else(|| panic!("this build carries no figure {}", id.0));
        assert_eq!(sprite.name, name, "figure {} is not {name}", id.0);
    }
}

/// A figure in one colour, which is the path that skips the ramp entirely and
/// draws the tessellator's own triangles.
#[test]
fn sprite_flat() {
    let image = render_snapshot(&fixture::sprite_flat_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_flat.png");
}

/// The colour sweep along each coordinate a figure can be indexed by.
///
/// Angular is the one with a seam in it — `atan2` wraps on the far side — so
/// it is the one that would show a band of spurious rainbow if the
/// same-branch shift were wrong.
#[test]
fn sprite_color_on_each_axis() {
    for (phase, name) in [
        (PhaseAxis::Angle, "sprite_color_angle.png"),
        (PhaseAxis::Radius, "sprite_color_radius.png"),
        (PhaseAxis::Linear, "sprite_color_linear.png"),
    ] {
        let image = render_snapshot(&fixture::sprite_color_snapshot(phase), &test_config());
        compare_fill_to_fixture(&image, name);
    }
}

/// A figure whose six sectors are carved by one contour meeting itself at a
/// single shared vertex, which is the case a moved coordinate destroys.
#[test]
fn sprite_shared_vertex() {
    let image = render_snapshot(&fixture::sprite_shared_vertex_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_shared_vertex.png");
}

/// A colour animation over a figure that already carries a colour sweep. The
/// animation's period is the figure's and not the colour cycle's, so one
/// saturation lobe runs across a figure swept three times.
#[test]
fn sprite_color_animation() {
    let image = render_snapshot(&fixture::sprite_color_animation_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_color_animation.png");
}

/// Spin shears the figure: the centre pinned, the rim carrying the full turn.
#[test]
fn sprite_spin() {
    let image = render_snapshot(&fixture::sprite_spin_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_spin.png");
}

/// A radial animation run around the angle, which deforms the outline into
/// petals rather than scaling the whole figure.
#[test]
fn sprite_radial_animation() {
    let image = render_snapshot(&fixture::sprite_radial_animation_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_radial_animation.png");
}

/// A position animation run along the figure, which shears it continuously
/// rather than translating it as a whole.
#[test]
fn sprite_position_animation() {
    let image = render_snapshot(
        &fixture::sprite_position_animation_snapshot(),
        &test_config(),
    );
    compare_fill_to_fixture(&image, "sprite_position_animation.png");
}

/// The same shear on a stroked figure, which reaches a point through the
/// contour it was offset from rather than through the refined mesh.
#[test]
fn sprite_position_animation_outline() {
    let image = render_snapshot(
        &fixture::sprite_position_animation_outline_snapshot(),
        &test_config(),
    );
    compare_fill_to_fixture(&image, "sprite_position_animation_outline.png");
}

/// A noise warp on an outline, which is the only waveform whose value depends
/// on which vertex it is asked about rather than only on where that vertex
/// sits on the figure.
#[test]
fn sprite_noise_warp_outline() {
    let image = render_snapshot(
        &fixture::sprite_noise_warp_outline_snapshot(),
        &test_config(),
    );
    compare_fill_to_fixture(&image, "sprite_noise_warp_outline.png");
}

/// A masked figure over a lit one intersects their apertures, which is how
/// gobo stacking already works for segments.
#[test]
fn sprite_masked_stack() {
    let image = render_snapshot(&fixture::sprite_masked_stack_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_masked_stack.png");
}

/// A gobo over a lit figure is the mask above inverted: the lit figure
/// survives inside the upper figure and is blacked everywhere else, where the
/// mask blacks the inside and leaves the rest.
#[test]
fn sprite_gobo_stack() {
    let image = render_snapshot(&fixture::sprite_gobo_stack_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_gobo_stack.png");
}

/// A gobo blacks the frame rather than clipping it, so a beam drawn after one
/// lands whole rather than being confined to the window.
#[test]
fn sprite_gobo_then_lit() {
    let image = render_snapshot(&fixture::sprite_gobo_then_lit_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_gobo_then_lit.png");
}

/// A look drawn as a gobo makes each of its channels a gobo in its own right,
/// and what survives all of them is what lies inside all of them.
#[test]
fn look_gobo_intersection() {
    let image = render_snapshot(&fixture::look_gobo_intersection_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "look_gobo_intersection.png");
}

/// A gobo whose window has closed blacks the frame entire.
///
/// Two ways for a window to close and both arrive here the same way. A
/// thickness animation winds the segments down until they tessellate to
/// nothing, which leaves the shapes in place carrying a thickness of zero;
/// blacking takes every segment away instead, which leaves no shapes at all.
/// Neither marks the stencil, so the ground goes down over everything.
///
/// A channel the operator has taken off its upfader is a different thing and
/// never reaches a renderer, because a channel at zero level emits no layer.
#[test]
fn gobo_closed_aperture() {
    let lit = || {
        default_layer(
            1.0,
            vec![
                test_arc(0.0, 0.0, 0.2),
                test_arc(0.0, 0.33, 0.35),
                test_arc(0.0, 0.66, 0.45),
            ],
        )
    };
    let thinned = |thickness| {
        let mut shapes = vec![test_arc(0.0, 0.0, 0.2), test_arc(0.0, 0.66, 0.45)];
        for shape in &mut shapes {
            shape.thickness = thickness;
        }
        shapes
    };

    // The lit rings underneath are what makes an all-black frame mean the
    // ground went down over them rather than nothing having been drawn, and
    // the assertion is spelt out as well as pinned so that the test cannot
    // come to rest on a picture that is merely stable.
    let closed_by_thickness = vec![lit(), layer_in_mode(PaintMode::Gobo, 1.0, thinned(0.0))];
    let image = render_snapshot(&closed_by_thickness, &test_config());
    assert_uniformly_black(&image, "a gobo thinned to nothing");

    let closed_by_blacking = vec![lit(), layer_in_mode(PaintMode::Gobo, 1.0, Vec::new())];
    assert_uniformly_black(
        &render_snapshot(&closed_by_blacking, &test_config()),
        "a gobo with every segment blacked",
    );

    // A window still open is the control: the same gobo at a thickness that
    // draws leaves some of the rings under it alight, so the black above is
    // the aperture closing and not the fixture failing to light anything.
    let open = vec![lit(), layer_in_mode(PaintMode::Gobo, 1.0, thinned(0.1))];
    let open = render_snapshot(&open, &test_config());
    assert!(
        open.pixels().any(|px| px.0 != [0, 0, 0, 255]),
        "a gobo with an open window blacked the whole frame"
    );

    compare_to_fixture(&image, "gobo_closed_aperture.png");
}

/// Assert every pixel is opaque black, naming what was drawn if one is not.
fn assert_uniformly_black(image: &image::RgbaImage, what: &str) {
    for (x, y, px) in image.enumerate_pixels() {
        assert_eq!(px.0, [0, 0, 0, 255], "{what} left {:?} at ({x}, {y})", px.0);
    }
}

/// A gobo's ground goes down at the layer's level, the shut aperture included.
///
/// A gobo blacked down to no shapes has none left to carry an alpha, so the
/// level rides on the layer itself. Part way up the fader the shut aperture
/// dims the frame instead of putting it out, and it is the top of the fader
/// that blacks it outright — a blackout stays available at the end of the
/// travel rather than being traded away for the fade.
#[test]
fn a_closed_gobo_shuts_as_far_as_its_fader_is_up() {
    let lit = || {
        default_layer(
            1.0,
            vec![
                test_arc(0.0, 0.0, 0.2),
                test_arc(0.0, 0.33, 0.35),
                test_arc(0.0, 0.66, 0.45),
            ],
        )
    };
    let shut = |level| {
        render_snapshot(
            &vec![
                lit(),
                layer_in_mode_at(PaintMode::Gobo, level, 1.0, Vec::new()),
            ],
            &test_config(),
        )
    };

    let open = shut(0.0);
    let half = shut(0.5);
    let closed = shut(1.0);
    assert_uniformly_black(&closed, "a gobo shut at the top of its fader");
    assert!(
        total_light(&half) < total_light(&open),
        "a gobo shut half way up its fader dimmed nothing"
    );
    assert!(
        total_light(&half) > total_light(&closed),
        "a gobo shut half way up its fader blacked the frame outright"
    );

    compare_to_fixture(&half, "gobo_closed_aperture_half_level.png");
}

/// A mask fades what it covers in over its fader rather than switching it out.
///
/// The bottom of the fader leaves the figure under it untouched, the top hides
/// it, and half way is between the two. Luminance falling strictly across the
/// three is what says the level reached the black the mask paints.
#[test]
fn a_mask_fades_in_over_its_fader() {
    let cfg = test_config();
    let masked = |level| {
        render_snapshot(
            &fixture::sprite_mask_at_level_snapshot(UnipolarFloat::new(level)),
            &cfg,
        )
    };

    let half = masked(0.5);
    let (off, half_light, full) = (
        total_light(&masked(0.0)),
        total_light(&half),
        total_light(&masked(1.0)),
    );
    assert!(off > 0, "the figure under the mask was never lit");
    assert!(
        full < half_light && half_light < off,
        "a mask did not fade over its fader: {off} unmasked, {half_light} half way, {full} at the top"
    );

    compare_fill_to_fixture(&half, "sprite_mask_half_level.png");
}

/// A gobo fades in over its fader the way a mask does, on the other side of
/// the outline.
///
/// What the window does not reach dims rather than goes out, and what it does
/// reach is untouched at every level — a gobo half way up its fader is the
/// figure under it with everything outside the window knocked back.
#[test]
fn a_gobo_fades_in_over_its_fader() {
    let cfg = test_config();
    let gobo = |level| {
        render_snapshot(
            &fixture::sprite_gobo_at_level_snapshot(UnipolarFloat::new(level)),
            &cfg,
        )
    };

    let half = gobo(0.5);
    let (off, half_light, full) = (
        total_light(&gobo(0.0)),
        total_light(&half),
        total_light(&gobo(1.0)),
    );
    assert!(off > 0, "the figure under the gobo was never lit");
    assert!(
        full < half_light && half_light < off,
        "a gobo did not fade over its fader: {off} open, {half_light} half way, {full} at the top"
    );

    compare_fill_to_fixture(&half, "sprite_gobo_half_level.png");
}

/// How much light a render carries, summed over every colour channel.
fn total_light(image: &image::RgbaImage) -> u64 {
    image
        .pixels()
        .map(|px| u64::from(px.0[0]) + u64::from(px.0[1]) + u64::from(px.0[2]))
        .sum()
}

/// A stroked outline takes its colour from the contour it follows, not from
/// where each offset vertex landed, so the colour is constant across the
/// ribbon's width and a long straight stroke does not band along its length.
#[test]
fn sprite_outline_color() {
    let image = render_snapshot(&fixture::sprite_outline_color_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_outline_color.png");
}

/// Outline mode strokes the contours the build ships instead of filling them,
/// which is what baking contours rather than triangles buys.
#[test]
fn sprite_outline() {
    let image = render_snapshot(&fixture::sprite_outline_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "sprite_outline.png");
}

/// A thickness animation's periodicity has to reach the outline.
///
/// Periodicity is how many times a waveform runs across the figure, so two
/// settings of it describe two different outlines and cannot draw the same
/// picture. A thickness resolved to one number before the layer is built
/// draws the same picture for every setting — and, for a sine, the same
/// picture as no animation at all, since the value at the start of the cycle
/// is zero.
#[test]
fn periodicity_varies_a_thickness_animation_around_the_figure() {
    let cfg = test_config();
    let once = render_snapshot(&fixture::sprite_thickness_animation_snapshot(1), &cfg);
    let thrice = render_snapshot(&fixture::sprite_thickness_animation_snapshot(3), &cfg);
    assert!(
        lit_pixels(&once) > 0 && lit_pixels(&thrice) > 0,
        "a tapered outline drew nothing at all, so there is nothing to compare"
    );
    assert!(
        once != thrice,
        "one period and three drew the same outline, so periodicity reached nothing"
    );
}

/// How many pixels a render leaves lit against its black ground.
fn lit_pixels(image: &image::RgbaImage) -> usize {
    image
        .pixels()
        .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
        .count()
}

/// The golden images below draw these three generated figures. A figure is a
/// family and two numbers, and what those numbers reach is the arity model's
/// business — so naming here what each of them selects is what turns a change
/// to that model into a failing test rather than a golden image quietly
/// becoming a picture of something else.
///
/// What is named is the selection: the count a family is built around and the
/// choice its secondary makes. The widths and angles that follow from those
/// are the arity model's own, and a change to one shows up as a golden that
/// has moved rather than as a figure that is no longer the same figure.
#[test]
fn the_generated_fixtures_select_the_figures_they_were_taken_of() {
    use tunnels_shapes::ShapeParams;
    use tunnels_shapes::families::MoireKind;
    use tunnels_shapes::geom::{CENTER, EXTENT};

    let figure = |id: tunnels_model::layer::GeneratedId| id.family.resolve(id.arity, id.secondary);

    match figure(fixture::generated_star()) {
        ShapeParams::StarPolygon(p) => assert_eq!(
            (p.points, p.step),
            (7, 3),
            "the star fixture selects another star"
        ),
        other => panic!("the star fixture is no longer a star polygon: {other:?}"),
    }

    match figure(fixture::generated_moire()) {
        ShapeParams::MoireBars(p) => assert_eq!(
            (p.kind, p.bars),
            (MoireKind::Grid, 8),
            "the moire fixture selects another pair of bar sets"
        ),
        other => panic!("the moire fixture is no longer a pair of bar sets: {other:?}"),
    }

    let lattice = fixture::generated_lattice();
    match figure(lattice) {
        ShapeParams::StarLattice(p) => assert_eq!(
            (p.tiles, p.reach),
            (1, 0.85),
            "the lattice fixture selects another lattice"
        ),
        other => panic!("the lattice fixture is no longer a star lattice: {other:?}"),
    }

    // The lattice is the figure the fixed map is read against, so how far it
    // runs outside its own frame is part of what its golden shows.
    let reach = figure(lattice)
        .generate()
        .contours
        .iter()
        .flat_map(|c| c.points())
        .fold(0.0f64, |acc, p| {
            acc.max((p.x - CENTER).abs()).max((p.y - CENTER).abs())
        });
    assert!(
        reach > EXTENT,
        "the lattice reaches {reach}, which no longer runs outside its own frame"
    );
}

/// A generated figure in one colour, which is the path that skips the ramp
/// entirely and draws the tessellator's own triangles.
#[test]
fn generated_flat() {
    let image = render_snapshot(&fixture::generated_flat_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "generated_flat.png");
}

/// Two sets of parallel bars at a small angle to each other. Every crossing is
/// a lens-shaped hole, which is what an even-odd fill makes of two contours
/// overlapping — under a non-zero rule the crossings would fill solid and the
/// beat the figure is made of would be gone.
#[test]
fn generated_even_odd() {
    let image = render_snapshot(&fixture::generated_even_odd_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "generated_even_odd.png");
}

/// A figure that runs well outside the frame it was built in, drawn at one
/// fixed scale and cut by the viewport rather than by anything in the
/// geometry. Fitted to its own reach instead, it would be a small object in
/// the middle of the frame.
#[test]
fn generated_field() {
    let image = render_snapshot(&fixture::generated_field_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "generated_field.png");
}

/// A colour sweep across a generated figure, which is the meshed and ramped
/// path rather than the flat one.
#[test]
fn generated_color() {
    let image = render_snapshot(&fixture::generated_color_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "generated_color.png");
}

/// A generated figure's contours stroked instead of its interior filled.
#[test]
fn generated_outline() {
    let image = render_snapshot(&fixture::generated_outline_snapshot(), &test_config());
    compare_fill_to_fixture(&image, "generated_outline.png");
}

/// The rasteriser's per-vertex colour paths, which no golden image reaches
/// yet: a figure's colour is resolved against the ramp, and a second colour
/// axis — the thing per-vertex tint carries — has no control on it.
///
/// Checked directly rather than left until then, because a rasteriser that
/// interpolates wrongly would be discovered as a wrong golden image and read
/// as a bug in the renderer.
#[test]
fn the_rasteriser_interpolates_across_a_triangle() {
    use graphics::draw_state::DrawState;

    // A right triangle filling the buffer, red at the origin and blue at the
    // far corner along x.
    let tri = [[0.0, 0.0], [16.0, 0.0], [0.0, 16.0]];
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];

    let mut buffer = RenderBuffer::new(16, 16);
    buffer.clear_color([0.0, 0.0, 0.0, 1.0]);
    buffer.tri_list_c(&DrawState::default(), |f| {
        f(&tri, &[red, blue, red]);
    });
    let image = buffer.into_image();
    // Each corner is its own colour, and halfway along the edge between them
    // is halfway between the two.
    let channels = |x, y| {
        let p = image.get_pixel(x, y).0;
        (f64::from(p[0]) / 255.0, f64::from(p[2]) / 255.0)
    };
    let (r, b) = channels(1, 1);
    assert!(r > 0.9 && b < 0.1, "the red corner came out ({r}, {b})");
    let (r, b) = channels(14, 0);
    assert!(b > 0.85 && r < 0.15, "the blue corner came out ({r}, {b})");
    let (r, b) = channels(8, 0);
    assert!(
        (r - 0.5).abs() < 0.05 && (b - 0.5).abs() < 0.05,
        "the midpoint came out ({r}, {b}), not halfway"
    );

    // The same triangle sampling a two-texel texture, tinted per vertex.
    let mut texture = image::RgbaImage::new(2, 1);
    texture.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
    texture.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
    let texture = RenderBuffer::from_image(texture);

    let mut buffer = RenderBuffer::new(16, 16);
    buffer.clear_color([0.0, 0.0, 0.0, 1.0]);
    buffer.tri_list_uv_c(&DrawState::default(), &texture, |f| {
        f(
            &tri,
            &[[0.0, 0.5], [0.99, 0.5], [0.0, 0.5]],
            &[[1.0; 4], [1.0; 4], [1.0; 4]],
        );
    });
    let image = buffer.into_image();
    // Nearest sampling, so the first half of the run is white and the second
    // half is the green texel.
    assert_eq!(
        image.get_pixel(1, 1).0,
        [255, 255, 255, 255],
        "not the white texel"
    );
    assert_eq!(
        image.get_pixel(14, 0).0,
        [0, 255, 0, 255],
        "not the green texel"
    );
}
