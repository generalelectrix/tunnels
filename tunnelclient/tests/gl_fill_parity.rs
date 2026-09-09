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

/// Pixels allowed to differ by more than [`TOLERANCE`].
///
/// A phase error at a colour discontinuity moves the edge between two
/// neighbouring texels of the ramp, and a figure has tens of thousands of
/// triangles for that edge to cross. Budgeted like the golden-image fill
/// comparison, for the same reason.
const BUDGET: usize = 200;

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

/// How many pixels of two images differ by more than [`TOLERANCE`].
fn mismatches(a: &image::RgbaImage, b: &image::RgbaImage) -> usize {
    a.pixels()
        .zip(b.pixels())
        .filter(|(p, q)| {
            p.0.iter()
                .zip(q.0.iter())
                .any(|(x, y)| x.abs_diff(*y) > TOLERANCE)
        })
        .count()
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
        let differing = mismatches(&cpu, &shader);
        let lit = cpu
            .pixels()
            .filter(|p| p.0[..3].iter().any(|&c| c > 8))
            .count();
        if lit == 0 {
            failures.push(format!("{name}: nothing was drawn to compare"));
        } else if differing > BUDGET {
            failures.push(format!(
                "{name}: {differing} pixels differ by more than {TOLERANCE} \
                 (budget {BUDGET}, {lit} lit)"
            ));
        } else {
            eprintln!("{name}: {differing} of {lit} lit pixels differ");
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
        let differing = mismatches(&off, &on);
        if differing != 0 {
            failures.push(format!("{name}: the switch moved {differing} pixels"));
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
        let differing = mismatches(&off, &on);
        dump(name, &off, &on);
        if differing > BUDGET {
            failures.push(format!(
                "{name}: {differing} pixels differ by more than {TOLERANCE} \
                 with the switch on (budget {BUDGET}, {draws} shader draws)"
            ));
        } else {
            eprintln!("{name}: {differing} pixels differ, {draws} shader draws");
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
