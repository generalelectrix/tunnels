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

use self::draw::{PhaseField, VertexBuffers, VertexWork, draw_flat, draw_textured, draw_tris};
use self::geom::TriangleList;
use self::geometry::{GeometryCache, Scale, Width};
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
use tunnels_model::layer::{ColorAdjust, FillLayer, Layer, LayerCollection, LayerKey, SpriteId};

/// How many textures a layer rotates through when its ramp changes.
///
/// Overwriting a texture the GPU is still sampling makes the driver stall until
/// the queued draws that read it retire. Measured on the prototype, that stall
/// was two orders of magnitude larger than building the ramp in the first
/// place. Rotating means a texture is only rewritten once the frames that used
/// it are long done.
const RAMP_BUFFERS: usize = 3;

/// A layer's ramp textures and per-frame scratch space.
struct LayerRuntime<T> {
    key: Option<RampKey>,
    textures: Vec<T>,
    current: usize,
    scratch: RgbaImage,
    verts: VertexBuffers,
}

impl<T> LayerRuntime<T>
where
    T: CreateTexture<()> + UpdateTexture<()>,
{
    fn new(settings: &TextureSettings) -> Option<Self> {
        let scratch = ramp::blank();
        let (width, height) = scratch.dimensions();
        let mut textures = Vec::with_capacity(RAMP_BUFFERS);
        for _ in 0..RAMP_BUFFERS {
            match T::create(
                &mut (),
                Format::Rgba8,
                scratch.as_raw(),
                [width, height],
                settings,
            ) {
                Ok(texture) => textures.push(texture),
                Err(e) => {
                    error!("Could not make a colour ramp texture: {e:?}");
                    return None;
                }
            }
        }
        Some(Self {
            key: None,
            textures,
            current: 0,
            scratch,
            verts: VertexBuffers::default(),
        })
    }

    /// Rebuild the ramp if it moved, and leave the texture to sample current.
    ///
    /// A live colour animation moves it every frame, which is affordable
    /// precisely because a rebuild is a thousand evaluations and a write to a
    /// texture nothing is still reading.
    fn refresh(&mut self, fill: &FillLayer) {
        let key = RampKey::of(&fill.color);
        let animated = !fill.color_anims.is_empty();
        if !animated && self.key == Some(key) {
            return;
        }
        ramp::build_into(&mut self.scratch, &fill.color, &fill.color_anims);
        self.current = (self.current + 1) % self.textures.len();
        let (width, height) = self.scratch.dimensions();
        if let Some(texture) = self.textures.get_mut(self.current)
            && let Err(e) = texture.update(
                &mut (),
                Format::Rgba8,
                self.scratch.as_raw(),
                [0, 0],
                [width, height],
            )
        {
            error!("Could not update a colour ramp texture: {e:?}");
        }
        self.key = Some(key);
    }
}

/// Everything a frame needs to draw layers, and the caches behind it.
///
/// Generic over the texture type so the GL client and the golden-image tests
/// share one draw path rather than each having its own.
pub struct Renderer<T> {
    geometry: GeometryCache,
    meshes: MeshLibrary,
    /// Keyed by the mixer path rather than by position in the output, because
    /// a channel at level zero is not expanded and every later position shifts
    /// when it comes up.
    ///
    /// Never evicts, on the same premise the mesh library rests on: a show
    /// touches a handful of channels and the set converges within seconds.
    layers: HashMap<LayerKey, LayerRuntime<T>>,
    /// Figures asked for that this build does not carry, so each is reported
    /// once rather than every frame.
    missing: HashSet<SpriteId>,
    ramp_settings: TextureSettings,
}

impl<T> Default for Renderer<T> {
    fn default() -> Self {
        Self {
            geometry: GeometryCache::default(),
            meshes: MeshLibrary::default(),
            layers: HashMap::new(),
            missing: HashSet::new(),
            // Repeating wrap is what lets a phase coordinate run past one cycle
            // and keep indexing the ramp, so the cycle count never reaches the
            // mesh. Linear filtering puts the sawtooth's jump inside one texel.
            ramp_settings: TextureSettings::new()
                .filter(Filter::Linear)
                .wrap_u(Wrap::Repeat)
                .wrap_v(Wrap::Repeat),
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
            layers,
            missing,
            ramp_settings,
        } = self;

        let Some(sprite) = tunnels_shapes::sprite(fill.sprite.0) else {
            if missing.insert(fill.sprite) {
                error!(
                    "This build carries no figure {}; it has {}.",
                    fill.sprite.0,
                    tunnels_shapes::count()
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
            Width::bucketed(
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
                draw_tris(geometry.fill(fill.sprite, sprite), color, placed.m, gl);
            }
            if let Some(width) = stroke {
                draw_tris(
                    geometry.stroke(fill.sprite, sprite, width),
                    color,
                    placed.m,
                    gl,
                );
            }
            return;
        }

        let runtime = match layers.entry(fill.key) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let Some(runtime) = LayerRuntime::new(ramp_settings) else {
                    return;
                };
                e.insert(runtime)
            }
        };
        if !flat {
            runtime.refresh(fill);
        }
        // Split the runtime into disjoint pieces: the texture is read while the
        // vertex buffers are written.
        let LayerRuntime {
            textures,
            current,
            verts,
            ..
        } = runtime;
        let Some(texture) = textures.get(*current) else {
            return;
        };

        let field = PhaseField {
            phase: fill.color.phase,
            cycles: fill.color.cycles as f32,
        };
        let mut piece = |source: &TriangleList, stroke: Option<u32>, gl: &mut G| {
            let mesh = meshes.get(
                MeshId {
                    sprite: fill.sprite,
                    stroke,
                    level,
                },
                source,
            );
            // Phase comes from the undeformed position, so a colour pattern
            // stays glued to the figure while a warp moves it rather than
            // sliding across it. One walk of the mesh produces displaced
            // positions and ramp coordinates together, because both want the
            // same polar coordinates for a point.
            draw::vertex_pass(
                verts,
                mesh,
                VertexWork {
                    field,
                    base_spin: fill.spin as f32,
                    warps: &fill.warps,
                },
            );
            if flat {
                draw_flat(mesh, verts, flat_color(fill), placed.m, gl);
            } else {
                draw_textured(mesh, verts, field.wrap_period(), texture, placed.m, gl);
            }
        };

        if fill.draw_mode.draws_fill() {
            piece(geometry.fill(fill.sprite, sprite), None, gl);
        }
        if let Some(width) = stroke {
            piece(
                geometry.stroke(fill.sprite, sprite, width),
                Some(width.key()),
                gl,
            );
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
