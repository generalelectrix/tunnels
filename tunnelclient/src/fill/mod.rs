//! Drawing a filled figure.
//!
//! The split this module is built around: **the mesh carries phase and a
//! texture carries colour.** Where a point sits in the colour cycle is a
//! function of its position alone, so a mesh built for a figure at a size holds
//! for every colour that figure can take — including one changing every frame.
//! Colour is then resolved per fragment against a thousand-texel ramp, which is
//! what lets the sawtooth's jump land exactly where it belongs however coarse
//! the mesh is.

mod draw;
mod fastmath;
mod geom;
mod geometry;
mod mesh;
mod ramp;

use self::draw::{
    PhaseField, VertexBuffers, VertexWork, draw_flat, draw_list_flat, draw_list_textured,
    draw_points, draw_textured,
};
use self::geometry::{GeometryCache, Scale, Thickness};
use self::mesh::{Level, MeshId, MeshLibrary};
use self::ramp::RampKey;
use crate::draw::{Draw, hsv_to_rgb};
use client_lib::config::ClientConfig;
use client_lib::transform::{Transform, TransformDirection};
use graphics::math::Matrix2d;
use graphics::types::Color;
use graphics::{Context, Graphics, Transformed};
use image::RgbaImage;
use log::error;
use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;
use texture::{CreateTexture, Filter, Format, TextureSettings, UpdateTexture, Wrap};
use tunnels_lib::number::Phase;
use tunnels_model::layer::{ColorAdjust, FillLayer, Layer, LayerCollection, SpriteId};

/// How many frames a texture may still be read after the last draw that used
/// it.
///
/// Overwriting a texture the GPU is still sampling makes the driver stall
/// until the queued draws that read it retire. Measured on the prototype, that
/// stall was two orders of magnitude larger than building the ramp in the
/// first place. A texture is only reused once this many frames have gone by
/// without it being drawn.
const FRAMES_IN_FLIGHT: u64 = 3;

/// One colour ramp on the GPU, and when it was last drawn with.
struct RampEntry<T> {
    texture: T,
    last_used: u64,
}

/// The colour ramps the GPU is holding, addressed by what is in them.
///
/// A ramp is entirely determined by the colour knobs it was built from, so
/// [`RampKey`] names the texture rather than the layer that asked for it. Two
/// figures dialled to the same colour share one texture, and a still look
/// uploads nothing after its first frame.
///
/// That also removes the write-after-read hazard rather than working around
/// it: a colour that changed is a *different* key, so a texture in use is
/// never the one being written. What is left is deciding when a texture no
/// longer wanted may be reused for something else, which is what
/// [`FRAMES_IN_FLIGHT`] answers.
///
/// The pool has no size cap. Recycling bounds it on its own — a key stops
/// being reachable as soon as the colour moves on, and the entry becomes
/// available a few frames later — and a 1024-texel ramp is four kilobytes, so
/// even a pool an order of magnitude larger than a show needs would not be
/// worth the eviction policy.
struct RampPool<T> {
    entries: HashMap<RampKey, RampEntry<T>>,
    scratch: RgbaImage,
    settings: TextureSettings,
    frame: u64,
}

impl<T> RampPool<T>
where
    T: CreateTexture<()> + UpdateTexture<()>,
{
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            scratch: ramp::blank(),
            // Repeating wrap is what lets a phase coordinate run past one
            // cycle and keep indexing the ramp, so the cycle count never
            // reaches the mesh. Linear filtering puts the sawtooth's jump
            // inside one texel.
            settings: TextureSettings::new()
                .filter(Filter::Linear)
                .wrap_u(Wrap::Repeat)
                .wrap_v(Wrap::Repeat),
            frame: 0,
        }
    }

    /// The texture holding this layer's colour, building it if no one has.
    ///
    /// `None` only if the backend refused to give up a texture, which is a
    /// reason to skip a figure rather than to stop the show.
    fn texture_for(&mut self, fill: &FillLayer) -> Option<&T> {
        let key = RampKey::of(&fill.color);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.frame;
            return self.entries.get(&key).map(|e| &e.texture);
        }

        ramp::build_into(&mut self.scratch, &fill.color, &fill.color_anims);
        let (width, height) = self.scratch.dimensions();

        // Take back a texture nothing has drawn with recently, in preference
        // to asking the backend for another one.
        let stale = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_used + FRAMES_IN_FLIGHT <= self.frame)
            .min_by_key(|(_, e)| e.last_used)
            .map(|(key, _)| *key);

        let mut entry = match stale.and_then(|key| self.entries.remove(&key)) {
            Some(entry) => entry,
            None => RampEntry {
                texture: match T::create(
                    &mut (),
                    Format::Rgba8,
                    self.scratch.as_raw(),
                    [width, height],
                    &self.settings,
                ) {
                    Ok(texture) => texture,
                    Err(e) => {
                        error!("Could not make a colour ramp texture: {e:?}");
                        return None;
                    }
                },
                last_used: self.frame,
            },
        };

        if let Err(e) = entry.texture.update(
            &mut (),
            Format::Rgba8,
            self.scratch.as_raw(),
            [0, 0],
            [width, height],
        ) {
            error!("Could not update a colour ramp texture: {e:?}");
            return None;
        }
        entry.last_used = self.frame;
        Some(
            &self
                .entries
                .entry(key)
                .insert_entry(entry)
                .into_mut()
                .texture,
        )
    }
}

/// Everything a frame needs to draw layers, and the caches behind it.
///
/// Generic over the texture type so the GL client and the golden-image tests
/// share one draw path rather than each having its own.
pub struct Renderer<T> {
    geometry: GeometryCache,
    meshes: MeshLibrary,
    ramps: RampPool<T>,
    /// Scratch for the per-vertex pass, reused across every layer and every
    /// frame. Layers draw one after another and none of this outlives the
    /// draw that fills it, so one buffer serves all of them.
    verts: VertexBuffers,
    /// Figures asked for that this build does not carry, so each is reported
    /// once rather than every frame.
    missing: HashSet<SpriteId>,
}

impl<T> Default for Renderer<T>
where
    T: CreateTexture<()> + UpdateTexture<()>,
{
    fn default() -> Self {
        Self {
            geometry: GeometryCache::default(),
            meshes: MeshLibrary::default(),
            ramps: RampPool::new(),
            verts: VertexBuffers::default(),
            missing: HashSet::new(),
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
        self.ramps.frame += 1;
        for layer in layers {
            match layer.as_ref() {
                Layer::Marks(marks) => marks.draw(c, gl, cfg),
                Layer::Fill(fill) => self.draw_fill(fill, c, gl, cfg),
            }
        }
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
            geometry,
            meshes,
            ramps,
            verts,
            missing,
        } = self;

        let Some(sprite) = tunnels_sprites::sprite(fill.sprite.0) else {
            if missing.insert(fill.sprite) {
                error!(
                    "This build carries no figure {}; it has {}.",
                    fill.sprite.0,
                    tunnels_sprites::count()
                );
            }
            return;
        };

        let placed = Placed::of(fill, c, cfg);
        if placed.px_per_unit <= 0.0 {
            // Scaled to nothing. Nothing to draw, and no density to draw it at.
            return;
        }
        let level = Level::for_screen(placed.px_per_unit, cfg.target_px);

        // A figure with one colour everywhere needs no ramp and no
        // interpolation — but a colour animation moves that one colour every
        // frame, so it is only uniform when there is no animation on it either.
        let flat = fill.color_anims.is_empty() && fill.color.is_uniform();
        let warping = fill.spin != 0.0 || !fill.warps.is_empty();

        let stroke = fill.draw_mode.draws_outline().then(|| {
            Thickness::bucketed(
                // The same expression a segment's stroke weight goes through,
                // so thickness means one thing across both media.
                fill.thickness * cfg.critical_size * cfg.thickness_scale / 2.0,
                Scale {
                    px_per_unit: placed.px_per_unit,
                    nominal_px_per_unit: level.nominal_px_per_unit(cfg.target_px),
                },
            )
        });

        // Nothing varies across the figure and nothing displaces it, so it
        // draws straight from the tessellator's own triangles.
        if flat && !warping {
            let color = flat_color(fill);
            if fill.draw_mode.draws_fill() {
                draw_points(
                    geometry.fill(fill.sprite, sprite).points(),
                    color,
                    placed.m,
                    gl,
                );
            }
            if let Some(thickness) = stroke {
                draw_points(
                    geometry.stroke(fill.sprite, sprite, thickness).points(),
                    color,
                    placed.m,
                    gl,
                );
            }
            return;
        }

        // A uniform figure needs no ramp; a varying one needs the texture
        // holding its colour, which the pool may already have.
        let texture = if flat {
            None
        } else {
            match ramps.texture_for(fill) {
                Some(texture) => Some(texture),
                None => return,
            }
        };

        let field = PhaseField {
            phase: fill.color.phase,
            cycles: fill.color.cycles as f32,
        };
        let work = |field| VertexWork {
            field,
            base_spin: fill.spin as f32,
            warps: &fill.warps,
        };

        if fill.draw_mode.draws_fill() {
            let mesh = meshes.get(
                MeshId {
                    sprite: fill.sprite,
                    level,
                },
                geometry.fill(fill.sprite, sprite),
            );
            // Phase comes from the undeformed position, so a colour pattern
            // stays glued to the figure while a warp moves it rather than
            // sliding across it. One walk of the mesh produces displaced
            // positions and ramp coordinates together, because both want the
            // same polar coordinates for a point.
            draw::vertex_pass(verts, mesh, work(field));
            match texture {
                Some(texture) => {
                    draw_textured(mesh, verts, field.wrap_period(), texture, placed.m, gl);
                }
                None => draw_flat(mesh, verts, flat_color(fill), placed.m, gl),
            }
        }

        if let Some(thickness) = stroke {
            // An outline is never meshed. Its colour comes from the contour, so
            // nothing varies across a ribbon that a finer mesh could resolve,
            // and the tessellator's own triangles are drawn as they come.
            let outline = geometry.stroke(fill.sprite, sprite, thickness);
            if outline.is_empty() {
                return;
            }
            draw::stroke_vertex_pass(verts, outline, work(field));
            match texture {
                Some(texture) => {
                    draw_list_textured(verts, field.wrap_period(), texture, placed.m, gl);
                }
                None => draw_list_flat(verts, flat_color(fill), placed.m, gl),
            }
        }
    }
}

/// The single colour a figure draws in when nothing varies across it.
fn flat_color(fill: &FillLayer) -> Color {
    let c = fill.color.sample(Phase::ZERO, ColorAdjust::default());
    hsv_to_rgb(c.hue, c.sat, c.val, c.level)
}

/// Where a figure lands on screen.
struct Placed {
    /// Maps the figure's unit box onto the viewport.
    m: Matrix2d,
    /// Pixels one shape-space unit covers, which is what mesh density and
    /// stroke bucketing are both measured against.
    px_per_unit: f64,
}

impl Placed {
    /// Placed the way a segment is, so a figure and a beam at the same knob
    /// settings cover the same ground.
    fn of(fill: &FillLayer, c: &Context, cfg: &ClientConfig) -> Self {
        let p = &fill.placement;
        let (x0, y0) = match cfg.transformation {
            None => (p.x, p.y),
            Some(Transform::Flip(TransformDirection::Horizontal)) => (-p.x, p.y),
            Some(Transform::Flip(TransformDirection::Vertical)) => (p.x, -p.y),
        };
        let x = x0 * f64::from(cfg.x_resolution) + cfg.x_center;
        let y = y0 * f64::from(cfg.y_resolution) + cfg.y_center;

        let placed = match cfg.transformation {
            None => c.transform.trans(x, y),
            Some(Transform::Flip(TransformDirection::Horizontal)) => {
                c.transform.trans(x, y).flip_h()
            }
            Some(Transform::Flip(TransformDirection::Vertical)) => c.transform.trans(x, y).flip_v(),
        }
        .rot_rad(p.rot_angle * TAU);

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

#[cfg(test)]
mod test {
    use super::*;
    use graphics::ImageSize;
    use texture::TextureOp;
    use tunnels_model::layer::{ColorField, ColorPhase, DrawMode, Placement, SpriteId};

    /// Stands in for a GPU texture, counting what the pool asks of it.
    struct FakeTexture {
        updates: usize,
    }

    impl ImageSize for FakeTexture {
        fn get_size(&self) -> (u32, u32) {
            (ramp::RAMP_TEXELS, 1)
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
            Ok(Self { updates: 0 })
        }
    }

    impl UpdateTexture<()> for FakeTexture {
        fn update<O, S>(&mut self, _: &mut (), _: Format, _: &[u8], _: O, _: S) -> Result<(), ()>
        where
            O: Into<[u32; 2]>,
            S: Into<[u32; 2]>,
        {
            self.updates += 1;
            Ok(())
        }
    }

    /// A figure whose colour is decided by `center` and nothing else.
    fn fill(center: f64) -> FillLayer {
        FillLayer {
            sprite: SpriteId(0),
            placement: Placement {
                x: 0.,
                y: 0.,
                extent_x: 0.5,
                extent_y: 0.5,
                rot_angle: 0.,
            },
            spin: 0.,
            thickness: 0.,
            draw_mode: DrawMode::Fill,
            color: ColorField {
                phase: ColorPhase::Angle,
                cycles: 3.,
                center,
                width: 1.,
                sat: 1.,
                val: 1.,
                level: 1.,
            },
            color_anims: Vec::new(),
            warps: Vec::new(),
        }
    }

    #[test]
    fn one_colour_is_one_texture_however_many_figures_want_it() {
        let mut pool: RampPool<FakeTexture> = RampPool::new();
        pool.frame = 1;
        for _ in 0..5 {
            assert!(pool.texture_for(&fill(0.25)).is_some());
        }
        assert_eq!(pool.entries.len(), 1, "one colour, one texture");

        // A still look uploads once and then never again, however many frames
        // go by — which is the whole of what content-addressing buys here.
        for frame in 2..10 {
            pool.frame = frame;
            assert!(pool.texture_for(&fill(0.25)).is_some());
        }
        let uploads = pool
            .entries
            .values()
            .map(|e| e.texture.updates)
            .sum::<usize>();
        assert_eq!(uploads, 1, "a still colour was uploaded {uploads} times");
    }

    #[test]
    fn figures_drawn_together_never_share_a_texture() {
        let mut pool: RampPool<FakeTexture> = RampPool::new();
        // Four different colours in one frame. None may be recycled out from
        // under a draw that has already been queued against it.
        pool.frame = 1;
        for i in 0..4 {
            assert!(pool.texture_for(&fill(f64::from(i) * 0.2)).is_some());
        }
        assert_eq!(pool.entries.len(), 4, "colours drawn together collided");
    }

    /// The pool bounds itself: an animated colour is a new key every frame, so
    /// what stops the pool growing is recycling rather than a size cap.
    #[test]
    fn an_animated_colour_settles_at_a_bounded_pool() {
        for layers in [1u32, 3] {
            let mut pool: RampPool<FakeTexture> = RampPool::new();
            let mut high_water = 0;
            for frame in 1..500u64 {
                pool.frame = frame;
                for layer in 0..layers {
                    // A colour that moves every frame, as a hue animation
                    // sweeping the ramp does.
                    let center = (frame as f64 * 0.01 + f64::from(layer) * 0.1).rem_euclid(1.0);
                    assert!(pool.texture_for(&fill(center)).is_some());
                }
                high_water = high_water.max(pool.entries.len());
            }
            // One texture per asking layer per frame still in flight, and
            // not one more: the frame being drawn reuses what fell out of
            // flight rather than adding to the pool.
            let bound = FRAMES_IN_FLIGHT as usize * layers as usize;
            assert_eq!(
                high_water, bound,
                "{layers} animated layer(s) settled at {high_water} textures, not {bound}"
            );
        }
    }
}
