//! The per-vertex pass and the two ways a figure reaches the backend.
//!
//! Generic over the backend, so the same code runs on the GL window and on the
//! software rasteriser the golden images are taken through.

use super::fastmath;
use super::mesh::RefinedMesh;
use graphics::Graphics;
use graphics::draw_state::DrawState;
use graphics::math::Matrix2d;
use tunnels_lib::number::Phase;
use tunnels_model::layer::{ColorPhase, FillAnimation, FillTarget};

/// Vertices per chunk handed to the backend. `BACK_END_MAX_VERTEX_COUNT` is
/// 1023, which divides evenly into triangles.
const CHUNK: usize = 1023;

/// How far a spin animation can turn a point at the rim: a quarter turn.
const MAX_SPIN: f32 = std::f32::consts::TAU / 4.0;

/// Which coordinate of a figure a phase is read along, and how many colour
/// cycles run across it.
#[derive(Copy, Clone)]
pub struct PhaseField {
    pub phase: ColorPhase,
    pub cycles: f32,
}

impl PhaseField {
    /// The period over which this phase wraps, if it wraps at all.
    ///
    /// Only angular phase does: `atan2` jumps a full turn on the far side of
    /// the figure, which is `cycles` in scaled units. The linear and radial
    /// coordinates are continuous, so there is nothing to unwrap and nothing
    /// that could be mistaken for a wrap.
    pub fn wrap_period(self) -> Option<f32> {
        match self.phase {
            ColorPhase::Angle if self.cycles > 0.0 => Some(self.cycles),
            _ => None,
        }
    }
}

/// Buffers the per-vertex pass fills, reused across frames.
///
/// Reused so a frame allocates nothing: at this triangle count, a fresh vertex
/// buffer every frame is megabytes of churn.
#[derive(Default)]
pub struct VertexBuffers {
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub tints: Vec<[f32; 4]>,
}

/// Everything the per-vertex pass reads.
pub struct VertexWork<'a> {
    /// The coordinate indexing the ramp, and its cycle count.
    pub field: PhaseField,
    /// The layer's own spin, in turns at the rim.
    pub base_spin: f32,
    /// Animations displacing geometry.
    pub warps: &'a [FillAnimation],
}

/// Everything a frame does per vertex, in one walk of the mesh.
///
/// Fused because the things it produces all want the same polar coordinates:
/// the ramp coordinate is an angle, a spin is a rotation about the same centre,
/// and a radial animation scales the same radius. Computed separately that was
/// two `atan2` calls a vertex, which a profile of the prototype put at a third
/// of the frame.
///
/// Position comes out displaced; the ramp coordinate is read from the vertex
/// *before* displacement, so a colour pattern stays glued to the figure while a
/// warp moves it rather than sliding across it.
pub fn vertex_pass(out: &mut VertexBuffers, mesh: &RefinedMesh, work: VertexWork) {
    let VertexWork {
        field,
        base_spin,
        warps,
    } = work;

    out.positions.clear();
    out.uvs.clear();
    out.tints.clear();

    // An angle costs an `atan2` and a radius a square root, so decide once
    // whether anything actually asks for them.
    let rotates = base_spin != 0.0 || warps.iter().any(|w| w.target == FillTarget::Spin);
    let needs_angle = rotates || field.phase == ColorPhase::Angle;
    let needs_radius = rotates
        || field.phase == ColorPhase::Radius
        || warps
            .iter()
            .any(|w| matches!(w.target, FillTarget::Radial | FillTarget::AspectRatio));

    for (i, v) in mesh.verts.iter().enumerate() {
        let polar = Polar::of(*v, needs_angle, needs_radius);
        let along = polar.phase(*v, field.phase);

        let mut radial = 1.0f32;
        let mut turn = base_spin * polar.radius * std::f32::consts::TAU;
        let (mut scale_x, mut scale_y) = (1.0f32, 1.0f32);
        for warp in warps {
            let value = warp.animation.value(Phase::new(f64::from(along)), i) as f32;
            match warp.target {
                // Multiplicative, so the deformation is proportional: a
                // waveform around the angle turns a disc into petals.
                FillTarget::Radial => radial *= 1.0 + value,
                FillTarget::Spin => turn += value * MAX_SPIN,
                FillTarget::AspectRatio => {
                    scale_x *= 1.0 + value;
                    scale_y *= 1.0 - value;
                }
                _ => {}
            }
        }
        // A negative radius would turn the figure inside out through the origin
        // rather than collapsing it.
        let radial = radial.max(0.0);
        out.positions.push(if rotates {
            let angle = polar.angle + turn;
            let radius = polar.radius * radial;
            [
                radius * angle.cos() * scale_x,
                radius * angle.sin() * scale_y,
            ]
        } else {
            // Without rotation the angle never changes, so scaling the radius
            // is scaling x and y — no round trip through polar coordinates.
            [v[0] * radial * scale_x, v[1] * radial * scale_y]
        });

        out.uvs.push([along * field.cycles, 0.5]);
    }
}

/// A vertex's polar coordinates, computed only where they are wanted.
#[derive(Clone, Copy, Default)]
struct Polar {
    radius: f32,
    angle: f32,
}

impl Polar {
    #[inline]
    fn of(v: [f32; 2], angle: bool, radius: bool) -> Self {
        Self {
            radius: if radius {
                (v[0] * v[0] + v[1] * v[1]).sqrt()
            } else {
                0.0
            },
            angle: if angle {
                fastmath::atan2(v[1], v[0])
            } else {
                0.0
            },
        }
    }

    /// Unit phase along one coordinate, reusing what has already been computed.
    #[inline]
    fn phase(self, v: [f32; 2], phase: ColorPhase) -> f32 {
        match phase {
            ColorPhase::Angle => self.angle / std::f32::consts::TAU + 0.5,
            ColorPhase::Radius => self.radius / std::f32::consts::SQRT_2,
            ColorPhase::Linear => (v[1] + 1.0) / 2.0,
        }
    }
}

/// Put two phase coordinates on the same branch.
///
/// Angular phase jumps by a whole turn across the far side of the figure, where
/// `atan2` wraps. Both ends still sample the right texel — the ramp repeats,
/// and a whole number of cycles is a whole number of periods — but interpolating
/// straight between them sweeps the long way round, painting a band of spurious
/// rainbow along the seam.
///
/// The shift has to be in units of the wrap period, not of one cycle. A coarse
/// mesh with a high cycle count has triangles legitimately spanning more than a
/// cycle, and rounding to the nearest cycle would "correct" those into
/// nonsense. A whole turn is the only jump that is ever real.
#[inline]
fn same_branch(reference: f32, u: f32, period: f32) -> f32 {
    u + ((reference - u) / period).round() * period
}

/// Draw a refined mesh in one flat colour.
///
/// Needed because a warped layer cannot take the unrefined fast path even when
/// its colour is uniform: the displacement lives on the refined mesh's
/// vertices.
pub fn draw_flat<G: Graphics>(
    mesh: &RefinedMesh,
    positions: &[[f32; 2]],
    color: [f32; 4],
    m: Matrix2d,
    gl: &mut G,
) {
    if mesh.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    gl.tri_list(&DrawState::default(), &color, |f| {
        for chunk in mesh.indices.chunks(stride) {
            pos.clear();
            pos.extend(
                chunk
                    .iter()
                    .filter_map(|&i| positions.get(i as usize).map(|v| project(m, *v))),
            );
            f(&pos);
        }
    });
}

/// Draw a raw triangle list in one flat colour, with no refinement.
///
/// The path a figure takes when nothing varies across it: no ramp, no per-
/// vertex pass, and the tessellator's own triangles rather than a refined mesh.
pub fn draw_tris<G: Graphics>(tris: &[[f32; 2]], color: [f32; 4], m: Matrix2d, gl: &mut G) {
    if tris.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    gl.tri_list(&DrawState::default(), &color, |f| {
        for chunk in tris.chunks(stride) {
            pos.clear();
            pos.extend(chunk.iter().map(|v| project(m, *v)));
            f(&pos);
        }
    });
}

/// Draw a refined mesh, taking its colour from a ramp texture indexed by phase.
///
/// The sampler resolves the waveform per fragment, so the sawtooth's jump lands
/// exactly where it belongs however coarse the mesh is.
pub fn draw_textured<G: Graphics>(
    mesh: &RefinedMesh,
    positions: &[[f32; 2]],
    uvs: &[[f32; 2]],
    period: Option<f32>,
    texture: &G::Texture,
    m: Matrix2d,
    gl: &mut G,
) {
    if mesh.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    let mut uv = Vec::with_capacity(stride);

    gl.tri_list_uv(&DrawState::default(), &[1.0; 4], texture, |f| {
        for chunk in mesh.indices.chunks(stride) {
            pos.clear();
            uv.clear();
            for tri in chunk.chunks(3) {
                let Some(reference) = tri.first().and_then(|&i| uvs.get(i as usize)) else {
                    continue;
                };
                let reference = reference[0];
                for &i in tri {
                    let (Some(p), Some(v)) = (positions.get(i as usize), uvs.get(i as usize))
                    else {
                        continue;
                    };
                    pos.push(project(m, *p));
                    uv.push([
                        match period {
                            Some(p) => same_branch(reference, v[0], p),
                            None => v[0],
                        },
                        v[1],
                    ]);
                }
            }
            f(&pos, &uv);
        }
    });
}

/// Apply a 2D affine matrix to a vertex.
///
/// Piston's backends take pre-transformed vertices when `tri_list` is called
/// directly, so this is the vertex shader a fixed pipeline does not have.
#[inline]
fn project(m: Matrix2d, v: [f32; 2]) -> [f32; 2] {
    let (x, y) = (f64::from(v[0]), f64::from(v[1]));
    [
        (m[0][0] * x + m[0][1] * y + m[0][2]) as f32,
        (m[1][0] * x + m[1][1] * y + m[1][2]) as f32,
    ]
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_seam_takes_the_short_way_round() {
        // Two ends of a triangle straddling where `atan2` wraps, with three
        // colour cycles across the figure. The far end is pulled back a whole
        // turn so the interpolation runs the short way.
        let period = 3.0;
        let pulled = same_branch(0.1, 3.05, period);
        assert!(
            (pulled - 0.05).abs() < 1e-6,
            "{pulled} is not next to the reference"
        );
        // A triangle legitimately spanning more than a cycle is left alone,
        // because the shift is in whole turns rather than whole cycles.
        assert_eq!(same_branch(0.0, 1.4, period), 1.4);
    }
}
