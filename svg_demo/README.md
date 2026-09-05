# svg_demo

A throwaway demo for judging whether a library of SVG shapes is worth building
into tunnels for texture projection onto trees.

## Running

```
cargo run --release -p svg_demo -- control
```

That opens the egui knob window and spawns the render window as a child.
The render window is the **same piston/OpenGL/SDL2 stack `tunnelclient` uses**
— `OpenGL::V3_2`, 4x MSAA, vsync — so what you see is what the real client
would produce. The two halves talk over localhost UDP; each can also be run on
its own (`-- render`, `-- control`).

## Headless output

```
cargo run --release -p svg_demo -- sheet     /tmp/shapes.png
cargo run --release -p svg_demo -- features  /tmp/features.png 72 83 75
```

`sheet` is a contact sheet of the whole library. `features` takes shape indices
(printed by `sheet`) and shows each across the four color phases, in outline
mode, and stacked with two masks.

## Color model

Ported from `Tunnel::render`, so a look dialled in here maps onto the same
knobs on the real control surface:

```
hue = col_center + 0.5 * col_width * sawtooth(phase * floor(16 * col_spread))
```

A tunnel's `phase` is `rel_angle` — a segment's position around the ring. A
closed figure has no segments, so **color along** picks what stands in for it.
`angle` is the literal analogue and behaves like a tunnel under rotation.

Brightness is `level` (the alpha channel), matching `Channel.level`; value is
fixed at 1.0 as it is in the real system.

## Gradient cost

Per-vertex color interpolates linearly, so a gradient is only faithful where
triangles are small relative to how fast the color moves. `draw.rs` subdivides
by longest-edge bisection while the color difference across a triangle exceeds
a threshold, with a floor on triangle size that bounds the total by area.

Subdivision happens in shape space and depends only on the shape and the color
knobs, so it is cached: position, rotation, scale and shear are a transform
applied to the finished mesh. Measure with `-- stats`:

| | median | worst |
|---|---|---|
| re-subdivide (on a color knob move) | 3.2ms | 28.6ms |
| per frame, cached (transform only) | 158us | 1.2ms |

Three worst-case layers cost 3.5ms of a 16.7ms frame at 60Hz. Moving a color
knob on the heaviest shape drops a frame or two.

Two things worth knowing if this goes into production:

- The cost is **not** the sawtooth's discontinuity. Measured against a
  continuous triangle wave it came out within 10% — the expense is resolving
  several hue cycles across a shape at all, which is area-bound.
- The size floor is in shape space, so how it looks depends on how large the
  shape is on screen. A shader computing hue per fragment would sidestep the
  whole problem; `opengl_graphics` has a fixed pipeline, but it is only GL
  underneath.

## Masks

The `mask` toggle paints opaque black instead of color, which is exactly what
`Channel.mask` does in the real mixer. Stacking a mask over a lit shape
intersects their apertures — gobo stacking, already supported by the existing
compositing model.

## Shapes

`shapes/` holds ~98 SVGs, loaded at runtime. See `CREDITS.json` for sources and
licenses. Three groups:

- **Procedural** (`scripts/gen_shapes.py`) — star polygons, concentric rings,
  radial spokes, sunbursts, petal mandalas, slats, grids, nested frames,
  chevrons, gears.
- **Dingbats** (`scripts/gen_dingbats.py`) — glyphs from Noto Sans Symbols 2
  (OFL 1.1), filtered to solid geometric figures: pinwheel stars, spoked
  asterisks, snowflakes, greek crosses, saltires, checkerboards, trigrams.
- **Wikimedia Commons** — only three survived. Most real-world SVG line art
  is stroke-based or carries a background rect, so it fills as a plain square
  or disc. Artwork has to be a filled silhouette to be usable.

Regenerate with `python3 scripts/gen_shapes.py` and
`python3 scripts/gen_dingbats.py` (the latter needs `fonttools` and the font in
`/tmp/fonts`).
