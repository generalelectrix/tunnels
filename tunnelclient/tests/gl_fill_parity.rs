//! The shader fill path against the CPU one, on whatever GPU is running.
//!
//! Both paths draw the same scene into the same framebuffer, through the same
//! rasteriser, so the only difference between the two images is where the
//! per-vertex work happened. Nothing is stored: a GPU's own idea of an
//! arctangent or a bilinear weight cancels, because each machine compares its
//! output to its own.
//!
//! The golden images keep pinning the CPU path exactly, through the software
//! rasteriser, with no GPU involved. This is the other half of that: it says
//! the two paths agree, not what either one should produce.
//!
//! A GL context is required, and a machine with none skips loudly rather than
//! passing.

use client_lib::config::ClientConfig;
use graphics::Viewport;
use graphics::clear;
use opengl_graphics::{GlGraphics, OpenGL, Texture};
use tunnelclient::fill::Renderer;
use tunnels_model::layer::{LayerCollection, PhaseAxis};
use tunnels_model::tunnel::fixture;

const WIDTH: u32 = 512;
const HEIGHT: u32 = 512;

/// Per-channel difference two rasterisations of the same geometry may show.
///
/// The CPU path computes in `f64` and with an approximate arctangent; the
/// shader computes in `f32` with the driver's own. A colour is read from a
/// ramp, so a phase error shows up as a neighbouring texel.
const TOLERANCE: u8 = 2;

/// Pixels on a figure's silhouette allowed to differ by more than
/// [`TOLERANCE`].
///
/// **This budget covers one thing only: a sub-pixel vertex shift flipping a
/// pixel's coverage at the silhouette.** Both paths here go through the same
/// rasteriser, so nothing is owed to two rasterisers disagreeing — the reason
/// the golden-image comparison carries a budget does not apply. What is left is
/// arithmetic: the CPU path resolves a vertex in `f64` through
/// `fastmath::atan2`, the shader in `f32` through the driver's own, and where
/// that moves a vertex by a fraction of a pixel the edge it bounds lands on the
/// other side of a pixel centre. Against an unlit background that reads as a
/// difference of the full range, which is why a magnitude cannot tell it from a
/// real fault and position has to.
///
/// Measured on llvmpipe at this size: 59 pixels at worst, every one of them on
/// a boundary and none in an interior. **Apple's GL over Metal may put the
/// arithmetic somewhere else, and this is the number to revisit there** — a
/// figure's silhouette also grows with resolution, so it is tied to the size
/// this test draws at and not to a figure.
const BOUNDARY_BUDGET: usize = 200;

/// Pixels away from a silhouette allowed to differ by more than [`TOLERANCE`].
///
/// None. A phase, a displacement or a colour resolved differently moves the
/// inside of a figure, and there is nothing in the arithmetic that reaches
/// there — an interior pixel is the same lookup into the same ramp either way.
/// So one is a fault, and this is what makes the comparison say anything: a
/// budget that absorbed interior differences would absorb exactly the failures
/// worth catching.
const INTERIOR_BUDGET: usize = 0;

fn test_config() -> ClientConfig {
    let mut cfg = ClientConfig::new(
        0,
        "test".to_string(),
        (WIDTH, HEIGHT),
        false,
        false,
        None,
        false,
        false,
    );
    cfg.critical_size = f64::from(WIDTH.min(HEIGHT));
    cfg
}

/// A GL context, and the offscreen buffer a comparison is drawn into.
///
/// The backend is declared before the context because fields drop in
/// declaration order, and `GlGraphics` deletes programs and vertex arrays as it
/// goes: doing that after the context has been destroyed is a segmentation
/// fault at the end of an otherwise passing test.
struct Offscreen {
    gl: GlGraphics,
    fbo: u32,
    /// Held so the context outlives the drawing.
    _context: sdl2::video::GLContext,
    _window: sdl2::video::Window,
    _video: sdl2::VideoSubsystem,
    _sdl: sdl2::Sdl,
}

impl Offscreen {
    /// Open a context and an sRGB buffer to draw into.
    ///
    /// `None` where the machine has no GL at all, which is a reason to skip a
    /// comparison rather than to fail it.
    fn open() -> Option<Self> {
        let sdl = sdl2::init().ok()?;
        let video = sdl.video().ok()?;
        {
            let attr = video.gl_attr();
            attr.set_context_profile(sdl2::video::GLProfile::Core);
            attr.set_context_version(3, 2);
        }
        let window = video
            .window("fill parity", WIDTH, HEIGHT)
            .opengl()
            .hidden()
            .build()
            .ok()?;
        let context = window.gl_create_context().ok()?;
        window.gl_make_current(&context).ok()?;
        gl::load_with(|name| video.gl_get_proc_address(name) as *const _);
        if !gl::Enable::is_loaded() {
            return None;
        }
        // An sRGB attachment, because piston enables `FRAMEBUFFER_SRGB` for
        // every frame it draws and the ramp texture is stored as sRGB too.
        let (mut fbo, mut color) = (0, 0);
        unsafe {
            gl::GenTextures(1, &mut color);
            gl::BindTexture(gl::TEXTURE_2D, color);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::SRGB8_ALPHA8 as i32,
                WIDTH as i32,
                HEIGHT as i32,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                std::ptr::null(),
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::GenFramebuffers(1, &mut fbo);
            gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                color,
                0,
            );
            if gl::CheckFramebufferStatus(gl::FRAMEBUFFER) != gl::FRAMEBUFFER_COMPLETE {
                return None;
            }
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
        Some(Self {
            gl: GlGraphics::new(OpenGL::V3_2),
            fbo,
            _context: context,
            _window: window,
            _video: video,
            _sdl: sdl,
        })
    }

    /// Draw one scene and read the buffer back.
    fn render(
        &mut self,
        snapshot: &LayerCollection,
        cfg: &ClientConfig,
        shader_fill: bool,
    ) -> (image::RgbaImage, usize) {
        let mut renderer: Renderer<Texture> = Renderer::default();
        renderer.use_shader_fill(shader_fill);
        let viewport = Viewport {
            rect: [0, 0, WIDTH as i32, HEIGHT as i32],
            draw_size: [WIDTH, HEIGHT],
            window_size: [f64::from(WIDTH), f64::from(HEIGHT)],
        };
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.fbo);
        }
        let gl = &mut self.gl;
        gl.draw(viewport, |c, gl| {
            clear([0.0, 0.0, 0.0, 1.0], gl);
            renderer.draw(snapshot, &c, gl, cfg);
        });
        let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        unsafe {
            gl::Finish();
            gl::ReadPixels(
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
        }
        let image = image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels)
            .expect("the buffer is the right size");
        (image, renderer.shader_draws())
    }
}

/// Where two renderings of the same scene disagree.
#[derive(Default, PartialEq, Eq)]
struct Divergence {
    /// Differing pixels whose neighbourhood straddles a figure's silhouette,
    /// where a fraction of a pixel decides coverage.
    boundary: usize,
    /// Differing pixels away from any silhouette.
    interior: usize,
}

impl Divergence {
    /// Whether either count has passed its budget.
    fn over_budget(&self) -> bool {
        self.boundary > BOUNDARY_BUDGET || self.interior > INTERIOR_BUDGET
    }

    /// Whether the two renderings agree everywhere.
    fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} on a silhouette (budget {BOUNDARY_BUDGET}), {} inside one \
             (budget {INTERIOR_BUDGET})",
            self.boundary, self.interior
        )
    }
}

/// Whether a pixel carries any light.
fn lit(p: &image::Rgba<u8>) -> bool {
    p.0[..3].iter().any(|&c| c > 8)
}

/// How two renderings of the same scene differ, split by where.
///
/// A pixel counts as being on a silhouette when its own eight neighbours cover
/// both lit and unlit ground in either rendering, because that is exactly the
/// neighbourhood in which a fraction of a pixel decides which side of an edge
/// a sample falls.
fn diverge(a: &image::RgbaImage, b: &image::RgbaImage) -> Divergence {
    let (w, h) = a.dimensions();
    let mut out = Divergence::default();
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (a.get_pixel(x, y), b.get_pixel(x, y));
            if !p
                .0
                .iter()
                .zip(q.0.iter())
                .any(|(u, v)| u.abs_diff(*v) > TOLERANCE)
            {
                continue;
            }
            let mut sees_light = false;
            let mut sees_dark = false;
            for ny in y.saturating_sub(1)..(y + 2).min(h) {
                for nx in x.saturating_sub(1)..(x + 2).min(w) {
                    for image in [a, b] {
                        if lit(image.get_pixel(nx, ny)) {
                            sees_light = true;
                        } else {
                            sees_dark = true;
                        }
                    }
                }
            }
            if sees_light && sees_dark {
                out.boundary += 1;
            } else {
                out.interior += 1;
            }
        }
    }
    out
}

/// The shader against the CPU path, on every scene either of them answers.
///
/// One test rather than one per scene: a GL context belongs to the thread that
/// made it and tests run in parallel, so a second context in the same process
/// is a second driver's worth of state and a race for the one that is current.
#[test]
fn the_shader_draws_what_the_cpu_draws() {
    let Some(mut offscreen) = Offscreen::open() else {
        eprintln!(
            "SKIPPED: no OpenGL context on this machine, so the two fill paths \
             were not compared. This test says nothing where there is no GPU."
        );
        return;
    };
    let cfg = test_config();
    let mut failures = Vec::new();

    // Scenes the shader answers. A refined mesh, and a colour that either
    // varies across the figure or does not.
    let answered: [(&str, LayerCollection); 7] = [
        (
            "colour along the angle",
            fixture::sprite_color_snapshot(PhaseAxis::Angle),
        ),
        (
            "colour along the radius",
            fixture::sprite_color_snapshot(PhaseAxis::Radius),
        ),
        (
            "colour along the linear axis",
            fixture::sprite_color_snapshot(PhaseAxis::Linear),
        ),
        (
            "a colour animation",
            fixture::sprite_color_animation_snapshot(),
        ),
        ("spin", fixture::sprite_spin_snapshot()),
        (
            "a radial animation",
            fixture::sprite_radial_animation_snapshot(),
        ),
        (
            "a position animation",
            fixture::sprite_position_animation_snapshot(),
        ),
    ];
    for (name, snapshot) in &answered {
        let (cpu, cpu_draws) = offscreen.render(snapshot, &cfg, false);
        let (shader, shader_draws) = offscreen.render(snapshot, &cfg, true);
        dump(name, &cpu, &shader);
        if cpu_draws != 0 {
            failures.push(format!("{name}: the shader drew with the switch off"));
            continue;
        }
        // Two images that agree prove nothing if the shader never ran.
        if shader_draws == 0 {
            failures.push(format!(
                "{name}: the shader drew nothing, so this compares the CPU path \
                 against itself"
            ));
            continue;
        }
        let divergence = diverge(&cpu, &shader);
        let drawn = cpu.pixels().filter(|p| lit(p)).count();
        if drawn == 0 {
            failures.push(format!("{name}: nothing was drawn to compare"));
        } else if divergence.over_budget() {
            failures.push(format!(
                "{name}: differs by more than {TOLERANCE} in {divergence}, of \
                 {drawn} lit"
            ));
        } else {
            eprintln!("{name}: {divergence}, of {drawn} lit");
        }
    }

    // One colour everywhere and nothing displacing it is the path that skips
    // the refined mesh entirely and draws the tessellator's own triangles, so
    // there is no per-vertex pass for a shader to stand in for. Turning the
    // switch on has to leave it alone rather than routing it somewhere new.
    let unanswered: [(&str, LayerCollection); 2] = [
        ("one flat colour", fixture::sprite_flat_snapshot()),
        ("a shared vertex", fixture::sprite_shared_vertex_snapshot()),
    ];
    for (name, snapshot) in &unanswered {
        let (off, _) = offscreen.render(snapshot, &cfg, false);
        let (on, draws) = offscreen.render(snapshot, &cfg, true);
        if draws != 0 {
            failures.push(format!("{name}: the shader answered a layer it cannot"));
        }
        let divergence = diverge(&off, &on);
        if !divergence.is_none() {
            failures.push(format!("{name}: the switch moved {divergence}"));
        } else {
            eprintln!("{name}: untouched by the switch");
        }
    }

    // Layers composite in the order they were submitted, and a mask paints
    // opaque black over what is already there — so a stack says whether the
    // shader's own draws land in that order among piston's batched ones,
    // whichever way each layer of it happens to route. This is the failure the
    // whole bracket around a shader draw exists to prevent, and the only one
    // that shows up as a layer intermittently missing rather than as a wrong
    // colour.
    let stacked: [(&str, LayerCollection); 3] = [
        ("a masked stack", fixture::sprite_masked_stack_snapshot()),
        (
            "a coloured outline",
            fixture::sprite_outline_color_snapshot(),
        ),
        ("an outline", fixture::sprite_outline_snapshot()),
    ];
    for (name, snapshot) in &stacked {
        let (off, _) = offscreen.render(snapshot, &cfg, false);
        let (on, draws) = offscreen.render(snapshot, &cfg, true);
        let divergence = diverge(&off, &on);
        dump(name, &off, &on);
        if divergence.over_budget() {
            failures.push(format!(
                "{name}: the switch moved {divergence}, {draws} shader draws"
            ));
        } else {
            eprintln!("{name}: {divergence}, {draws} shader draws");
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Save both images where the environment asks for them.
fn dump(name: &str, cpu: &image::RgbaImage, shader: &image::RgbaImage) {
    let Some(dir) = std::env::var_os("PARITY_DUMP") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let slug: String = name
        .chars()
        .map(|c| if c == ' ' { '_' } else { c })
        .collect();
    cpu.save(dir.join(format!("{slug}-cpu.png"))).ok();
    shader.save(dir.join(format!("{slug}-gpu.png"))).ok();
}
