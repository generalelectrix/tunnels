//! The drawable geometry a beam expands into.
//!
//! This is the far end of the model: everything above it describes a show,
//! and everything here describes shapes on a screen.

use crate::animation::{PreparedAnimation, TargetedAnimation};
use crate::waveforms::{WaveformArgs, sawtooth};
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use tunnels_lib::number::{Phase, UnipolarFloat};
use tunnels_shapes::{Arity, Secondary, ShapeFamily};

/// Controls how a shape is rendered.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum RenderMode {
    /// Render as an arc segment (default).
    #[default]
    Arc,
    /// Render as a filled circle positioned at the arc segment centroid.
    Dot,
    /// Render as a filled ellipse at the arc centroid, sized by chord and thickness.
    Saucer,
}

/// What a beam draws, and how its geometry parameters are read.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum ShapeMode {
    /// Segments are distributed along an ellipse (default).
    #[default]
    Ellipse,
    /// Segments are distributed along a straight line.
    Line,
    /// A filled figure computed from the beam's own parameters.
    Generated,
    /// A filled figure baked into the build.
    Sprite,
}

impl ShapeMode {
    /// The path this mode's segments are distributed along, or `None` if it
    /// fills an area instead of drawing segments.
    pub fn segment_path(self) -> Option<SegmentPath> {
        match self {
            Self::Ellipse => Some(SegmentPath::Ellipse),
            Self::Line => Some(SegmentPath::Line),
            Self::Generated | Self::Sprite => None,
        }
    }

    /// The library this mode's figure comes from, or `None` if it draws
    /// segments instead of filling an area.
    ///
    /// The other half of the pair [`ShapeMode::segment_path`] opens: a mode
    /// either distributes marks along a path or names a figure in a library,
    /// and which of the two it does decides what every geometry control means.
    pub fn figure_library(self) -> Option<FigureLibrary> {
        match self {
            Self::Ellipse | Self::Line => None,
            Self::Sprite => Some(FigureLibrary::Baked),
            Self::Generated => Some(FigureLibrary::Generated),
        }
    }

    /// Whether this mode draws a run of segments.
    ///
    /// The controls that act on segments — the marquee and the render mode —
    /// mean nothing to a mode that draws none.
    pub fn draws_segments(self) -> bool {
        self.segment_path().is_some()
    }
}

/// Where a figure mode's figures come from.
///
/// The two libraries are shaped alike for a control to walk — a list of
/// families, each a run of figures — which is what lets one pair of knobs
/// address either.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FigureLibrary {
    /// Figures baked into the build from artwork.
    Baked,
    /// Figures built from a family and the two numbers that place one in it.
    Generated,
}

/// The curve a run of segments is distributed along.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SegmentPath {
    Ellipse,
    Line,
}

/// The coordinate of a figure that anything varying across it is read along.
///
/// One axis serves the colour ramp, an animation's phase and a taper alike:
/// each asks how far along the figure a point lies and takes the same answer,
/// so a pattern, a distortion and a thickness run together rather than along
/// axes chosen apart.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum PhaseAxis {
    /// The angle about the figure's center (default).
    #[default]
    Angle,
    /// The distance from the figure's center.
    Radius,
    /// The figure's own vertical coordinate.
    ///
    /// The frame is the figure's rather than the screen's, so this axis turns
    /// with the figure: a ramp along any other axis is this one plus a
    /// rotation, which is why there is only one linear phase.
    Linear,
}

/// How much of a figure is painted.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum DrawMode {
    /// The interior only (default).
    #[default]
    Fill,
    /// The contours only, stroked at the beam's thickness.
    Outline,
    /// The interior with its contours stroked over it.
    Both,
}

impl DrawMode {
    pub fn draws_fill(self) -> bool {
        matches!(self, Self::Fill | Self::Both)
    }

    pub fn draws_outline(self) -> bool {
        matches!(self, Self::Outline | Self::Both)
    }
}

/// What a beam's shapes do to the frame they are drawn into.
///
/// A beam either paints in its own colours, or paints opaque black on one side
/// or the other of the shapes it draws. The two black modes are the same
/// operation about opposite sides of the same outline: a mask blacks the
/// inside, so what is under it is hidden where the shapes fall; a gobo blacks
/// the outside, so what is under it survives only where the shapes fall and the
/// beam becomes a window rather than a hole.
///
/// Black is painted rather than clipped, so what the mode does is done by the
/// time the next beam draws. A beam drawn after a gobo paints over its black
/// exactly as it paints over a mask's.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub enum PaintMode {
    /// The beam's own colours (default).
    #[default]
    Normal,
    /// Opaque black where the beam's shapes are.
    Mask,
    /// Opaque black everywhere the beam's shapes are not.
    Gobo,
}

impl PaintMode {
    /// This mode standing in for another's.
    ///
    /// A beam drawn in black draws everything inside it in black too, so a
    /// composition put into one of the black modes paints that way throughout
    /// rather than letting its parts each decide. `Normal` is the absence of
    /// such an imposition, and yields to whatever the inner beam asks for.
    pub fn over(self, inner: Self) -> Self {
        match self {
            Self::Normal => inner,
            imposed => imposed,
        }
    }

    /// Whether this mode paints opaque black rather than the beam's colours.
    pub fn paints_black(self) -> bool {
        !matches!(self, Self::Normal)
    }
}

/// Identifies one figure baked into the build.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct SpriteId(pub u16);

/// Identifies one figure computed on demand.
///
/// A family and the two numbers that place a figure inside it, which is the
/// whole of what builds one. Every other number a family carries follows from
/// these, so this is the figure and not a handle to it.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GeneratedId {
    pub family: ShapeFamily,
    pub arity: Arity,
    pub secondary: Secondary,
}

impl GeneratedId {
    /// Every figure the generated library holds.
    ///
    /// Each family's offered arities at the one secondary that family is drawn
    /// at, which is the same set the two figure knobs address. Naming it is
    /// what lets a client build the whole of it before a show rather than
    /// tessellating each figure the first time an operator lands on it.
    pub fn library() -> impl Iterator<Item = Self> {
        ShapeFamily::ALL.into_iter().flat_map(|family| {
            family.arities().iter().map(move |&arity| Self {
                family,
                arity,
                secondary: family.secondary(),
            })
        })
    }
}

impl Default for GeneratedId {
    /// The first figure of the first family, which is what a beam draws before
    /// any knob has named another.
    fn default() -> Self {
        let family = ShapeFamily::ALL[0];
        Self {
            family,
            arity: family.arities().first().copied().unwrap_or(Arity::new(0)),
            secondary: family.secondary(),
        }
    }
}

/// Identifies one figure, however it came to exist.
///
/// A figure is drawn the same way whichever half of the library it came from,
/// so this is what the caches between the model and the screen are keyed on:
/// one figure, one set of contours, one mesh per density.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FigureId {
    /// A figure baked into the build from artwork.
    Baked(SpriteId),
    /// A figure built from a family and its two parameters.
    Generated(GeneratedId),
}

/// A command to draw a single shape, less the render mode and segment path
/// that the layer holding it fixes for all of its shapes at once.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct ShapeGeometry {
    pub color: Hsva,
    pub placement: Placement,
    pub thickness: f64,
    /// Where the shape starts along the layer's path, in turns.
    pub start: f64,
    pub spin_angle: f64,
}

/// A run of shapes drawn the same way.
///
/// The render mode and segment path apply to every shape in the layer, which
/// is what makes a layer the unit a renderer can dispatch on once instead of
/// per shape.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SegmentLayer {
    pub render_mode: RenderMode,
    pub segment_path: SegmentPath,
    pub mode: PaintMode,
    /// The angular width every segment in this layer spans, in turns.
    ///
    /// A segment's stop angle is its `start` plus this, so a segment that
    /// closes into a full circle spans exactly one turn — a test the renderer
    /// can make without subtracting two nearly equal angles.
    pub span: f64,
    pub shapes: Vec<ShapeGeometry>,
}

impl SegmentLayer {
    pub fn new(
        render_mode: RenderMode,
        segment_path: SegmentPath,
        mode: PaintMode,
        span: f64,
        shapes: Vec<ShapeGeometry>,
    ) -> Self {
        Self {
            render_mode,
            segment_path,
            mode,
            span,
            shapes,
        }
    }
}

/// Where a shape sits, how large it is, and which way it is turned.
///
/// The half-extents mean whatever the shape they place reads them as: the two
/// radii of an ellipse, the half-length and offset of a line, or the box a
/// figure's own unit square is scaled into.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub extent_x: f64,
    pub extent_y: f64,
    pub rot_angle: f64,
}

/// How a figure's colour is resolved from a coordinate on it.
///
/// This is a tunnel's colour model with a figure's coordinate standing in for
/// the segment index: `hue = center + 0.5 * width * sawtooth(phase * cycles)`.
/// A closed figure has no segments, so [`PhaseAxis`] picks what does the
/// indexing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct ColorField {
    pub phase: PhaseAxis,
    /// Whole colour cycles across the figure, already floored.
    pub cycles: f64,
    pub center: f64,
    pub width: f64,
    pub sat: f64,
    pub val: f64,
    pub level: f64,
}

impl ColorField {
    /// Whether every point of the figure resolves to the same colour.
    ///
    /// A uniform figure needs no ramp and no interpolation, which is also the
    /// state a mask is in. An animation on the colour can still move that one
    /// colour over time, so this is not on its own a reason to skip the ramp.
    pub fn is_uniform(&self) -> bool {
        self.width == 0.0 || self.cycles == 0.0
    }

    /// Whether this field masks: opaque black everywhere, whatever is asked of
    /// it.
    ///
    /// Every channel a colour resolves to is scaled by the value, so a field
    /// with no value paints black at any point and under any colour animation
    /// -- hue and saturation are multiplied away before they can reach a
    /// pixel. That is what lets a mask be resolved once instead of per point
    /// or per texel.
    ///
    /// The three adjustments an animation makes -- centre, width, saturation
    /// -- are what this rests on. A target that moved the value would break
    /// it, and there is none.
    pub fn is_mask(&self) -> bool {
        self.val == 0.0
    }

    /// The colour at a point of one cycle.
    ///
    /// This is a tunnel's own hue expression with the cycle count taken out:
    /// the count multiplies the coordinate rather than the table, so one cycle
    /// is all a table has to hold and a figure's colour reads the same as a
    /// beam's at the same knob settings.
    pub fn sample(&self, phase: Phase, adjust: ColorAdjust) -> Hsva {
        let hue = Phase::new(
            (self.center + adjust.center)
                + 0.5
                    * (self.width + adjust.width)
                    * sawtooth(&WaveformArgs {
                        phase_spatial: phase,
                        phase_temporal: Phase::ZERO,
                        smoothing: UnipolarFloat::ZERO,
                        duty_cycle: UnipolarFloat::ONE,
                        pulse: false,
                        standing: false,
                    }),
        );
        Hsva {
            hue: hue.val(),
            sat: UnipolarFloat::new(self.sat + adjust.sat).val(),
            val: self.val,
            level: self.level,
        }
    }
}

/// What animations add to a colour before it is resolved.
#[derive(Debug, Clone, Copy, Default)]
pub struct ColorAdjust {
    pub center: f64,
    pub width: f64,
    pub sat: f64,
}

/// A resolved colour, and the level it is drawn at.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    /// Alpha, carrying the channel's own level.
    pub level: f64,
}

/// A figure drawn as an area rather than as a run of segments.
///
/// Carries no geometry: a figure is named rather than described, and the client
/// is where the name becomes contours, so what travels is where to put it and
/// how to colour it.
#[derive(Debug, Clone)]
pub struct FillLayer {
    pub figure: FigureId,
    pub placement: Placement,
    /// The beam's spin knob, as the operator set it.
    pub spin_speed: f64,
    /// The width an outline is stroked at where nothing tapers it, in the same
    /// units a segment's thickness is.
    pub thickness: f64,
    pub draw_mode: DrawMode,
    pub mode: PaintMode,
    pub color: ColorField,
    /// Animations resolved when the colour ramp is built, once per texel.
    pub color_anims: Vec<TargetedAnimation<PreparedAnimation>>,
    /// Animations resolved per point of the figure, displacing it.
    pub warps: Vec<TargetedAnimation<PreparedAnimation>>,
    /// Animations resolved per point of an outline, scaling its width there.
    ///
    /// Kept apart from the warps because they answer different questions about
    /// the same point: a warp says where it goes, and these say how far the
    /// ribbon reaches either side of it. Only an outline has the second, so a
    /// figure that is filled and not stroked ignores these entirely.
    pub taper: Vec<TargetedAnimation<PreparedAnimation>>,
}

impl FillLayer {
    /// Whether anything displaces the figure's points.
    ///
    /// A figure nothing displaces is drawn from the tessellator's own
    /// triangles, with no per-vertex pass at all; a displaced one needs the
    /// refined mesh the displacement lives on, which is more triangles and a
    /// different tessellation of the same region.
    pub fn warps_points(&self) -> bool {
        self.spin_speed != 0.0 || !self.warps.is_empty()
    }
}

/// What a beam expands into for one frame.
#[derive(Debug, Clone)]
pub enum Layer {
    /// A run of segments along a path.
    Segments(SegmentLayer),
    /// A filled figure.
    Fill(FillLayer),
}

impl Layer {
    /// What this layer's shapes do to the frame they are drawn into.
    pub fn mode(&self) -> PaintMode {
        match self {
            Self::Segments(l) => l.mode,
            Self::Fill(l) => l.mode,
        }
    }

    /// Whether this layer would draw nothing, and so can be dropped before it
    /// reaches a renderer.
    ///
    /// A gobo is never this. What it draws is black everywhere its shapes are
    /// not, so a run left with no shapes blacks the frame entire — the most it
    /// can draw rather than the least, and not something to drop.
    ///
    /// A run reaches that state by blacking taking every segment away. It is
    /// not how a thickness animation closes a gobo's window: thickness is a
    /// field each shape carries, so winding it to nothing leaves the shapes
    /// where they are and the run is never empty. That window closes in the
    /// renderer, where shapes of no thickness tessellate to nothing.
    pub fn is_empty(&self) -> bool {
        if self.mode() == PaintMode::Gobo {
            return false;
        }
        match self {
            Self::Segments(l) => l.shapes.is_empty(),
            // A figure is one shape and is always there; whether the build
            // carries the figure it names is the renderer's question.
            Self::Fill(_) => false,
        }
    }
}

pub type LayerCollection = Vec<Layer>;

#[cfg(test)]
mod test {
    use super::*;

    /// An empty run draws nothing and is dropped, unless it is a gobo — a gobo
    /// with no shapes blacks the frame entire rather than drawing nothing.
    ///
    /// A run is emptied by blacking, not by a thickness animation, which
    /// leaves its shapes in place carrying no thickness. So this covers only
    /// one of the ways a gobo's window closes; the renderer covers the other,
    /// and a channel off at its upfader emits no layer for either to be asked
    /// about.
    #[test]
    fn an_empty_run_is_dropped_unless_it_is_a_gobo() {
        let run = |mode| {
            Layer::Segments(SegmentLayer::new(
                RenderMode::default(),
                SegmentPath::Ellipse,
                mode,
                1.0,
                Vec::new(),
            ))
        };
        for mode in [PaintMode::Normal, PaintMode::Mask] {
            assert!(
                run(mode).is_empty(),
                "an empty run in {mode:?} draws nothing and can be dropped"
            );
        }
        assert!(
            !run(PaintMode::Gobo).is_empty(),
            "an empty gobo blacks the frame, which is the most it can draw"
        );
    }

    /// A composition drawn in a black mode imposes it on everything inside it,
    /// and `Normal` imposes nothing.
    ///
    /// This is what makes each channel of a gobo'd look a gobo in its own
    /// right, and so what makes such a look come out as the intersection of
    /// its figures rather than their union.
    #[test]
    fn an_imposed_mode_overrides_a_beams_own() {
        use PaintMode::{Gobo, Mask, Normal};
        for inner in [Normal, Mask, Gobo] {
            assert_eq!(
                Normal.over(inner),
                inner,
                "a look in no particular mode leaves {inner:?} alone"
            );
            for imposed in [Mask, Gobo] {
                assert_eq!(
                    imposed.over(inner),
                    imposed,
                    "a {imposed:?} look draws a {inner:?} channel as {imposed:?}"
                );
            }
        }
        assert!(!Normal.paints_black());
        assert!(Mask.paints_black());
        assert!(Gobo.paints_black());
    }

    /// A mask is resolved once for the whole layer rather than once per point.
    /// That is only sound if no adjustment an animation can make reaches the
    /// result.
    #[test]
    fn no_adjustment_moves_what_a_mask_paints() {
        let mask = ColorField {
            phase: PhaseAxis::Angle,
            cycles: 0.,
            center: 0.,
            width: 0.,
            sat: 0.,
            val: 0.,
            level: 1.,
        };
        assert!(mask.is_mask());

        let flat = mask.sample(Phase::ZERO, ColorAdjust::default());
        for adjust in [
            ColorAdjust {
                center: 0.4,
                width: 0.,
                sat: 0.,
            },
            ColorAdjust {
                center: 0.,
                width: 1.,
                sat: 0.,
            },
            ColorAdjust {
                center: 0.,
                width: 0.,
                sat: 1.,
            },
            ColorAdjust {
                center: 0.9,
                width: 1.,
                sat: 1.,
            },
        ] {
            for phase in [0., 0.25, 0.5, 0.75] {
                let sampled = mask.sample(Phase::new(phase), adjust);
                assert_eq!(
                    sampled.val, flat.val,
                    "a mask gained a value from {adjust:?}"
                );
                assert_eq!(
                    sampled.level, flat.level,
                    "a mask changed alpha under {adjust:?}"
                );
            }
        }

        // A field with a value does move, so the test above is not vacuous.
        let lit = ColorField { val: 1., ..mask };
        assert!(!lit.is_mask());
        assert_ne!(
            lit.sample(Phase::ZERO, ColorAdjust::default()).val,
            flat.val
        );
    }

    /// A figure reaches a render client as its name and nothing else, so the
    /// name has to arrive as it left.
    ///
    /// A generated name is a family and two numbers rather than an index, and
    /// one of the two is a position read back through the constructor that
    /// bounds it — written as two halves that have to agree. A name that
    /// decoded to another name would draw another figure, and the encoding
    /// carries no schema that would notice.
    #[test]
    fn a_figure_name_arrives_as_it_left() {
        for id in [
            FigureId::Baked(SpriteId(0)),
            FigureId::Baked(SpriteId(37)),
            FigureId::Generated(GeneratedId::default()),
            FigureId::Generated(GeneratedId {
                family: ShapeFamily::MoireWeave,
                arity: Arity::new(11),
                secondary: Secondary::new(0.375),
            }),
            FigureId::Generated(GeneratedId {
                family: ShapeFamily::StarLattice85,
                arity: Arity::new(1),
                secondary: Secondary::new(1.0),
            }),
        ] {
            let bytes = postcard::to_allocvec(&id).expect("a figure name encodes");
            let back: FigureId = postcard::from_bytes(&bytes).expect("a figure name decodes");
            assert_eq!(back, id, "{id:?} came back as {back:?}");
        }
    }

    /// A position off the wire is bounded like any other.
    ///
    /// The encoding carries no schema, so what arrives is whatever bytes
    /// arrived, and a family resolves its second degree of freedom against a
    /// position between zero and one. Bytes naming anything else are brought
    /// back into that range rather than reaching a generator.
    #[test]
    fn a_position_from_the_wire_is_still_a_position() {
        for (bytes, expected) in [
            (4.5f64, Secondary::new(1.0)),
            (-2.0, Secondary::new(0.0)),
            (f64::NAN, Secondary::new(0.0)),
        ] {
            let encoded = postcard::to_allocvec(&bytes).expect("a float encodes");
            let position: Secondary = postcard::from_bytes(&encoded).expect("a position decodes");
            assert_eq!(
                position, expected,
                "{bytes} came off the wire as {position:?}"
            );
        }
    }
}
