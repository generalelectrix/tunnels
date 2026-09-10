//! Drawing a filled figure.
//!
//! The split this module is built around: **the mesh carries phase and a
//! texture carries colour.** Where a point sits in the colour cycle is a
//! function of its position alone, so a mesh built for a figure at a size holds
//! for every colour that figure can take — including one changing every frame.
//! Colour is then resolved per fragment against a ramp texture, which is what
//! lets the sawtooth's jump land exactly where it belongs however coarse the
//! mesh is.

mod draw;
mod fastmath;
mod figure;
mod geom;
mod geometry;
mod mesh;
mod ramp;

use self::draw::{PhaseField, VertexBuffers, VertexWork, draw_flat, draw_textured};
use self::figure::FigureCache;
use self::geometry::{FillGeometry, StrokeGeometry};
use self::mesh::{Level, MeshId, MeshLibrary};
use self::ramp::RampSpan;
use crate::draw::{draw_segments, hsv_to_rgb, place, thickness_px};
use client_lib::config::ClientConfig;
use graphics::math::Matrix2d;
use graphics::types::Color;
use graphics::{Context, Graphics, Transformed};
use image::RgbaImage;
use log::{error, info};
use texture::{CreateTexture, Filter, Format, TextureSettings, UpdateTexture, Wrap};
use tunnels_lib::number::Phase;
use tunnels_model::layer::{
    ColorAdjust, FigureId, FillLayer, GeneratedId, Layer, LayerCollection, SpriteId,
};

/// How many frames a texture may still be read after the last draw that used
/// it.
///
/// Overwriting a texture the GPU is still sampling makes the driver stall
/// until the queued draws that read it retire. Measured on the prototype, that
/// stall was two orders of magnitude larger than building the ramp in the
/// first place. A texture is not written again until this many frames have
/// passed since the draw that used it — write at frame 1, reuse at frame 4,
/// which is what rotating three buffers amounts to.
const FRAMES_IN_FLIGHT: u64 = 3;

/// How many frames a renderer has drawn.
///
/// One count serves every cache that acts on how long ago something was used,
/// so "three frames ago" means the same to all of them and none keeps a clock
/// of its own. It advances once per drawn frame and never runs backwards,
/// which is the whole of what those caches ask of it.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Frame(u64);

impl Frame {
    fn advance(&mut self) {
        self.0 += 1;
    }

    /// Frames passed since `then`, saturating at zero for a count that has not
    /// happened yet.
    fn since(self, then: Self) -> u64 {
        self.0.saturating_sub(then.0)
    }
}

/// One colour ramp on the GPU, and when it was last drawn with.
struct RampEntry<T> {
    texture: T,
    last_used: Frame,
    /// A texture is created at a width and keeps it, so an entry can only be
    /// taken back for a ramp of the same span.
    texels: u32,
}

/// The colour ramp textures, recycled once the GPU is done reading them.
///
/// **Not a cache.** A ramp is one evaluation per texel: across the cycle-wide
/// table's 1024, 22 µs still and 109 µs with three colour animations running
/// the worst waveform, against an 8.3 ms frame — and in proportion to that
/// across the figure-wide table, which is four times as long. Rebuilding it
/// every frame is affordable, and every frame that animates a colour has to
/// rebuild it anyway, so remembering the answer only ever accelerated the case
/// that was already free.
///
/// What this exists for is the write-after-read hazard, which is a different
/// problem and does not go away: overwriting a texture the GPU is still
/// sampling stalls the driver until the queued draws that read it retire,
/// measured on the prototype at two orders of magnitude more than building the
/// ramp costs. So a texture is handed out, written once, and not touched again
/// until [`FRAMES_IN_FLIGHT`] frames have passed without a draw using it.
///
/// The pool sizes itself and needs no cap: it grows to however many layers
/// draw in one frame times the frames in flight, and stops, because past that
/// there is always an entry old enough to take back. A layer whose colour is
/// animated across the figure needs a wider ramp than one whose is not, so that
/// bound is per width; there are two.
struct RampPool<T> {
    entries: Vec<RampEntry<T>>,
    /// Where a ramp is built before it is uploaded, one image per span.
    ///
    /// One image between them would be resized to each span in turn for as
    /// long as two layers of different spans are both on screen, which is a
    /// figure-wide image freed and reallocated every frame.
    cycle_scratch: RgbaImage,
    figure_scratch: RgbaImage,
    settings: TextureSettings,
}

impl<T> RampPool<T>
where
    T: CreateTexture<()> + UpdateTexture<()>,
{
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            cycle_scratch: ramp::blank(RampSpan::Cycle),
            figure_scratch: ramp::blank(RampSpan::Figure),
            // Repeating wrap is what lets a phase coordinate run past one
            // cycle and keep indexing the ramp, so the cycle count never
            // reaches the mesh. Linear filtering puts the sawtooth's jump
            // inside one texel.
            settings: TextureSettings::new()
                .filter(Filter::Linear)
                .wrap_u(Wrap::Repeat)
                .wrap_v(Wrap::Repeat),
        }
    }

    /// This layer's colour, built into a texture nothing is still reading.
    ///
    /// `None` only if the backend refused to give up a texture, which is a
    /// reason to skip a figure rather than to stop the show.
    fn texture_for(&mut self, fill: &FillLayer, span: RampSpan, frame: Frame) -> Option<&T> {
        let Self {
            entries,
            cycle_scratch,
            figure_scratch,
            settings,
        } = self;
        let scratch = match span {
            RampSpan::Cycle => cycle_scratch,
            RampSpan::Figure => figure_scratch,
        };
        ramp::build_into(scratch, span, &fill.color, &fill.color_anims);
        let (width, height) = scratch.dimensions();

        let free = entries
            .iter()
            .position(|e| e.texels == width && frame.since(e.last_used) >= FRAMES_IN_FLIGHT);
        let index = match free {
            Some(index) => index,
            None => {
                match T::create(
                    &mut (),
                    Format::Rgba8,
                    scratch.as_raw(),
                    [width, height],
                    settings,
                ) {
                    Ok(texture) => entries.push(RampEntry {
                        texture,
                        last_used: frame,
                        texels: width,
                    }),
                    Err(e) => {
                        error!("Could not make a colour ramp texture: {e:?}");
                        return None;
                    }
                }
                entries.len() - 1
            }
        };

        let entry = entries.get_mut(index)?;
        if let Err(e) = entry.texture.update(
            &mut (),
            Format::Rgba8,
            scratch.as_raw(),
            [0, 0],
            [width, height],
        ) {
            error!("Could not update a colour ramp texture: {e:?}");
            return None;
        }
        entry.last_used = frame;
        Some(&entry.texture)
    }
}

/// Everything a frame needs to draw layers, and the caches behind it.
///
/// Generic over the texture type so the GL client and the golden-image tests
/// share one draw path rather than each having its own.
pub struct Renderer<T> {
    figures: FigureCache,
    fills: FillGeometry,
    outlines: StrokeGeometry,
    meshes: MeshLibrary,
    ramps: RampPool<T>,
    /// What the caches measure age against.
    frame: Frame,
    /// Scratch for the per-vertex pass, reused across every layer and every
    /// frame. Layers draw one after another and none of this outlives the
    /// draw that fills it, so one buffer serves all of them.
    verts: VertexBuffers,
}

impl<T> Default for Renderer<T>
where
    T: CreateTexture<()> + UpdateTexture<()>,
{
    fn default() -> Self {
        Self {
            figures: FigureCache::default(),
            fills: FillGeometry::default(),
            outlines: StrokeGeometry::default(),
            meshes: MeshLibrary::default(),
            ramps: RampPool::new(),
            frame: Frame::default(),
            verts: VertexBuffers::default(),
        }
    }
}

impl<T> Renderer<T>
where
    T: CreateTexture<()> + UpdateTexture<()>,
{
    /// Draw a video channel's layers, in the order they were mixed.
    ///
    /// Order is the whole of how masking works — a mask paints opaque black
    /// over what is already there — so this never reorders or groups them.
    pub fn draw<G: Graphics<Texture = T>>(
        &mut self,
        layers: &LayerCollection,
        c: &Context,
        gl: &mut G,
        cfg: &ClientConfig,
    ) {
        self.frame.advance();
        self.meshes.reap(self.frame);
        for layer in layers {
            match layer {
                Layer::Segments(segments) => draw_segments(segments, c, gl, cfg),
                Layer::Fill(fill) => self.draw_fill(fill, c, gl, cfg),
            }
        }
    }

    /// Tessellate every figure both libraries hold, before the show starts.
    ///
    /// **What is expensive to build is built once; what is expensive to hold
    /// is held only while it is used.** Those are two different things, and
    /// one word used to cover both. Turning a figure's contours into triangles
    /// is slow and small — 1.5 ms on the average figure and 49 ms at the
    /// worst, against 17.5 MB for all 576 of them — and the answer does not
    /// depend on how densely the figure will be drawn, so it is worth paying
    /// for up front and keeping for the run. Refining those triangles to a
    /// density is the opposite: a couple of milliseconds, and megabytes that
    /// quadruple with every level. That is built on demand and reaped when
    /// nothing is drawing it.
    ///
    /// So this builds no meshes. It costs **17.5 MB and 1.5 s**, and what it
    /// buys is that no figure a show reaches has to be tessellated while the
    /// show is running: a look recall clobbers every mixer channel at once,
    /// and the eight most expensive figures tessellated on one frame is a
    /// third of a second of frozen frames.
    ///
    /// **Most of that 17.5 MB is figures nobody touches**, and it is worth
    /// knowing that before deciding it is worth paying: an interior averages
    /// 30 kB, so a show that draws twenty figures would accumulate about
    /// 600 kB of them on demand. What the eager pass buys is not the memory, which is
    /// cheaper the other way, but the absence of a stall on the one frame
    /// where every channel changes at once.
    ///
    /// Called once at startup, where seconds are free — the bootstrapper
    /// pushes a client and waits for it.
    pub fn tessellate_library(&mut self) {
        let Self { figures, fills, .. } = self;
        let baked = (0..tunnels_sprites::count())
            .filter_map(|id| u16::try_from(id).ok())
            .map(|id| FigureId::Baked(SpriteId(id)));
        let generated = GeneratedId::library().map(FigureId::Generated);
        let mut tessellated = 0;
        for figure in baked.chain(generated) {
            let Some(contours) = figures.get(figure) else {
                continue;
            };
            fills.get(figure, contours);
            tessellated += 1;
        }
        info!("Tessellated {tessellated} figure interiors.");
    }

    /// Total triangles held in refined meshes, for reporting memory pressure.
    pub fn mesh_triangles(&self) -> usize {
        self.meshes.triangles()
    }

    fn draw_fill<G: Graphics<Texture = T>>(
        &mut self,
        fill: &FillLayer,
        c: &Context,
        gl: &mut G,
        cfg: &ClientConfig,
    ) {
        let Self {
            figures,
            fills,
            outlines,
            meshes,
            ramps,
            frame,
            verts,
        } = self;

        // A figure this build does not carry draws nothing.
        let Some(contours) = figures.get(fill.figure) else {
            return;
        };

        let placed = Placed::of(fill, c, cfg);
        if placed.px_per_unit <= 0.0 {
            // Scaled to nothing. Nothing to draw, and no density to draw it at.
            return;
        }
        let level = Level::for_screen(
            placed.px_per_unit,
            if cfg.refine_large_figures {
                Level::FINEST
            } else {
                Level::DEFAULT_FINEST
            },
        );

        // A figure with one colour everywhere needs no ramp and no
        // interpolation — but a colour animation moves that one colour every
        // frame, so it is only uniform when there is no animation on it either.
        // A mask is the exception: no animation can move opaque black, so it
        // takes this path however many are running on it.
        let flat = fill.color.is_mask() || (fill.color_anims.is_empty() && fill.color.is_uniform());
        let color = flat_color(fill);
        let warping = fill.warps_points();
        // A colour animation that varies across the figure has to be resolved
        // against the figure's own coordinate, not the colour cycle's, or its
        // period comes out as the colour's rather than its own.
        let span = RampSpan::of(&fill.color_anims);

        // What the thickness knob comes to as a fraction of the width every
        // outline is stroked at, which is what the per-vertex pass narrows by.
        let stroke_width = (thickness_px(fill.thickness, cfg) / placed.px_per_unit) as f32
            / geometry::REFERENCE_WIDTH;
        let interior = fill
            .draw_mode
            .draws_fill()
            .then(|| fills.get(fill.figure, contours));
        let outline = fill
            .draw_mode
            .draws_outline()
            .then(|| outlines.get(fill.figure, contours));
        // A uniform figure needs no ramp; a varying one needs the texture
        // holding its colour, which the pool may already have.
        let texture = if flat {
            None
        } else {
            match ramps.texture_for(fill, span, *frame) {
                Some(texture) => Some(texture),
                None => return,
            }
        };
        let field = PhaseField {
            phase: fill.color.phase,
            cycles: fill.color.cycles as f32,
            span,
        };
        let work = VertexWork {
            field,
            spin_speed: fill.spin_speed as f32,
            warps: &fill.warps,
            taper: &fill.taper,
            stroke_width,
        };

        // Nothing varies across the figure and nothing displaces it, so its
        // interior draws straight from the tessellator's own triangles. An
        // outline never takes that path: it is stroked at one width for every
        // beam and reaches its own by being narrowed per vertex, so it goes
        // through the pass however still the figure is.
        if flat
            && !warping
            && let Some(interior) = interior
        {
            draw_flat(interior.points(), interior.indices(), color, placed.m, gl);
        } else if let Some(interior) = interior {
            let mesh = meshes.get(
                MeshId {
                    figure: fill.figure,
                    level,
                },
                interior,
                *frame,
            );
            // Phase comes from the undeformed position, so a colour pattern
            // stays glued to the figure while a warp moves it rather than
            // sliding across it. One walk of the mesh produces displaced
            // positions and ramp coordinates together, because both want the
            // same polar coordinates for a point.
            verts.vertex_pass(mesh, work);
            match texture {
                Some(texture) => {
                    draw_textured(
                        mesh.indices(),
                        verts,
                        field.wrap_period(),
                        texture,
                        placed.m,
                        gl,
                    );
                }
                None => draw_flat(verts.positions(), mesh.indices(), color, placed.m, gl),
            }
        }

        if let Some(outline) = outline {
            // An outline is never meshed. Its colour comes from the contour, so
            // nothing varies across a ribbon that a finer mesh could resolve,
            // and the tessellator's own triangles are drawn as they come.
            if outline.is_empty() {
                return;
            }
            verts.stroke_vertex_pass(outline, work);
            match texture {
                Some(texture) => {
                    draw_textured(
                        outline.indices(),
                        verts,
                        field.wrap_period(),
                        texture,
                        placed.m,
                        gl,
                    );
                }
                None => draw_flat(verts.positions(), outline.indices(), color, placed.m, gl),
            }
        }
    }
}

/// The single colour a figure draws in when nothing varies across it.
fn flat_color(fill: &FillLayer) -> Color {
    hsv_to_rgb(&fill.color.sample(Phase::ZERO, ColorAdjust::default()))
}

/// Where a figure lands on screen.
struct Placed {
    /// Maps the figure's unit box onto the viewport.
    m: Matrix2d,
    /// Pixels one figure-space unit covers, which is what mesh density and
    /// stroke bucketing are both measured against.
    px_per_unit: f64,
}

impl Placed {
    /// Placed the way a segment is, so a figure and a beam at the same knob
    /// settings cover the same ground.
    fn of(fill: &FillLayer, c: &Context, cfg: &ClientConfig) -> Self {
        let p = &fill.placement;
        let placed = place(p, c, cfg);
        let (half_x, half_y) = (
            p.extent_x * cfg.critical_size,
            p.extent_y * cfg.critical_size,
        );
        Self {
            m: placed.scale(half_x, half_y),
            px_per_unit: half_x.abs().max(half_y.abs()),
        }
    }
}

/// An animation shaped so that its value reads where on the figure it is
/// asked, aimed at `target`.
///
/// Noise is the only waveform that reads a second coordinate of the figure as
/// well as the one its phase runs along, and unsmoothed it spends the whole of
/// the figure's spread — which is the setting where a coordinate and a vertex
/// index differ most.
#[cfg(test)]
pub(crate) fn noise_animation(
    target: tunnels_model::animation_target::AnimationTarget,
) -> tunnels_model::animation::TargetedAnimation<tunnels_model::animation::PreparedAnimation> {
    use std::time::Duration;
    use tunnels_lib::number::UnipolarFloat;
    use tunnels_model::animation::{
        Animation, ControlMessage, EmitStateChange, OffsetSpan, StateChange, TargetedAnimation,
        Waveform,
    };
    use tunnels_model::clock_bank::ClockBank;

    struct Noop;
    impl EmitStateChange for Noop {
        fn emit_animation_state_change(&mut self, _: StateChange) {}
    }

    let mut animation = Animation::default();
    for sc in [
        StateChange::Waveform(Waveform::Noise),
        StateChange::NPeriods(1),
        StateChange::Size(UnipolarFloat::ONE),
        StateChange::Smoothing(UnipolarFloat::ZERO),
    ] {
        animation.control(ControlMessage::Set(sc), &mut Noop);
    }
    // Smoothing is reached over time rather than set. The animation runs at no
    // speed, so nothing else moves while it gets there.
    animation.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
    TargetedAnimation {
        animation: animation.prepare(
            &ClockBank::default(),
            UnipolarFloat::ZERO,
            OffsetSpan::Figure,
        ),
        target,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use graphics::ImageSize;
    use texture::TextureOp;
    use tunnels_model::layer::{ColorField, DrawMode, PhaseAxis, Placement, SpriteId};

    /// The warm-up tessellates every figure either library names, and refines
    /// none of them.
    ///
    /// Both halves matter. A figure left untessellated is one the operator
    /// reaches and waits tens of milliseconds for, in the middle of a show. A
    /// mesh built here would be speculative: a density guessed at before any
    /// screen has been measured, and the guess costs four times as much for
    /// each level it is wrong by.
    #[test]
    fn the_warm_up_tessellates_every_figure_and_refines_none() {
        let mut renderer = Renderer::<FakeTexture>::default();
        renderer.tessellate_library();
        let figures = tunnels_sprites::count() + GeneratedId::library().count();
        assert_eq!(
            renderer.fills.held(),
            figures,
            "the warm-up does not tessellate both libraries"
        );
        // What that comes to is the number the docstring quotes, and the whole
        // of what a client holds before it has drawn anything.
        let mb = renderer.fills.bytes() as f64 / 1e6;
        assert!(
            (15.0..21.0).contains(&mb),
            "the interiors weigh {mb:.1} MB, not the 17.5 the docstring quotes"
        );
        assert_eq!(
            renderer.meshes.len(),
            0,
            "the warm-up refines meshes at a density nothing has asked for"
        );
        // Outlines are not warmed either. One is 47,554 vertices on the
        // average figure, so the whole library would be 110 MB against the
        // 17 MB of interiors beside it — and a show draws a handful of figures
        // rather than five hundred.
        assert_eq!(
            renderer.outlines.held(),
            0,
            "the warm-up strokes outlines it has no reason to build"
        );
    }

    /// Stands in for a GPU texture, identifiable so a test can tell which one
    /// the pool handed back.
    struct FakeTexture {
        id: usize,
    }

    impl ImageSize for FakeTexture {
        fn get_size(&self) -> (u32, u32) {
            (RampSpan::Cycle.texels(), 1)
        }
    }

    impl TextureOp<()> for FakeTexture {
        type Error = ();
    }

    impl CreateTexture<()> for FakeTexture {
        fn create<S: Into<[u32; 2]>>(
            _: &mut (),
            _: Format,
            _: &[u8],
            _: S,
            _: &TextureSettings,
        ) -> Result<Self, Self::Error> {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            Ok(Self {
                id: NEXT.fetch_add(1, Ordering::Relaxed),
            })
        }
    }

    impl UpdateTexture<()> for FakeTexture {
        fn update<O, S>(&mut self, _: &mut (), _: Format, _: &[u8], _: O, _: S) -> Result<(), ()>
        where
            O: Into<[u32; 2]>,
            S: Into<[u32; 2]>,
        {
            Ok(())
        }
    }

    /// A figure whose colour is decided by `center` and nothing else.
    fn fill(center: f64) -> FillLayer {
        FillLayer {
            figure: FigureId::Baked(SpriteId(0)),
            placement: Placement {
                x: 0.,
                y: 0.,
                extent_x: 0.5,
                extent_y: 0.5,
                rot_angle: 0.,
            },
            spin_speed: 0.,
            thickness: 0.,
            draw_mode: DrawMode::Fill,
            color: ColorField {
                phase: PhaseAxis::Angle,
                cycles: 3.,
                center,
                width: 1.,
                sat: 1.,
                val: 1.,
                level: 1.,
            },
            color_anims: Vec::new(),
            warps: Vec::new(),
            taper: Vec::new(),
        }
    }

    #[test]
    fn a_texture_is_not_written_again_while_a_draw_may_be_reading_it() {
        use std::collections::HashMap;

        let mut pool: RampPool<FakeTexture> = RampPool::new();
        let mut last_written: HashMap<usize, u64> = HashMap::new();
        // One layer, one colour, many frames. The ramp is rebuilt every frame
        // — that is the point of dropping the content cache — so what matters
        // is only which texture it is written into.
        for n in 1..40 {
            let id = pool
                .texture_for(&fill(0.25), RampSpan::Cycle, Frame(n))
                .expect("a texture")
                .id;
            if let Some(previous) = last_written.insert(id, n) {
                assert!(
                    n - previous >= FRAMES_IN_FLIGHT,
                    "texture {id} was written at frame {previous} and again at {n}, \
                     inside the {FRAMES_IN_FLIGHT} frames a draw may still be reading it"
                );
            }
        }
        // And it does come back round: a single layer settles on a fixed set
        // rather than asking for a new texture every frame.
        assert_eq!(last_written.len(), FRAMES_IN_FLIGHT as usize);
    }

    #[test]
    fn figures_drawn_together_never_share_a_texture() {
        let mut pool: RampPool<FakeTexture> = RampPool::new();
        // Four figures in one frame. None may be handed a texture another has
        // already been drawn with, whatever their colours are.
        let frame = Frame(1);
        let ids: Vec<usize> = (0..4)
            .map(|i| {
                pool.texture_for(&fill(f64::from(i) * 0.2), RampSpan::Cycle, frame)
                    .expect("a texture")
                    .id
            })
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            ids.len(),
            "two figures shared a texture: {ids:?}"
        );
    }

    /// Two layers at different spans draw in the same frame, one after the
    /// other, for as long as both are on screen. Each keeps its own scratch, so
    /// neither frees the other's.
    #[test]
    fn alternating_spans_do_not_reallocate_the_scratch() {
        let mut pool: RampPool<FakeTexture> = RampPool::new();
        let buffers = |p: &RampPool<FakeTexture>| {
            (
                p.cycle_scratch.as_raw().as_ptr(),
                p.figure_scratch.as_raw().as_ptr(),
            )
        };
        let before = buffers(&pool);
        for frame in 1..20 {
            let frame = Frame(frame);
            for span in [RampSpan::Cycle, RampSpan::Figure] {
                assert!(pool.texture_for(&fill(0.25), span, frame).is_some());
            }
        }
        assert_eq!(
            buffers(&pool),
            before,
            "a scratch image was reallocated while both spans were drawing"
        );
        assert_eq!(pool.cycle_scratch.width(), RampSpan::Cycle.texels());
        assert_eq!(pool.figure_scratch.width(), RampSpan::Figure.texels());
    }

    /// The pool bounds itself, so there is no cap and no eviction policy: it
    /// grows to what one frame draws times the frames in flight, and stops.
    #[test]
    fn the_pool_settles_at_frames_in_flight_per_layer() {
        for layers in [1u32, 3] {
            let mut pool: RampPool<FakeTexture> = RampPool::new();
            for n in 1..500u64 {
                for layer in 0..layers {
                    let center = (n as f64 * 0.01 + f64::from(layer) * 0.1).rem_euclid(1.0);
                    assert!(
                        pool.texture_for(&fill(center), RampSpan::Cycle, Frame(n))
                            .is_some()
                    );
                }
            }
            let bound = FRAMES_IN_FLIGHT as usize * layers as usize;
            assert_eq!(
                pool.entries.len(),
                bound,
                "{layers} layer(s) settled at {} textures, not {bound}",
                pool.entries.len()
            );
        }
    }
}
