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

## Meshing and gradient cost

Per-vertex color interpolates linearly, so a gradient is only faithful where
triangles are small relative to how fast the color moves. `mesh.rs` refines a
shape by longest-edge bisection until every edge is under a target length —
splitting only the longest edge so a long thin triangle refines along its
length instead of shattering in both directions.

**Mesh density depends on on-screen size and nothing else.** It is deliberately
independent of the color knobs, which is what lets a mesh outlive a color
change — including a waveform driven by a clock, which changes every frame.

Densities are bucketed as powers of two (`Level`), from a quarter-unit edge
down to 1/128, which is where a shape filling a 1080-line projector puts
triangles under a pixel. `MeshLibrary` builds a level the first time a shape is
drawn at that size and keeps it, so a scale slider steps between a handful of
prebuilt meshes. The render window logs the library as it grows: a hitch on
first use is expected, a hitch later is not.

Building every level of every shape up front is not viable, which is why it is
lazy — measure with `-- stats`:

| edge | build all 98 | median tris | max tris | held |
|---|---|---|---|---|
| 0.25 | 3ms | 252 | 2,336 | 0.6MB |
| 0.0625 | 31ms | 2,844 | 22,316 | 5.9MB |
| 0.0156 | 372ms | 37,216 | 207,536 | 72MB |
| 0.0078 | 1,469ms | 147,136 | 747,128 | 265MB |

Per frame at the density a full-screen shape on a 1080-line projector asks for
(edge 0.0156), color evaluation runs once per *unique* vertex — vertices are
shared, so a flat triangle list's sixfold repetition is avoided:

- median 402us, worst 2,191us (100k verts)
- three worst-case layers: 6.6ms of a 16.7ms frame at 60Hz

Two things worth knowing if this goes into production:

- The cost is **not** the sawtooth's discontinuity. Measured against a
  continuous triangle wave it came out within 10% — the expense is resolving
  several hue cycles across a shape at all, which is area-bound.
- A shader computing hue per fragment would sidestep the whole problem;
  `opengl_graphics` has a fixed pipeline, but it is only GL underneath.

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
