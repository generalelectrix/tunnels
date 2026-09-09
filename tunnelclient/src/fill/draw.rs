//! The per-vertex pass and the two ways a figure reaches the backend.
//!
//! Generic over the backend, so the same code runs on the GL window and on the
//! software rasteriser the golden images are taken through.

use super::fastmath;
use super::geometry::StrokeMesh;
use super::mesh::RefinedMesh;
use super::ramp::RampSpan;
use graphics::Graphics;
use graphics::draw_state::DrawState;
use graphics::math::Matrix2d;
use tunnels_lib::number::Phase;
use tunnels_model::animation::{PreparedAnimation, TargetedAnimation};
use tunnels_model::animation_target::AnimationTarget;
use tunnels_model::layer::PhaseAxis;
use tunnels_sprites::Point;

/// Vertices per chunk handed to the backend. `BACK_END_MAX_VERTEX_COUNT` is
/// 1023, which divides evenly into triangles.
const CHUNK: usize = 1023;

/// How far a spin animation can turn a point at the rim: a quarter turn.
const MAX_SPIN: f32 = std::f32::consts::TAU / 4.0;

/// Which coordinate of a figure a phase is read along, how many colour cycles
/// run across it, and what the ramp it indexes holds.
#[derive(Copy, Clone)]
pub struct PhaseField {
    pub phase: PhaseAxis,
    pub cycles: f32,
    pub span: RampSpan,
}

impl PhaseField {
    /// What a figure coordinate is stretched by to index the ramp.
    ///
    /// This is where the cycle count lives or does not: a cycle-wide table is
    /// indexed `cycles` times across the figure, and a figure-wide one once,
    /// with the count moved onto the colour's own sample instead. The scale
    /// belongs to the coordinate rather than to the mesh, so a mesh built for a
    /// figure holds however its colour is tabulated.
    fn ramp_scale(self) -> f32 {
        match self.span {
            RampSpan::Cycle => self.cycles,
            RampSpan::Figure => 1.0,
        }
    }

    /// The period over which this phase wraps, if it wraps at all.
    ///
    /// Only angular phase does: `atan2` jumps a full turn on the far side of
    /// the figure, which is one whole traversal of the coordinate in scaled
    /// units. The linear and radial coordinates are continuous, so there is
    /// nothing to unwrap and nothing that could be mistaken for a wrap.
    pub fn wrap_period(self) -> Option<f32> {
        match (self.phase, self.span) {
            // Scaled to nothing: every vertex reads one texel and no seam can
            // show.
            (PhaseAxis::Angle, RampSpan::Cycle) if self.cycles == 0.0 => None,
            (PhaseAxis::Angle, _) => Some(self.ramp_scale()),
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
    /// Figure-space positions, displaced by whatever warps the layer.
    positions: Vec<Point>,
    /// Where each vertex looks in the colour ramp. Already in the backend's
    /// vocabulary, since nothing between here and the sampler reads it.
    uvs: Vec<[f32; 2]>,
}

impl VertexBuffers {
    /// Everything a frame does per vertex, in one walk of the mesh.
    ///
    /// Fused because the things it produces all want the same polar
    /// coordinates: the ramp coordinate is an angle, a spin is a rotation
    /// about the same centre, and a radial animation scales the same radius.
    /// Computed separately that was two `atan2` calls a vertex, which a
    /// profile of the prototype put at a third of the frame.
    ///
    /// Position comes out displaced; the ramp coordinate is read from the
    /// vertex *before* displacement, so a colour pattern stays glued to the
    /// figure while a warp moves it rather than sliding across it.
    pub fn vertex_pass(&mut self, mesh: &RefinedMesh, work: VertexWork) {
        let needs = Needs::of(&work);
        let anchor = Displacement::anchor(&work);
        self.positions.clear();
        self.uvs.clear();

        for (i, v) in mesh.verts.iter().enumerate() {
            let polar = Polar::of(*v, needs.angle, needs.radius);
            let along = polar.phase(*v, work.field.phase);
            let displacement = Displacement::of(&work, polar, along, i).beyond(anchor);

            let (x, y) = if needs.rotates {
                // The figure's own points, moved in polar terms because a
                // spin is a rotation about the same centre the phase is
                // measured from.
                let angle = polar.angle + displacement.turn;
                let radius = polar.radius * displacement.radial;
                (radius * angle.cos(), radius * angle.sin())
            } else {
                // Without rotation the angle never changes, so scaling the
                // radius is scaling x and y — no round trip through polar
                // coordinates.
                (v.x() * displacement.radial, v.y() * displacement.radial)
            };
            self.positions.push(displacement.place(x, y));
            self.uvs.push([along * work.field.ramp_scale(), 0.5]);
        }
    }

    /// As [`vertex_pass`](Self::vertex_pass), for an outline.
    ///
    /// Differs in one thing, and it is the whole of what makes a stroke
    /// cheap: **phase comes from the contour point a vertex was offset from,
    /// not from where the vertex landed.** A ribbon is one segment at one
    /// place on the figure, so both its edges take the same colour, and there
    /// is no variation across its width for a refinement to resolve.
    ///
    /// The displacement is worked out from the contour point too, so a warp
    /// moves the ribbon as a unit rather than shearing its two edges apart. It
    /// is then applied to the vertex where it actually is — as a rotation and
    /// a scale about the origin, which is the same transform the fill reaches
    /// through polar coordinates, without a second arctangent per vertex.
    ///
    /// The width is answered here as well, and only here. An outline is
    /// stroked once at a width no beam exceeds, and each vertex is drawn in
    /// from there toward the contour point it was offset from — so a thickness
    /// animation reaches a figure the way every other geometry animation does,
    /// at each of its points, and an outline can be thick at one place around
    /// a figure and gone at another.
    pub fn stroke_vertex_pass(&mut self, mesh: &StrokeMesh, work: VertexWork) {
        let needs = Needs::of(&work);
        let anchor = Displacement::anchor(&work);
        self.positions.clear();
        self.uvs.clear();

        for (i, vertex) in mesh.vertices().enumerate() {
            let polar = Polar::of(vertex.on_path, needs.angle, needs.radius);
            let along = polar.phase(vertex.on_path, work.field.phase);
            let displacement = Displacement::of(&work, polar, along, i).beyond(anchor);

            // The ribbon's own width, applied before anything moves the
            // point: the offset is in the figure's undisplaced coordinates,
            // which is where the contour point it is measured from lives.
            let reach = work.taper_at(along, i);
            let (x, y) = (
                vertex.on_path.x() + (vertex.position.x() - vertex.on_path.x()) * reach,
                vertex.on_path.y() + (vertex.position.y() - vertex.on_path.y()) * reach,
            );
            let (x, y) = if needs.rotates {
                let (sin, cos) = displacement.turn.sin_cos();
                (x * cos - y * sin, x * sin + y * cos)
            } else {
                (x, y)
            };
            self.positions
                .push(displacement.place(x * displacement.radial, y * displacement.radial));
            self.uvs.push([along * work.field.ramp_scale(), 0.5]);
        }
    }
}

/// Everything the per-vertex pass reads.
#[derive(Copy, Clone)]
pub struct VertexWork<'a> {
    /// The coordinate indexing the ramp, and its cycle count.
    pub field: PhaseField,
    /// The beam's spin knob.
    pub spin_speed: f32,
    /// Animations displacing geometry.
    pub warps: &'a [TargetedAnimation<PreparedAnimation>],
    /// Animations scaling an outline's width along the contour.
    pub taper: &'a [TargetedAnimation<PreparedAnimation>],
    /// The width an outline is drawn at where nothing tapers it, as a fraction
    /// of the width it was stroked at.
    pub stroke_width: f32,
}

impl VertexWork<'_> {
    /// How far a ribbon reaches at one point, as a fraction of the width its
    /// outline was stroked at.
    ///
    /// Never past one, so the width only ever narrows. Narrowing is what keeps
    /// the ribbon the shape it was cut as: an offset scaled down lands where a
    /// thinner stroke's own vertex would, and one scaled up would carry joins
    /// built for a stroke this wide out to a width they were not cut for.
    ///
    /// Never below zero either, and zero is reached rather than approached: a
    /// waveform at full amplitude troughs at exactly minus one, which is a
    /// ribbon of no width and a beam that has gone out.
    #[inline]
    fn taper_at(&self, along: f32, index: usize) -> f32 {
        // Summed the way a beam sums them, so a figure and a run of segments
        // answer a stack of thickness animations alike.
        let scale = self.taper.iter().fold(1.0, |acc, a| {
            acc + a.animation.value(Phase::new(f64::from(along)), index) as f32
        });
        (self.stroke_width * scale).clamp(0.0, 1.0)
    }
}

/// Which coordinates a frame's work actually asks for.
///
/// An angle costs an arctangent and a radius a square root, so this is decided
/// once for a layer rather than per point.
#[derive(Copy, Clone)]
struct Needs {
    angle: bool,
    radius: bool,
    rotates: bool,
}

impl Needs {
    fn of(work: &VertexWork) -> Self {
        let rotates =
            work.spin_speed != 0.0 || work.warps.iter().any(|w| w.target == AnimationTarget::Spin);
        Self {
            rotates,
            angle: rotates || work.field.phase == PhaseAxis::Angle,
            radius: rotates || work.field.phase == PhaseAxis::Radius,
        }
    }
}

/// What a point's warps work out to: a scale about the origin, a turn, a
/// squash, and an offset.
#[derive(Copy, Clone)]
struct Displacement {
    radial: f32,
    turn: f32,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Displacement {
    fn of(work: &VertexWork, polar: Polar, along: f32, index: usize) -> Self {
        let mut out = Self {
            radial: 1.0,
            // A tunnel integrates this knob into an angle that grows without
            // limit; a figure reads the same knob as the winding itself,
            // measured in turns at the rim. Winding does not wrap the way a
            // rotation does — five turns at the rim stays five turns tighter
            // than one — so a figure takes the amount and not a rate.
            turn: work.spin_speed * polar.radius * std::f32::consts::TAU,
            scale_x: 1.0,
            scale_y: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        for warp in work.warps {
            let value = warp.animation.value(Phase::new(f64::from(along)), index) as f32;
            // Where a target means something different on a figure than on a
            // run of segments, this is where it is reinterpreted. `Size` scales a
            // segment; here it scales each point's distance from the centre,
            // so run along the angle it deforms a disc into petals and along
            // the radius it pinches one into rings. `Spin` turns a segment about
            // its own centroid; a point has no orientation to turn, so the
            // same intent arrives as a shear growing with radius. A position
            // moves a whole run of segments at once; here each point moves on
            // its own, and the figure bends between them.
            match warp.target {
                // Multiplicative, so the deformation is proportional.
                AnimationTarget::Size => out.radial *= 1.0 + value,
                AnimationTarget::Spin => out.turn += value * MAX_SPIN,
                AnimationTarget::AspectRatio => {
                    out.scale_x *= 1.0 + value;
                    out.scale_y *= 1.0 - value;
                }
                // Additive, in the figure's own units, so a unit of it is a
                // unit of the figure — the same amount of travel whatever the
                // figure has been scaled to.
                AnimationTarget::PositionX => out.offset_x += value,
                AnimationTarget::PositionY => out.offset_y += value,
                // The rest never arrive. A colour target is answered once per
                // ramp texel; thickness is answered per vertex too, but on the
                // outline alone, where it scales the ribbon's reach rather
                // than moving the point; a rotation turns the whole figure and
                // is folded into the placement; and a marquee resolves into
                // nothing, a figure having no segments to slide along a path.
                AnimationTarget::Color
                | AnimationTarget::ColorSpread
                | AnimationTarget::ColorSaturation
                | AnimationTarget::Rotation
                | AnimationTarget::Thickness
                | AnimationTarget::MarqueeRotation => {}
            }
        }
        // A negative radius would turn the figure inside out through the
        // origin rather than collapsing it.
        out.radial = out.radial.max(0.0);
        out
    }

    /// The displacement where a figure has no coordinate: at the start of the
    /// cycle, and at the centre.
    ///
    /// A figure's placement is answered there, so this is the offset it already
    /// carries.
    fn anchor(work: &VertexWork) -> Self {
        Self::of(work, Polar::default(), 0.0, 0)
    }

    /// This displacement with the offset the placement already carries taken
    /// out, leaving only what deviates from it.
    ///
    /// A placement is in the units a position knob is in and a point is in the
    /// figure's own, so the two cannot be added together and the whole value
    /// cannot be spent twice. Measuring the points from the placement's own
    /// reading leaves a figure that varies nowhere exactly where it was put.
    fn beyond(self, anchor: Self) -> Self {
        Self {
            offset_x: self.offset_x - anchor.offset_x,
            offset_y: self.offset_y - anchor.offset_y,
            ..self
        }
    }

    /// Where a point lands once this displacement has been applied to it.
    ///
    /// The squash multiplies and the offset adds, in that order, so a figure
    /// flattened onto one axis is still moved the same distance along it.
    #[inline]
    fn place(self, x: f32, y: f32) -> Point {
        Point::new(
            x * self.scale_x + self.offset_x,
            y * self.scale_y + self.offset_y,
        )
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
    fn of(v: Point, angle: bool, radius: bool) -> Self {
        Self {
            radius: if radius { v.radius() } else { 0.0 },
            angle: if angle {
                fastmath::atan2(v.y(), v.x())
            } else {
                0.0
            },
        }
    }

    /// Unit phase along one coordinate, reusing what has already been computed.
    #[inline]
    fn phase(self, v: Point, phase: PhaseAxis) -> f32 {
        match phase {
            PhaseAxis::Angle => self.angle / std::f32::consts::TAU + 0.5,
            PhaseAxis::Radius => self.radius / std::f32::consts::SQRT_2,
            PhaseAxis::Linear => (v.y() + 1.0) / 2.0,
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
    verts: &VertexBuffers,
    color: [f32; 4],
    m: Matrix2d,
    gl: &mut G,
) {
    if mesh.is_empty() {
        return;
    }
    let mut pos = Vec::with_capacity(CHUNK);
    gl.tri_list(&DrawState::default(), &color, |f| {
        for batch in mesh.batches(CHUNK) {
            pos.clear();
            // A flat colour interpolates nothing, so the grouping into
            // triangles carries no meaning here.
            pos.extend(
                batch
                    .indices()
                    .iter()
                    .filter_map(|&i| verts.positions.get(i as usize).map(|v| project(m, *v))),
            );
            f(&pos);
        }
    });
}

/// Draw a flat run of triangles in one colour.
///
/// The path a figure takes when nothing varies across it: no ramp, no per-
/// vertex pass, and the tessellator's own triangles rather than a refined
/// mesh. An outline reaches it the same way when its colour is flat, and needs
/// no refined mesh even when it is not.
///
/// The backend takes a bounded number of vertices per call, and a run ending
/// mid-triangle would draw a torn one, so the cap is rounded down to a whole
/// number of triangles here rather than at each call site.
pub fn draw_points<G: Graphics>(points: &[Point], color: [f32; 4], m: Matrix2d, gl: &mut G) {
    if points.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    gl.tri_list(&DrawState::default(), &color, |f| {
        for batch in points.chunks(stride) {
            pos.clear();
            pos.extend(batch.iter().map(|v| project(m, *v)));
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
    verts: &VertexBuffers,
    period: Option<f32>,
    texture: &G::Texture,
    m: Matrix2d,
    gl: &mut G,
) {
    if mesh.is_empty() {
        return;
    }
    let mut pos = Vec::with_capacity(CHUNK);
    let mut uv = Vec::with_capacity(CHUNK);

    gl.tri_list_uv(&DrawState::default(), &[1.0; 4], texture, |f| {
        for batch in mesh.batches(CHUNK) {
            pos.clear();
            uv.clear();
            // Grouped by triangle here, because the seam shift is taken
            // against one corner's coordinate for the whole triangle.
            for tri in batch.triangles() {
                let corners = tri.map(|i| {
                    let i = i as usize;
                    Some((*verts.positions.get(i)?, *verts.uvs.get(i)?))
                });
                // A triangle naming a vertex the pass did not produce is
                // dropped entire, as it is where the tessellator's output is
                // resolved. Pushing the corners that do exist would shift
                // every later vertex in the run and tear the batch into
                // triangles nothing meshed.
                let [Some(a), Some(b), Some(c)] = corners else {
                    continue;
                };
                let reference = a.1[0];
                for (p, v) in [a, b, c] {
                    pos.push(project(m, p));
                    uv.push([
                        match period {
                            Some(period) => same_branch(reference, v[0], period),
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

/// Draw the vertex pass's own output in one colour.
pub fn draw_list_flat<G: Graphics>(
    verts: &VertexBuffers,
    color: [f32; 4],
    m: Matrix2d,
    gl: &mut G,
) {
    draw_points(&verts.positions, color, m, gl);
}

/// Draw a flat vertex list against the ramp texture.
pub fn draw_list_textured<G: Graphics>(
    verts: &VertexBuffers,
    period: Option<f32>,
    texture: &G::Texture,
    m: Matrix2d,
    gl: &mut G,
) {
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    let mut uv = Vec::with_capacity(stride);
    gl.tri_list_uv(&DrawState::default(), &[1.0; 4], texture, |f| {
        for (batch, uvs) in verts.positions.chunks(stride).zip(verts.uvs.chunks(stride)) {
            pos.clear();
            uv.clear();
            for (tri, tri_uv) in batch.as_chunks::<3>().0.iter().zip(tri_uvs(uvs)) {
                let reference = tri_uv[0][0];
                for (p, v) in tri.iter().zip(tri_uv) {
                    pos.push(project(m, *p));
                    uv.push([
                        match period {
                            Some(period) => same_branch(reference, v[0], period),
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

fn tri_uvs(uvs: &[[f32; 2]]) -> impl Iterator<Item = [[f32; 2]; 3]> + '_ {
    uvs.as_chunks::<3>().0.iter().copied()
}

/// Apply a 2D affine matrix to a point, leaving figure space for the backend's.
///
/// The return type is deliberately bare: past here the values are the
/// backend's, in its coordinates and its layout, and losing [`Point`] is the
/// signal that they are no longer ours to reason about. Piston takes
/// pre-transformed vertices, so this is the vertex shader a fixed pipeline
/// does not have.
#[inline]
fn project(m: Matrix2d, v: Point) -> [f32; 2] {
    let (x, y) = (f64::from(v.x()), f64::from(v.y()));
    [
        (m[0][0] * x + m[0][1] * y + m[0][2]) as f32,
        (m[1][0] * x + m[1][1] * y + m[1][2]) as f32,
    ]
}

#[cfg(test)]
mod test {
    use super::*;
    use tunnels_lib::number::UnipolarFloat;
    use tunnels_model::animation::Animation;
    use tunnels_model::clock_bank::ClockBank;

    /// An animation aimed at `target`. Its value is never asked for here; only
    /// what it is aimed at decides which coordinates a layer pays for.
    fn warp(target: AnimationTarget) -> TargetedAnimation<PreparedAnimation> {
        TargetedAnimation {
            animation: Animation::default().prepare(&ClockBank::default(), UnipolarFloat::ZERO),
            target,
        }
    }

    /// An angle costs an arctangent and a radius a square root, so a layer
    /// should ask for a coordinate only where something reads it.
    #[test]
    fn a_layer_asks_only_for_the_coordinates_something_reads() {
        let needs = |phase, spin_speed, targets: &[AnimationTarget]| {
            let warps: Vec<_> = targets.iter().copied().map(warp).collect();
            Needs::of(&VertexWork {
                field: PhaseField {
                    phase,
                    cycles: 1.0,
                    span: RampSpan::Cycle,
                },
                spin_speed,
                warps: &warps,
                taper: &[],
                stroke_width: 1.0,
            })
        };

        // Size and aspect ratio deform a point by the animation's value along
        // the phase coordinate. Neither reads where the point is.
        let deformed = needs(
            PhaseAxis::Linear,
            0.0,
            &[AnimationTarget::Size, AnimationTarget::AspectRatio],
        );
        assert!(!deformed.radius, "a deformed layer asked for the radius");
        assert!(!deformed.angle, "a deformed layer asked for the angle");
        assert!(!deformed.rotates);

        // The radius is read where the colour runs along it, and wherever the
        // figure turns — a spin is a rotation about the centre the radius is
        // measured from.
        assert!(needs(PhaseAxis::Radius, 0.0, &[]).radius);
        assert!(needs(PhaseAxis::Linear, 0.5, &[]).radius);
        assert!(needs(PhaseAxis::Linear, 0.0, &[AnimationTarget::Spin]).radius);

        // The angle likewise.
        assert!(needs(PhaseAxis::Angle, 0.0, &[]).angle);
        assert!(!needs(PhaseAxis::Linear, 0.0, &[]).angle);
        assert!(needs(PhaseAxis::Linear, 0.0, &[AnimationTarget::Spin]).angle);
    }

    /// An animation aimed at `target`, shaped so that its value runs across the
    /// figure rather than standing at one number.
    fn varying_warp(target: AnimationTarget) -> TargetedAnimation<PreparedAnimation> {
        use std::time::Duration;
        use tunnels_model::animation::{ControlMessage, StateChange, Waveform};

        struct Noop;
        impl tunnels_model::animation::EmitStateChange for Noop {
            fn emit_animation_state_change(&mut self, _: StateChange) {}
        }

        let mut animation = Animation::default();
        for sc in [
            StateChange::Waveform(Waveform::Sawtooth),
            StateChange::NPeriods(1),
            StateChange::Size(UnipolarFloat::ONE),
            // Unsmoothed, so the ramp is the straight line the arithmetic here
            // is written against rather than one with its corners rounded off.
            StateChange::Smoothing(UnipolarFloat::ZERO),
        ] {
            animation.control(ControlMessage::Set(sc), &mut Noop);
        }
        // Smoothing is reached over time rather than set. The animation runs at
        // no speed, so nothing else moves while it gets there.
        animation.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        TargetedAnimation {
            animation: animation.prepare(&ClockBank::default(), UnipolarFloat::ZERO),
            target,
        }
    }

    /// A position warp displaces a point by how far the animation departs from
    /// the value the placement already took, and by nothing else.
    ///
    /// The placement is answered where a figure has no coordinate — at the
    /// start of the cycle — so that is where the deviation is measured from.
    /// A point read there is left exactly where an unwarped one would be,
    /// which is what keeps a figure placed where it was put.
    #[test]
    fn a_position_warp_displaces_a_point_by_what_it_deviates() {
        let warps = [
            varying_warp(AnimationTarget::PositionX),
            varying_warp(AnimationTarget::PositionY),
        ];
        let work = VertexWork {
            field: PhaseField {
                phase: PhaseAxis::Linear,
                cycles: 1.0,
                span: RampSpan::Cycle,
            },
            spin_speed: 0.0,
            warps: &warps,
            taper: &[],
            stroke_width: 1.0,
        };
        let anchor = Displacement::anchor(&work);
        let at = |along| Displacement::of(&work, Polar::default(), along, 0).beyond(anchor);

        let start = at(0.0);
        assert_eq!(
            (start.offset_x, start.offset_y),
            (0.0, 0.0),
            "the point the placement was answered at moved"
        );

        // A sawtooth a quarter of the way along is halfway up its rise, which
        // is a displacement of half a figure-space unit on both axes.
        let quarter = at(0.25);
        assert_eq!(quarter.offset_x, 0.5);
        assert_eq!(quarter.offset_y, 0.5);

        // Nothing else the warps could have touched moved with them.
        assert_eq!(quarter.radial, 1.0);
        assert_eq!(quarter.turn, 0.0);
        assert_eq!((quarter.scale_x, quarter.scale_y), (1.0, 1.0));
    }

    /// The offset lands on the point after everything that scales it, so a
    /// figure squashed by an aspect animation is displaced by the same
    /// distance a round one is.
    #[test]
    fn an_offset_moves_a_point_after_the_scales() {
        let displacement = Displacement {
            radial: 1.0,
            turn: 0.0,
            scale_x: 2.0,
            scale_y: 0.5,
            offset_x: 0.25,
            offset_y: -0.75,
        };
        assert_eq!(
            displacement.place(3.0, 4.0),
            Point::new(3.0 * 2.0 + 0.25, 4.0 * 0.5 - 0.75)
        );
    }

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

    /// The seam `atan2` leaves has to be closed wherever the angle actually
    /// indexes the ramp. A figure-wide table is indexed by the raw angle, so it
    /// wraps at one turn even where the colour makes no cycles at all — which
    /// is the setting an animation sweeping a figure of one colour runs at.
    #[test]
    fn the_angular_seam_closes_wherever_the_angle_is_read() {
        let field = |cycles, span| PhaseField {
            phase: PhaseAxis::Angle,
            cycles,
            span,
        };

        assert_eq!(field(3.0, RampSpan::Cycle).wrap_period(), Some(3.0));
        assert_eq!(
            field(0.0, RampSpan::Cycle).wrap_period(),
            None,
            "scaled to nothing, every vertex reads one texel"
        );
        assert_eq!(field(3.0, RampSpan::Figure).wrap_period(), Some(1.0));
        assert_eq!(
            field(0.0, RampSpan::Figure).wrap_period(),
            Some(1.0),
            "one colour still sweeps across the figure under an animation"
        );

        // The continuous coordinates have no seam to close.
        for phase in [PhaseAxis::Radius, PhaseAxis::Linear] {
            for span in [RampSpan::Cycle, RampSpan::Figure] {
                let field = PhaseField {
                    phase,
                    cycles: 3.0,
                    span,
                };
                assert_eq!(field.wrap_period(), None, "{phase:?} {span:?}");
            }
        }
    }
}
