//! The fill, drawn by a shader instead of by the CPU.
//!
//! The CPU path pays for a mesh twice a frame: once walking it to displace
//! vertices and work out where each one looks in the ramp, and again projecting
//! every vertex into the backend's buffers, because `tri_list_uv` takes
//! pre-transformed vertices. Both are things a vertex shader does for free.
//! Here a refined mesh is uploaded once and never touched again, the transform
//! and the animation parameters are uniforms, and the fragment shader samples
//! the same ramp texture the CPU path samples.
//!
//! Targets GLSL 1.50, since OpenGL 3.2 core is the ceiling on the render
//! clients — no compute stage, no storage buffer, no `layout` qualifiers.
//!
//! # Sharing a context with piston
//!
//! `GlGraphics` batches vertices and caches the program and the draw state it
//! last bound. Issuing GL calls while it holds that cache corrupts the batch,
//! and the symptom is a layer that intermittently fails to draw. So a draw here
//! is bracketed: piston's batch is flushed first, the blend state the CPU path
//! composites under is asked for explicitly rather than inherited, and its
//! caches are invalidated afterwards so it rebinds from scratch rather than
//! trusting what it remembers. See [`Boundary`].

use super::Frame;
use super::geom::IndexBatch;
use super::mesh::{MeshId, RefinedMesh};
use gl::types::{GLchar, GLenum, GLint, GLsizeiptr, GLuint};
use graphics::Graphics;
use graphics::color::gamma_srgb_to_linear;
use graphics::draw_state::DrawState;
use graphics::math::Matrix2d;
use log::{error, info};
use opengl_graphics::{GlGraphics, Texture};
use std::collections::HashMap;
use tunnels_model::animation::{PreparedAnimation, Waveform};
use tunnels_model::animation_target::AnimationTarget;
use tunnels_model::layer::PhaseAxis;
use tunnels_model::tunnel::N_ANIM;

const VERTEX_SRC: &str = include_str!("shaders/fill.vert");
const FRAGMENT_SRC: &str = include_str!("shaders/fill.frag");

// An array size in GLSL has to be a constant expression written in the shader
// source, so the slot count is spelled out in `fill.vert` and cannot be derived
// from the model's. It can at least be made impossible to change only there.
const _: () = assert!(
    N_ANIM == 4,
    "fill.vert declares MAX_WAVES as a literal; change it alongside N_ANIM"
);

/// Bytes one vertex of the fill buffer occupies: the figure-space point and the
/// whole turns that put it on its triangle's angular branch.
const VERTEX_STRIDE: i32 = 12;

/// How long a buffer nothing has drawn is kept, in frames.
///
/// Matched to what a refined mesh is kept for, because the two hold the same
/// figure at the same density and one is built from the other: a buffer that
/// outlived its mesh would be rebuilt from a mesh that had to be refined again,
/// and a mesh that outlived its buffer would be re-uploaded on the frame it was
/// next drawn.
const MAX_AGE: u64 = 600;

/// Frames between sweeps of the buffers.
const REAP_INTERVAL: u64 = 120;

/// What a slot drives, as the shader numbers it. Zero is a slot the layer does
/// not fill, or one whose target a figure's geometry does not answer.
const TARGET_NONE: i32 = 0;
const TARGET_SIZE: i32 = 1;
const TARGET_SPIN: i32 = 2;
const TARGET_ASPECT: i32 = 3;
const TARGET_POSITION_X: i32 = 4;
const TARGET_POSITION_Y: i32 = 5;

const WAVE_SINE: i32 = 0;
const WAVE_TRIANGLE: i32 = 1;
const WAVE_SQUARE: i32 = 2;
const WAVE_SAWTOOTH: i32 = 3;
const WAVE_CONSTANT: i32 = 4;

const FLAG_PULSE: i32 = 1;
const FLAG_STANDING: i32 = 2;
const FLAG_INVERT: i32 = 4;

/// The coordinate a phase is read along, as the shader numbers it.
pub fn phase_code(axis: PhaseAxis) -> i32 {
    match axis {
        PhaseAxis::Angle => 0,
        PhaseAxis::Radius => 1,
        PhaseAxis::Linear => 2,
    }
}

/// The waveform, as the shader numbers it.
///
/// `None` for a waveform the shader does not carry. Noise reads a field no
/// uniform can hold.
pub fn waveform_code(waveform: Waveform) -> Option<i32> {
    match waveform {
        Waveform::Sine => Some(WAVE_SINE),
        Waveform::Triangle => Some(WAVE_TRIANGLE),
        Waveform::Square => Some(WAVE_SQUARE),
        Waveform::Sawtooth => Some(WAVE_SAWTOOTH),
        Waveform::Constant => Some(WAVE_CONSTANT),
        Waveform::Noise => None,
    }
}

/// What a slot drives, as the shader numbers it.
///
/// `None` for a target a figure's geometry does not answer: a colour is
/// resolved into the ramp, a thickness reaches an outline alone, a rotation is
/// folded into the placement, and a marquee resolves into nothing on a figure
/// that has no segments to slide along a path.
fn target_code(target: AnimationTarget) -> Option<i32> {
    match target {
        AnimationTarget::Size => Some(TARGET_SIZE),
        AnimationTarget::Spin => Some(TARGET_SPIN),
        AnimationTarget::AspectRatio => Some(TARGET_ASPECT),
        AnimationTarget::PositionX => Some(TARGET_POSITION_X),
        AnimationTarget::PositionY => Some(TARGET_POSITION_Y),
        AnimationTarget::Color
        | AnimationTarget::ColorSpread
        | AnimationTarget::ColorSaturation
        | AnimationTarget::Rotation
        | AnimationTarget::Thickness
        | AnimationTarget::MarqueeRotation => None,
    }
}

/// One animation slot, in the numbers the shader reads.
#[derive(Copy, Clone)]
pub struct Wave {
    target: i32,
    form: i32,
    flags: i32,
    periods: f32,
    duty: f32,
    smoothing: f32,
    temporal: f32,
    scale: f32,
}

impl Default for Wave {
    /// A slot that contributes nothing.
    fn default() -> Self {
        Self {
            target: TARGET_NONE,
            form: WAVE_CONSTANT,
            flags: 0,
            periods: 0.0,
            duty: 0.0,
            smoothing: 0.0,
            temporal: 0.0,
            scale: 0.0,
        }
    }
}

impl Wave {
    /// The slot an animation fills, with everything it resolved to.
    ///
    /// `None` where the shader cannot stand in for the animation, which is the
    /// signal to draw the layer on the CPU instead. An animation the geometry
    /// does not answer, or one contributing nothing, is not a refusal: it fills
    /// no slot and the rest of the layer still reaches the shader.
    pub fn of(
        target: AnimationTarget,
        animation: &PreparedAnimation,
    ) -> Result<Option<Self>, Unsupported> {
        let params = animation.static_params();
        let Some(form) = waveform_code(params.waveform) else {
            return Err(Unsupported);
        };
        if !animation.is_active() {
            return Ok(None);
        }
        let Some(target) = target_code(target) else {
            return Ok(None);
        };
        let mut flags = 0;
        if params.pulse {
            flags |= FLAG_PULSE;
        }
        if params.standing {
            flags |= FLAG_STANDING;
        }
        if params.invert {
            flags |= FLAG_INVERT;
        }
        Ok(Some(Self {
            target,
            form,
            flags,
            periods: f32::from(params.n_periods),
            duty: params.duty_cycle.val() as f32,
            smoothing: animation.smoothing().val() as f32,
            temporal: animation.phase_temporal().val() as f32,
            scale: animation.scale() as f32,
        }))
    }
}

/// A layer the shader cannot stand in for.
#[derive(Debug)]
pub struct Unsupported;

/// Everything one draw through the shader varies.
pub struct Uniforms {
    /// The rows of the affine transform. It carries the viewport, so its
    /// output is in clip space.
    pub xform: Matrix2d,
    pub phase: i32,
    pub ramp_scale: f32,
    /// The period the baked branch shift is measured in, or zero where the
    /// coordinate does not wrap.
    pub unwrap: f32,
    pub spin_speed: f32,
    pub rotates: bool,
    pub needs_angle: bool,
    pub needs_radius: bool,
    /// The offset a figure's placement carries, which a point's own offset is
    /// measured from.
    pub anchor: [f32; 2],
    /// The one colour a figure takes where nothing varies across it, in place
    /// of a ramp to read.
    pub flat: Option<[f32; 4]>,
    pub waves: [Wave; N_ANIM],
}

/// The linked program and where its uniforms live.
struct Program {
    program: GLuint,
    loc: Locations,
}

struct Locations {
    xform0: GLint,
    xform1: GLint,
    phase: GLint,
    ramp_scale: GLint,
    unwrap: GLint,
    spin_speed: GLint,
    rotates: GLint,
    needs_angle: GLint,
    needs_radius: GLint,
    anchor: GLint,
    ramp: GLint,
    flat: GLint,
    color: GLint,
    wave_target: GLint,
    wave_form: GLint,
    wave_flags: GLint,
    wave_periods: GLint,
    wave_duty: GLint,
    wave_smoothing: GLint,
    wave_temporal: GLint,
    wave_scale: GLint,
}

impl Locations {
    fn of(program: GLuint) -> Self {
        let at = |name: &str| {
            let c = std::ffi::CString::new(name).unwrap_or_default();
            unsafe { gl::GetUniformLocation(program, c.as_ptr()) }
        };
        Self {
            xform0: at("u_xform0"),
            xform1: at("u_xform1"),
            phase: at("u_phase"),
            ramp_scale: at("u_ramp_scale"),
            unwrap: at("u_unwrap"),
            spin_speed: at("u_spin_speed"),
            rotates: at("u_rotates"),
            needs_angle: at("u_needs_angle"),
            needs_radius: at("u_needs_radius"),
            anchor: at("u_anchor"),
            ramp: at("u_ramp"),
            flat: at("u_flat"),
            color: at("u_color"),
            wave_target: at("u_wave_target"),
            wave_form: at("u_wave_form"),
            wave_flags: at("u_wave_flags"),
            wave_periods: at("u_wave_periods"),
            wave_duty: at("u_wave_duty"),
            wave_smoothing: at("u_wave_smoothing"),
            wave_temporal: at("u_wave_temporal"),
            wave_scale: at("u_wave_scale"),
        }
    }
}

/// One mesh resident on the GPU, and when a draw last wanted it.
struct Held {
    vao: GLuint,
    vbo: GLuint,
    ibo: GLuint,
    index_count: GLint,
    last_used: Frame,
}

impl Held {
    /// Give the buffers back to the driver.
    fn release(&self) {
        unsafe {
            gl::DeleteVertexArrays(1, &self.vao);
            let buffers = [self.vbo, self.ibo];
            gl::DeleteBuffers(2, buffers.as_ptr());
        }
    }
}

/// Whether the program has been asked for yet, and what came back.
#[derive(Default)]
enum Compiled {
    /// Not asked for. A context may not exist yet.
    #[default]
    Untried,
    Ready(Program),
    /// The driver refused it. A shader it will not take should cost the show a
    /// fallback to the CPU path, not a second attempt every frame.
    Refused,
}

/// The shader fill path's program and its resident meshes.
///
/// Constructing this touches no GL, so it costs nothing where there is no
/// context and nothing ever asks it to draw. The program is compiled on the
/// first draw.
///
/// Buffers are returned to the driver by [`reap`](Self::reap) and nowhere else.
/// Dropping this does not release them, deliberately: a context is commonly
/// torn down before the things drawn through it, and a GL call made after that
/// is a segmentation fault rather than an error. What a process still holds at
/// exit is the driver's to reclaim.
#[derive(Default)]
pub struct GpuFill {
    program: Compiled,
    meshes: HashMap<MeshId, Held>,
    reaped: Frame,
    shaded: usize,
}

impl GpuFill {
    /// Meshes resident on the GPU.
    pub fn resident(&self) -> usize {
        self.meshes.len()
    }

    /// Draws the shader has answered since it was built.
    pub fn shaded(&self) -> usize {
        self.shaded
    }

    /// Drop the buffers nothing has drawn for [`MAX_AGE`], every
    /// [`REAP_INTERVAL`] frames.
    pub fn reap(&mut self, frame: Frame) {
        if frame.since(self.reaped) < REAP_INTERVAL {
            return;
        }
        self.reaped = frame;
        let before = self.meshes.len();
        self.meshes.retain(|_, held| {
            let keep = frame.since(held.last_used) < MAX_AGE;
            if !keep {
                held.release();
            }
            keep
        });
        if self.meshes.len() < before {
            info!(
                "Released {} figure vertex buffers; {} held.",
                before - self.meshes.len(),
                self.meshes.len()
            );
        }
    }

    /// Draw one refined mesh through the shader.
    ///
    /// Answers whether the draw happened. It does not where the driver refused
    /// the program, which is a reason to draw the layer on the CPU rather than
    /// to stop the show.
    ///
    /// `gl` is borrowed to flush and invalidate piston's batch around the draw,
    /// not to draw through.
    fn draw(
        &mut self,
        gl: &mut GlGraphics,
        id: MeshId,
        mesh: &RefinedMesh,
        frame: Frame,
        uniforms: &Uniforms,
        ramp: Option<&Texture>,
    ) -> bool {
        // Building a buffer binds a vertex array and an element buffer, so it
        // belongs inside the bracket alongside the draw rather than in front.
        let _boundary = Boundary::enter(gl);

        if matches!(self.program, Compiled::Untried) {
            self.program = match build_program() {
                Ok(program) => Compiled::Ready(program),
                Err(e) => {
                    error!("Could not build the fill shader: {e}");
                    Compiled::Refused
                }
            };
        }
        let Compiled::Ready(program) = &self.program else {
            return false;
        };

        let held = self.meshes.entry(id).or_insert_with(|| upload(mesh, frame));
        held.last_used = frame;
        if held.index_count == 0 {
            return true;
        }

        unsafe {
            gl::UseProgram(program.program);
            set_uniforms(&program.loc, uniforms);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, ramp.map_or(0, Texture::get_id));
            // Piston draws both windings, and a tessellated fill contains both.
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(held.vao);
            gl::DrawElements(
                gl::TRIANGLES,
                held.index_count,
                gl::UNSIGNED_INT,
                std::ptr::null(),
            );
            gl::BindVertexArray(0);
        }
        self.shaded += 1;
        true
    }
}

/// Push one draw's uniforms into the program, which must be in use.
unsafe fn set_uniforms(loc: &Locations, u: &Uniforms) {
    let row = |r: [f64; 3]| [r[0] as f32, r[1] as f32, r[2] as f32];
    let r0 = row(u.xform[0]);
    let r1 = row(u.xform[1]);
    let n = N_ANIM as i32;
    let targets: [i32; N_ANIM] = u.waves.map(|w| w.target);
    let forms: [i32; N_ANIM] = u.waves.map(|w| w.form);
    let flags: [i32; N_ANIM] = u.waves.map(|w| w.flags);
    let periods: [f32; N_ANIM] = u.waves.map(|w| w.periods);
    let duty: [f32; N_ANIM] = u.waves.map(|w| w.duty);
    let smoothing: [f32; N_ANIM] = u.waves.map(|w| w.smoothing);
    let temporal: [f32; N_ANIM] = u.waves.map(|w| w.temporal);
    let scale: [f32; N_ANIM] = u.waves.map(|w| w.scale);
    unsafe {
        gl::Uniform3fv(loc.xform0, 1, r0.as_ptr());
        gl::Uniform3fv(loc.xform1, 1, r1.as_ptr());
        gl::Uniform1i(loc.phase, u.phase);
        gl::Uniform1f(loc.ramp_scale, u.ramp_scale);
        gl::Uniform1f(loc.unwrap, u.unwrap);
        gl::Uniform1f(loc.spin_speed, u.spin_speed);
        gl::Uniform1i(loc.rotates, i32::from(u.rotates));
        gl::Uniform1i(loc.needs_angle, i32::from(u.needs_angle));
        gl::Uniform1i(loc.needs_radius, i32::from(u.needs_radius));
        gl::Uniform2fv(loc.anchor, 1, u.anchor.as_ptr());
        // Piston never sets its own sampler uniforms, so it relies on unit zero
        // being the active one; staying on it is what keeps that true.
        gl::Uniform1i(loc.ramp, 0);
        match u.flat {
            Some(color) => {
                // Converted here because piston converts a colour handed to
                // its triangle list, and a figure drawn either way has to
                // reach the framebuffer as the same colour.
                let color = gamma_srgb_to_linear(color);
                gl::Uniform1i(loc.flat, 1);
                gl::Uniform4fv(loc.color, 1, color.as_ptr());
            }
            None => gl::Uniform1i(loc.flat, 0),
        }
        gl::Uniform1iv(loc.wave_target, n, targets.as_ptr());
        gl::Uniform1iv(loc.wave_form, n, forms.as_ptr());
        gl::Uniform1iv(loc.wave_flags, n, flags.as_ptr());
        gl::Uniform1fv(loc.wave_periods, n, periods.as_ptr());
        gl::Uniform1fv(loc.wave_duty, n, duty.as_ptr());
        gl::Uniform1fv(loc.wave_smoothing, n, smoothing.as_ptr());
        gl::Uniform1fv(loc.wave_temporal, n, temporal.as_ptr());
        gl::Uniform1fv(loc.wave_scale, n, scale.as_ptr());
    }
}

/// A backend the shader fill path can draw through.
pub trait FillBackend: Graphics {
    /// Draw a refined mesh through the fill shader.
    ///
    /// Answers whether the draw happened. A backend with no shader path answers
    /// false, and what asked walks the mesh itself.
    fn shader_fill(
        &mut self,
        _gpu: &mut GpuFill,
        _id: MeshId,
        _mesh: &RefinedMesh,
        _frame: Frame,
        _uniforms: &Uniforms,
        _ramp: Option<&Self::Texture>,
    ) -> bool {
        false
    }
}

impl FillBackend for GlGraphics {
    fn shader_fill(
        &mut self,
        gpu: &mut GpuFill,
        id: MeshId,
        mesh: &RefinedMesh,
        frame: Frame,
        uniforms: &Uniforms,
        ramp: Option<&Texture>,
    ) -> bool {
        gpu.draw(self, id, mesh, frame, uniforms, ramp)
    }
}

/// A clean boundary between piston's batch and ours.
///
/// Entering flushes whatever piston has queued, so nothing of ours lands in the
/// middle of it and the drawing order across the two paths is the order the
/// layers were submitted in.
///
/// Entering also asks for the draw state the CPU path draws every layer under.
/// Piston establishes that on its own first draw of a frame and not before, so
/// a frame drawn entirely through this pass would never give it the chance, and
/// blending would be left wherever the previous frame happened to leave it —
/// off, on the first frame of all. Going through `use_draw_state` rather than
/// raw GL is what keeps piston's own record of it true.
///
/// Leaving drops the caches `GlGraphics` keeps of the program and the draw
/// state it last bound. The program is genuinely stale, because it is ours. The
/// draw state is not, but dropping it means the next pass through here rebinds
/// blend, scissor and stencil unconditionally rather than trusting a record of
/// them, which costs three state calls a draw and removes the entire class of
/// bug where either side skips a change it believes is in effect. The symptom
/// that buys is a layer that intermittently fails to draw, so the trade is not
/// close.
///
/// The rest of the state this pass touches it restores itself: the vertex array
/// binding goes back to zero, the array buffer binding to zero, the texture
/// binding to zero, the active texture unit is never moved off zero, and face
/// culling is left disabled — which is where every piston flush leaves it too.
struct Boundary<'a> {
    gl: &'a mut GlGraphics,
}

impl<'a> Boundary<'a> {
    fn enter(gl: &'a mut GlGraphics) -> Self {
        gl.draw_end();
        gl.use_draw_state(&DrawState::default());
        Self { gl }
    }
}

impl Drop for Boundary<'_> {
    fn drop(&mut self) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        }
        self.gl.clear_program();
        self.gl.clear_draw_state();
    }
}

/// Build the vertex and index buffers for a mesh and hand them to the driver.
fn upload(mesh: &RefinedMesh, frame: Frame) -> Held {
    let (verts, indices) = unwrapped(mesh);
    let mut vao = 0;
    let mut vbo = 0;
    let mut ibo = 0;
    unsafe {
        gl::GenVertexArrays(1, &mut vao);
        gl::GenBuffers(1, &mut vbo);
        gl::GenBuffers(1, &mut ibo);
        gl::BindVertexArray(vao);

        gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
        gl::BufferData(
            gl::ARRAY_BUFFER,
            std::mem::size_of_val(verts.as_slice()) as GLsizeiptr,
            verts.as_ptr().cast(),
            gl::STATIC_DRAW,
        );
        gl::EnableVertexAttribArray(0);
        gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, VERTEX_STRIDE, std::ptr::null());
        gl::EnableVertexAttribArray(1);
        gl::VertexAttribPointer(
            1,
            1,
            gl::FLOAT,
            gl::FALSE,
            VERTEX_STRIDE,
            std::ptr::without_provenance(8),
        );

        // The element buffer binding belongs to the vertex array, so it is set
        // while the array is bound and left alone afterwards.
        gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ibo);
        gl::BufferData(
            gl::ELEMENT_ARRAY_BUFFER,
            std::mem::size_of_val(indices.as_slice()) as GLsizeiptr,
            indices.as_ptr().cast(),
            gl::STATIC_DRAW,
        );

        gl::BindVertexArray(0);
        gl::BindBuffer(gl::ARRAY_BUFFER, 0);
    }
    Held {
        vao,
        vbo,
        ibo,
        index_count: indices.len() as GLint,
        last_used: frame,
    }
}

/// Bake each triangle's angular branch into its vertices.
///
/// Angular phase jumps by a whole turn across the far side of a figure, where
/// `atan2` wraps, and interpolating straight across that seam paints a band of
/// spurious rainbow. The CPU path shifts a triangle's coordinates onto one
/// branch as it gathers them, which it can do because it gathers them; a shared
/// vertex buffer has no per-triangle place to put the shift. So it becomes a
/// vertex attribute, and the few triangles straddling the seam get their own
/// copies of the vertices they need. The shift is a whole number of turns,
/// which is the only jump that is ever real, so it survives every colour knob
/// and is never rebuilt.
fn unwrapped(mesh: &RefinedMesh) -> (Vec<[f32; 3]>, Vec<u32>) {
    let points: Vec<[f32; 2]> = mesh.points().map(|p| [p.x(), p.y()]).collect();
    let phase: Vec<f32> = points
        .iter()
        .map(|p| p[1].atan2(p[0]) / std::f32::consts::TAU + 0.5)
        .collect();
    let mut verts: Vec<[f32; 3]> = points.iter().map(|p| [p[0], p[1], 0.0]).collect();
    let mut indices = Vec::with_capacity(mesh.indices().len());
    let mut shifted: HashMap<(u32, i32), u32> = HashMap::new();
    for tri in mesh
        .indices()
        .batches(usize::MAX)
        .flat_map(IndexBatch::triangles)
    {
        let Some(&reference) = phase.get(tri[0] as usize) else {
            continue;
        };
        for i in tri {
            let Some(&p) = phase.get(i as usize) else {
                continue;
            };
            let turn = (reference - p).round();
            if turn == 0.0 {
                indices.push(i);
                continue;
            }
            let index = *shifted.entry((i, turn as i32)).or_insert_with(|| {
                let v = verts[i as usize];
                verts.push([v[0], v[1], turn]);
                (verts.len() - 1) as u32
            });
            indices.push(index);
        }
    }
    (verts, indices)
}

/// Compile and link the fill shader against the current context.
fn build_program() -> Result<Program, String> {
    let vertex = compile(gl::VERTEX_SHADER, VERTEX_SRC)?;
    let fragment = compile(gl::FRAGMENT_SHADER, FRAGMENT_SRC)?;
    let program = link(vertex, fragment)?;
    // The shaders are attached to the program, which holds them alive.
    unsafe {
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
    }
    Ok(Program {
        loc: Locations::of(program),
        program,
    })
}

fn compile(kind: GLenum, src: &str) -> Result<GLuint, String> {
    let source = std::ffi::CString::new(src).map_err(|e| e.to_string())?;
    unsafe {
        let shader = gl::CreateShader(kind);
        gl::ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
        let mut status = GLint::from(gl::FALSE);
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut status);
        if status == GLint::from(gl::TRUE) {
            return Ok(shader);
        }
        let log = shader_log(shader, gl::GetShaderiv, gl::GetShaderInfoLog);
        gl::DeleteShader(shader);
        Err(log)
    }
}

fn link(vertex: GLuint, fragment: GLuint) -> Result<GLuint, String> {
    unsafe {
        let program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        // GLSL 1.50 has no `layout` qualifier, so attribute and fragment-output
        // locations are bound here rather than declared in the source.
        let pos = std::ffi::CString::new("a_pos").map_err(|e| e.to_string())?;
        let turn = std::ffi::CString::new("a_turn").map_err(|e| e.to_string())?;
        let out = std::ffi::CString::new("o_Color").map_err(|e| e.to_string())?;
        gl::BindAttribLocation(program, 0, pos.as_ptr());
        gl::BindAttribLocation(program, 1, turn.as_ptr());
        gl::BindFragDataLocation(program, 0, out.as_ptr());
        gl::LinkProgram(program);
        let mut status = GLint::from(gl::FALSE);
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut status);
        if status == GLint::from(gl::TRUE) {
            return Ok(program);
        }
        let log = shader_log(program, gl::GetProgramiv, gl::GetProgramInfoLog);
        gl::DeleteProgram(program);
        Err(log)
    }
}

/// Read a driver's own account of why it refused a shader or a program.
fn shader_log(
    object: GLuint,
    length_of: unsafe fn(GLuint, GLenum, *mut GLint),
    log_of: unsafe fn(GLuint, GLint, *mut GLint, *mut GLchar),
) -> String {
    unsafe {
        let mut len = 0;
        length_of(object, gl::INFO_LOG_LENGTH, &mut len);
        if len <= 0 {
            return "the driver gave no reason".to_string();
        }
        let mut buf = vec![0u8; len as usize];
        log_of(
            object,
            len,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast::<GLchar>(),
        );
        // The length includes the terminator, which is not part of the message.
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8_lossy(&buf).into_owned()
    }
}
