//! The same fill, drawn by a shader instead of by the CPU.
//!
//! The CPU path pays for a mesh twice every frame: once walking it to displace
//! vertices and work out where each one looks in the ramp, and again projecting
//! every vertex and gathering it into piston's buffers, because `tri_list_uv`
//! takes pre-transformed vertices. Both are things a vertex shader does for
//! free. Here the mesh is uploaded once and never touched again, the transform
//! and the animation parameters are uniforms, and the fragment shader samples
//! the same ramp texture the CPU path samples.
//!
//! Targets GLSL 1.50 — the render clients are old enough that OpenGL 3.2 core
//! is the ceiling, which is why the shader carries no compute stage, no storage
//! buffer, and no `layout` qualifiers.
//!
//! # Sharing a context with piston
//!
//! `GlGraphics` batches vertices and caches the program and draw state it last
//! bound. Issuing GL calls while it holds that cache corrupts the batch, and
//! the symptom is a layer that intermittently fails to draw. So a draw here is
//! bracketed: piston's batch is flushed first, and its caches are invalidated
//! afterwards so it rebinds from scratch rather than trusting what it
//! remembers. See `Boundary`.

use crate::draw::AxisWave;
use crate::mesh::{MeshId, RefinedMesh};
use crate::params::{AnimTarget, ColorPhase, LayerParams, PhaseField};
use gl::types::{GLchar, GLenum, GLint, GLsizeiptr, GLuint};
use graphics::color::gamma_srgb_to_linear;
use graphics::math::Matrix2d;
use opengl_graphics::{GlGraphics, Texture};
use std::collections::HashMap;
use std::f32::consts::TAU;
use tunnels_model::animation::{Waveform, WaveformState};

/// Animation slots the shader carries, matching `params::N_WAVES`.
const MAX_WAVES: usize = crate::params::N_WAVES;

const VERTEX_SRC: &str = include_str!("shaders/fill.vert");
const FRAGMENT_SRC: &str = include_str!("shaders/fill.frag");

/// What a slot drives, as the shader numbers it. Zero is an empty slot.
///
/// Only the colour targets the ramp cannot carry appear: one that runs along
/// the layer's own axis is baked into the ramp texture, on the CPU, where a
/// discontinuity gets a clean edge.
const TARGET_NONE: i32 = 0;
const TARGET_RADIAL: i32 = 1;
const TARGET_SPIN: i32 = 2;
const TARGET_ASPECT: i32 = 3;
const TARGET_HUE: i32 = 4;
const TARGET_BRIGHTNESS: i32 = 5;

/// Waveforms, as the shader numbers them.
const WAVE_SINE: i32 = 0;
const WAVE_TRIANGLE: i32 = 1;
const WAVE_SQUARE: i32 = 2;
const WAVE_SAWTOOTH: i32 = 3;
const WAVE_CONSTANT: i32 = 4;

/// Waveform switches, packed one to a bit.
const FLAG_PULSE: i32 = 1;
const FLAG_STANDING: i32 = 2;
const FLAG_INVERT: i32 = 4;

/// Bytes one vertex occupies: position, then the branch offset.
const VERTEX_STRIDE: i32 = 12;

fn phase_code(phase: ColorPhase) -> i32 {
    match phase {
        ColorPhase::Angle => 0,
        ColorPhase::Radius => 1,
        ColorPhase::LinearX => 2,
        ColorPhase::LinearY => 3,
    }
}

/// The shader's number for a waveform, or `None` for one it cannot evaluate.
///
/// Noise is the exception, and the reason is not effort: it reads a 2D simplex
/// field indexed by the vertex's own position in the mesh, which is neither a
/// closed form nor something a uniform can carry.
fn waveform_code(waveform: Waveform) -> Option<i32> {
    match waveform {
        Waveform::Sine => Some(WAVE_SINE),
        Waveform::Triangle => Some(WAVE_TRIANGLE),
        Waveform::Square => Some(WAVE_SQUARE),
        Waveform::Sawtooth => Some(WAVE_SAWTOOTH),
        Waveform::Constant => Some(WAVE_CONSTANT),
        Waveform::Noise => None,
    }
}

/// One animation slot as the shader reads it.
#[derive(Clone, Copy)]
struct WaveUniform {
    target: i32,
    phase: i32,
    form: i32,
    flags: i32,
    periods: f32,
    duty: f32,
    smoothing: f32,
    temporal: f32,
    scale: f32,
}

impl Default for WaveUniform {
    fn default() -> Self {
        Self {
            target: TARGET_NONE,
            phase: 0,
            form: WAVE_CONSTANT,
            flags: 0,
            periods: 0.0,
            duty: 1.0,
            smoothing: 0.0,
            temporal: 0.0,
            scale: 0.0,
        }
    }
}

impl WaveUniform {
    /// One axis animation, or `None` if the shader cannot evaluate its
    /// waveform.
    fn of(wave: &AxisWave, target: i32) -> Option<Self> {
        let state: WaveformState = wave.wave.state();
        let form = waveform_code(state.waveform)?;
        let mut flags = 0;
        if state.pulse {
            flags |= FLAG_PULSE;
        }
        if state.standing {
            flags |= FLAG_STANDING;
        }
        if state.invert {
            flags |= FLAG_INVERT;
        }
        Some(Self {
            // A zero-size animation contributes nothing, which is what an
            // empty slot means here.
            target: if state.active { target } else { TARGET_NONE },
            phase: phase_code(wave.phase),
            form,
            flags,
            periods: f32::from(state.n_periods),
            duty: state.duty_cycle.val() as f32,
            smoothing: state.smoothing.val() as f32,
            temporal: state.phase_temporal.val() as f32,
            scale: state.scale as f32,
        })
    }
}

/// Everything one draw call varies.
pub struct Uniforms {
    /// The rows of the affine transform, which land straight in clip space.
    xform: Matrix2d,
    phase: i32,
    cycles: f32,
    /// Whether the branch baked into the mesh applies. Only angular phase
    /// wraps, so only angular phase needs unwrapping.
    unwrap: f32,
    spin: f32,
    /// Whether anything rotates. Without a rotation the angle never changes,
    /// so the shader can scale x and y directly rather than round-tripping
    /// through polar coordinates — the same split the CPU pass makes.
    rotates: i32,
    /// The single colour a uniform or masked layer draws in, in the linear
    /// space a vertex colour reaches a shader in.
    flat: Option<[f32; 4]>,
    waves: [WaveUniform; MAX_WAVES],
}

/// The axis animations of one layer, split by where they are resolved.
pub struct LayerWaves<'a> {
    pub warps: &'a [AxisWave<'a>],
    pub hue_axes: &'a [AxisWave<'a>],
    pub bright_axes: &'a [AxisWave<'a>],
}

impl Uniforms {
    /// A layer whose colour comes from the ramp.
    ///
    /// `None` when an animation on this layer uses a waveform the shader
    /// cannot evaluate.
    pub fn textured(
        layer: &LayerParams,
        field: PhaseField,
        waves: LayerWaves,
        xform: Matrix2d,
    ) -> Option<Self> {
        let mut slots = [WaveUniform::default(); MAX_WAVES];
        let mut next = 0;
        let mut push = |wave: &AxisWave, target: i32| -> Option<()> {
            // More animations than slots cannot happen: the shader carries one
            // per slot the control surface offers.
            let slot = slots.get_mut(next)?;
            *slot = WaveUniform::of(wave, target)?;
            next += 1;
            Some(())
        };
        for w in waves.warps {
            let target = match w.target {
                AnimTarget::Radial => TARGET_RADIAL,
                AnimTarget::Spin => TARGET_SPIN,
                AnimTarget::AspectRatio => TARGET_ASPECT,
                // A colour target never reaches the warp list.
                _ => continue,
            };
            push(w, target)?;
        }
        for w in waves.hue_axes {
            push(w, TARGET_HUE)?;
        }
        for w in waves.bright_axes {
            push(w, TARGET_BRIGHTNESS)?;
        }

        let rotates = layer.spin != 0.0 || slots.iter().any(|s| s.target == TARGET_SPIN);
        Some(Self {
            xform,
            phase: phase_code(field.phase),
            cycles: field.cycles,
            unwrap: if field.wrap_period().is_some() {
                1.0
            } else {
                0.0
            },
            spin: layer.spin as f32,
            rotates: i32::from(rotates),
            flat: None,
            waves: slots,
        })
    }

    /// A layer drawn in one colour, undeformed.
    pub fn flat(color: [f32; 4], xform: Matrix2d) -> Self {
        Self {
            xform,
            phase: 0,
            cycles: 0.0,
            unwrap: 0.0,
            spin: 0.0,
            rotates: 0,
            // Piston linearises a vertex colour on its way into the batch, so
            // a flat colour has to arrive here having had the same done to it.
            flat: Some(gamma_srgb_to_linear(color)),
            waves: [WaveUniform::default(); MAX_WAVES],
        }
    }
}

/// What to draw, and what to build its buffer from on first use.
pub enum MeshSource<'a> {
    /// A refined mesh, whose vertices carry the angular branch.
    Refined { id: MeshId, mesh: &'a RefinedMesh },
    /// The tessellator's own triangle list, which is all a layer drawn in one
    /// colour needs.
    Source {
        shape: usize,
        stroke_width: Option<u32>,
        tris: &'a [[f32; 2]],
    },
}

/// Identifies one uploaded buffer.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum BufferId {
    Refined(MeshId),
    Source {
        shape: usize,
        stroke_width: Option<u32>,
    },
}

impl MeshSource<'_> {
    fn id(&self) -> BufferId {
        match *self {
            Self::Refined { id, .. } => BufferId::Refined(id),
            Self::Source {
                shape,
                stroke_width,
                ..
            } => BufferId::Source {
                shape,
                stroke_width,
            },
        }
    }
}

/// A mesh living in GPU memory.
#[derive(Clone, Copy)]
struct GpuMesh {
    vao: GLuint,
    index_count: i32,
    triangles: usize,
}

/// The shader, its uniform locations, and the buffers drawn through it.
///
/// Buffers are never evicted, for the same reason `MeshLibrary` never evicts:
/// a show touches a handful of shapes at a handful of sizes, so the set
/// converges within the first few seconds and nothing is rebuilt mid-show.
pub struct GpuRenderer {
    program: GLuint,
    loc: Locations,
    meshes: HashMap<BufferId, GpuMesh>,
}

/// Where each uniform lives in the linked program.
struct Locations {
    xform0: GLint,
    xform1: GLint,
    phase: GLint,
    cycles: GLint,
    unwrap: GLint,
    spin: GLint,
    rotates: GLint,
    flat: GLint,
    color: GLint,
    ramp: GLint,
    wave_target: GLint,
    wave_phase: GLint,
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
        let at = |name: &str| -> GLint {
            let Ok(c) = std::ffi::CString::new(name) else {
                return -1;
            };
            // A location of -1 makes every `glUniform` call against it a
            // silent no-op, which is the right answer for a uniform the
            // compiler folded away.
            unsafe { gl::GetUniformLocation(program, c.as_ptr()) }
        };
        Self {
            xform0: at("u_xform0"),
            xform1: at("u_xform1"),
            phase: at("u_phase"),
            cycles: at("u_cycles"),
            unwrap: at("u_unwrap"),
            spin: at("u_spin"),
            rotates: at("u_rotates"),
            flat: at("u_flat"),
            color: at("u_color"),
            ramp: at("u_ramp"),
            wave_target: at("u_wave_target"),
            wave_phase: at("u_wave_phase"),
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

/// What one draw through the shader put on the GPU.
#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    /// Building and uploading a buffer, which happens once per mesh.
    pub upload_us: u128,
    /// Setting uniforms and issuing the draw.
    pub submit_us: u128,
    pub triangles: usize,
}

impl GpuRenderer {
    /// Compile and link the fill shader against the current context.
    ///
    /// Fails rather than panics: a shader the driver rejects should cost the
    /// show a fallback to the CPU path, not the show.
    pub fn new() -> Result<Self, String> {
        let vertex = compile(gl::VERTEX_SHADER, VERTEX_SRC)?;
        let fragment = compile(gl::FRAGMENT_SHADER, FRAGMENT_SRC)?;
        let program = link(vertex, fragment)?;
        // The shaders are attached to the program, which holds them alive.
        unsafe {
            gl::DeleteShader(vertex);
            gl::DeleteShader(fragment);
        }
        Ok(Self {
            loc: Locations::of(program),
            program,
            meshes: HashMap::new(),
        })
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Draw one piece of a layer.
    ///
    /// `gl` is borrowed to flush and invalidate piston's batch around the
    /// draw, not to draw through.
    pub fn draw(
        &mut self,
        gl: &mut GlGraphics,
        source: MeshSource,
        uniforms: &Uniforms,
        ramp: &Texture,
    ) -> DrawStats {
        let mut stats = DrawStats::default();
        let id = source.id();
        let mesh = *self.meshes.entry(id).or_insert_with(|| {
            let mark = std::time::Instant::now();
            let mesh = upload(&source);
            stats.upload_us = mark.elapsed().as_micros();
            mesh
        });
        if mesh.index_count == 0 {
            return stats;
        }
        stats.triangles = mesh.triangles;

        let mark = std::time::Instant::now();
        let _boundary = Boundary::enter(gl);
        unsafe {
            gl::UseProgram(self.program);
            self.set(uniforms);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, ramp.get_id());
            // Piston draws both windings, and a tessellated fill contains
            // both.
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(mesh.vao);
            gl::DrawElements(
                gl::TRIANGLES,
                mesh.index_count,
                gl::UNSIGNED_INT,
                std::ptr::null(),
            );
            gl::BindVertexArray(0);
        }
        stats.submit_us = mark.elapsed().as_micros();
        stats
    }

    /// Push one draw's uniforms into the program, which must be in use.
    unsafe fn set(&self, u: &Uniforms) {
        let loc = &self.loc;
        let row = |r: [f64; 3]| [r[0] as f32, r[1] as f32, r[2] as f32];
        let r0 = row(u.xform[0]);
        let r1 = row(u.xform[1]);
        let targets: [i32; MAX_WAVES] = u.waves.map(|w| w.target);
        let phases: [i32; MAX_WAVES] = u.waves.map(|w| w.phase);
        let forms: [i32; MAX_WAVES] = u.waves.map(|w| w.form);
        let flags: [i32; MAX_WAVES] = u.waves.map(|w| w.flags);
        let periods: [f32; MAX_WAVES] = u.waves.map(|w| w.periods);
        let duty: [f32; MAX_WAVES] = u.waves.map(|w| w.duty);
        let smoothing: [f32; MAX_WAVES] = u.waves.map(|w| w.smoothing);
        let temporal: [f32; MAX_WAVES] = u.waves.map(|w| w.temporal);
        let scale: [f32; MAX_WAVES] = u.waves.map(|w| w.scale);
        let n = MAX_WAVES as i32;
        unsafe {
            gl::Uniform3fv(loc.xform0, 1, r0.as_ptr());
            gl::Uniform3fv(loc.xform1, 1, r1.as_ptr());
            gl::Uniform1i(loc.phase, u.phase);
            gl::Uniform1f(loc.cycles, u.cycles);
            gl::Uniform1f(loc.unwrap, u.unwrap);
            gl::Uniform1f(loc.spin, u.spin);
            gl::Uniform1i(loc.rotates, u.rotates);
            match u.flat {
                Some(c) => {
                    gl::Uniform1i(loc.flat, 1);
                    gl::Uniform4fv(loc.color, 1, c.as_ptr());
                }
                None => gl::Uniform1i(loc.flat, 0),
            }
            // Piston never sets its own sampler uniforms, so it relies on unit
            // zero being the active one; staying on it is what keeps that true.
            gl::Uniform1i(loc.ramp, 0);
            gl::Uniform1iv(loc.wave_target, n, targets.as_ptr());
            gl::Uniform1iv(loc.wave_phase, n, phases.as_ptr());
            gl::Uniform1iv(loc.wave_form, n, forms.as_ptr());
            gl::Uniform1iv(loc.wave_flags, n, flags.as_ptr());
            gl::Uniform1fv(loc.wave_periods, n, periods.as_ptr());
            gl::Uniform1fv(loc.wave_duty, n, duty.as_ptr());
            gl::Uniform1fv(loc.wave_smoothing, n, smoothing.as_ptr());
            gl::Uniform1fv(loc.wave_temporal, n, temporal.as_ptr());
            gl::Uniform1fv(loc.wave_scale, n, scale.as_ptr());
        }
    }
}

/// A clean boundary between piston's batch and ours.
///
/// Entering flushes whatever piston has queued, so nothing of ours lands in
/// the middle of it and the drawing order across the two paths is the order
/// the layers were submitted in. Leaving drops the caches `GlGraphics` keeps
/// of the program and draw state it last bound, because both are now stale:
/// the program is ours, and the blend, scissor and stencil state is only
/// guaranteed to be what piston last set if nothing between then and now
/// touched it. Dropping them costs one redundant rebind per frame and removes
/// the entire class of bug where piston skips a state change it believes is
/// in effect.
///
/// The rest of the state this pass touches it restores itself: the vertex
/// array binding goes back to zero, the texture unit stays at zero, and face
/// culling is left disabled, which is where every piston flush leaves it.
struct Boundary<'a> {
    gl: &'a mut GlGraphics,
}

impl<'a> Boundary<'a> {
    fn enter(gl: &'a mut GlGraphics) -> Self {
        gl.draw_end();
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

/// Build the vertex and index buffers for a mesh and hand them to the GPU.
fn upload(source: &MeshSource) -> GpuMesh {
    let (verts, indices) = match source {
        MeshSource::Refined { mesh, .. } => unwrapped(mesh),
        MeshSource::Source { tris, .. } => (
            tris.iter().map(|v| [v[0], v[1], 0.0]).collect(),
            (0..tris.len() as u32).collect(),
        ),
    };
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
    GpuMesh {
        vao,
        index_count: indices.len() as i32,
        triangles: indices.len() / 3,
    }
}

/// Bake each triangle's angular branch into its vertices.
///
/// Angular phase jumps a whole turn across the far side of the shape, where
/// `atan2` wraps, and interpolating straight across that seam paints a band of
/// spurious rainbow. The CPU path shifts a triangle's coordinates onto one
/// branch as it gathers them, which it can do because it gathers them; a
/// shared vertex buffer has no per-triangle place to put the shift. So it
/// becomes a vertex attribute, and the few triangles that straddle the seam
/// get their own copies of the vertices they need. The shift is a whole number
/// of turns, which is the only jump that is ever real, so it survives every
/// colour knob and never has to be rebuilt.
fn unwrapped(mesh: &RefinedMesh) -> (Vec<[f32; 3]>, Vec<u32>) {
    let phase: Vec<f32> = mesh
        .verts
        .iter()
        .map(|v| v[1].atan2(v[0]) / TAU + 0.5)
        .collect();
    let mut verts: Vec<[f32; 3]> = mesh.verts.iter().map(|v| [v[0], v[1], 0.0]).collect();
    let mut indices = Vec::with_capacity(mesh.indices.len());
    let mut shifted: HashMap<(u32, i32), u32> = HashMap::new();
    for tri in mesh.indices.as_chunks::<3>().0 {
        let Some(&reference) = phase.get(tri[0] as usize) else {
            continue;
        };
        for &i in tri {
            let Some(&p) = phase.get(i as usize) else {
                continue;
            };
            let turn = (reference - p).round();
            if turn == 0.0 {
                indices.push(i);
                continue;
            }
            let index = *shifted.entry((i, turn as i32)).or_insert_with(|| {
                verts.push([verts[i as usize][0], verts[i as usize][1], turn]);
                (verts.len() - 1) as u32
            });
            indices.push(index);
        }
    }
    (verts, indices)
}

fn compile(kind: GLenum, src: &str) -> Result<GLuint, String> {
    let source = std::ffi::CString::new(src).map_err(|e| e.to_string())?;
    unsafe {
        let shader = gl::CreateShader(kind);
        gl::ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
        let mut status: GLint = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut status);
        if status == GLint::from(gl::TRUE) {
            return Ok(shader);
        }
        let log = info_log(shader, gl::GetShaderiv, gl::GetShaderInfoLog);
        gl::DeleteShader(shader);
        Err(log)
    }
}

fn link(vertex: GLuint, fragment: GLuint) -> Result<GLuint, String> {
    // Attribute locations are bound rather than declared, because a `layout`
    // qualifier on an input needs a GLSL version past the one the oldest
    // client can compile.
    let pos = std::ffi::CString::new("a_pos").map_err(|e| e.to_string())?;
    let turn = std::ffi::CString::new("a_turn").map_err(|e| e.to_string())?;
    let out = std::ffi::CString::new("o_Color").map_err(|e| e.to_string())?;
    unsafe {
        let program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::BindAttribLocation(program, 0, pos.as_ptr());
        gl::BindAttribLocation(program, 1, turn.as_ptr());
        gl::BindFragDataLocation(program, 0, out.as_ptr());
        gl::LinkProgram(program);
        let mut status: GLint = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut status);
        if status == GLint::from(gl::TRUE) {
            return Ok(program);
        }
        let log = info_log(program, gl::GetProgramiv, gl::GetProgramInfoLog);
        gl::DeleteProgram(program);
        Err(log)
    }
}

/// Read back the driver's account of why a shader or program failed.
unsafe fn info_log(
    object: GLuint,
    get: unsafe fn(GLuint, GLenum, *mut GLint),
    read: unsafe fn(GLuint, i32, *mut i32, *mut GLchar),
) -> String {
    unsafe {
        let mut len: GLint = 0;
        get(object, gl::INFO_LOG_LENGTH, &mut len);
        if len <= 0 {
            return "no diagnostic from the driver".to_string();
        }
        let mut buf = vec![0u8; len as usize];
        read(object, len, std::ptr::null_mut(), buf.as_mut_ptr().cast());
        String::from_utf8_lossy(&buf)
            .trim_end_matches(['\0', '\n'])
            .to_string()
    }
}
