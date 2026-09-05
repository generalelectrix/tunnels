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

## Profiling

`stats` measures the CPU stages with no window. To measure the real thing —
actual GL, actual GPU, on your hardware — use `profile`:

```
cargo run --release -p svg_demo -- profile
cargo run --release -p svg_demo -- profile --layers 4 --size 1920x1080 --seconds 10
cargo run --release -p svg_demo -- profile --shapes 72,83,75
```

It opens a window with **vsync off** (with it on you measure the display, not
the work), draws the heaviest shapes in the library near full screen with
gradients on and spinning, discards 120 warmup frames, and reports frame time
percentiles with the CPU broken down by stage.

It runs the scenario twice — once sweeping hue every frame, once holding it
still. The difference is what animated color actually costs, which is the claim
most worth checking: hue is meant to be a ramp rebuild and nothing more.

The profiler drives the same `Renderer` the live window does, not a copy, so a
number it reports is a number the show would see. Frame time minus our own CPU
time is attributed to the driver, the GPU, and the buffer swap.

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

## Meshing and color

Color never touches the mesh. The split is:

- **the mesh carries phase** — where each vertex sits in the color cycle, a
  function of position alone
- **a 1024-texel ramp texture carries color** — one cycle of the hue waveform,
  sampled with repeating wrap

The sampler resolves the waveform per fragment, so the sawtooth's discontinuity
lands exactly where it belongs however coarse the mesh is. Per-vertex color
cannot do that: interpolation across a triangle renders the jump as a ramp, and
because the mesh is not aligned to the boundary that ramp wanders, which reads
as raggedness. Refining the mesh to hide it makes every color knob invalidate
the geometry — and an animated one invalidate it every frame.

Consequences worth knowing:

- The **cycle count is free**. A phase coordinate past one simply repeats
  against the texture, so `col_spread` never reaches the mesh.
- A **color change is a thousand-texel rewrite** and nothing else. Animating hue
  costs the same as holding it.
- Angular phase jumps a whole cycle where `atan2` wraps. Both ends still sample
  correctly — the ramp repeats and the cycle count is an integer — so
  `same_branch` shifts a triangle's coordinates by whole periods to take the
  short path between them.

`mesh.rs` refines by longest-edge bisection to a target edge chosen from
on-screen size, tightening near the origin where angular phase varies fastest.
Both criteria are pure geometry. Densities are bucketed as powers of two
(`Level`); `MeshLibrary` builds one the first time a shape is drawn at that size
and keeps it, so a scale slider steps between prebuilt meshes.

Measured on a 1080p window, three heavy shapes near full screen, 151k
triangles, vsync off (`-- profile --shapes 72,83,75`):

| stage | p50 | note |
|---|---|---|
| submit | 1.44ms | projecting 454k vertices and filling piston's buffers |
| phase eval | 0.21ms | once per unique vertex |
| ramp build | 0.05ms | only when a color knob moves |
| mesh build | 0.00ms | cached; nonzero here means bucket thrashing |
| gpu + swap | 0.34ms | fill rate is not the constraint |
| **frame p99** | **5.27ms** | 32% of a 60Hz budget |

Holding hue still instead of sweeping it every frame changes CPU by 0.05ms —
the ramp rebuild and nothing else, which is the whole point of the split.

`submit` dominates, and it is there because `tri_list_uv` takes *pre-transformed*
vertices: piston's fixed pipeline makes the CPU do what a vertex shader would do
for free. It scales linearly with triangle count, so it is what would need to
change to go much past a handful of full-screen layers.

Mesh building, measured with `-- stats`:

| edge | build all 98 | median tris | max tris | held |
|---|---|---|---|---|
| 0.25 | 7ms | 419 | 2,790 | 0.9MB |
| 0.0625 | 43ms | 4,241 | 23,794 | 8MB |
| 0.0156 | 520ms | 57,253 | 304,865 | 99MB |
| 0.0078 | 1,892ms | 219,544 | 1,167,707 | 369MB |

Building every level of every shape up front is not viable, hence lazy. Per
frame at the density a full-screen shape on a 1080-line projector asks for,
phase evaluation runs once per *unique* vertex: median 254us, worst 1.4ms.
Three worst-case layers is 4.3ms of a 16.7ms frame at 60Hz.

Where this still bites: **2D noise does not factor through a scalar the way an
analytic waveform does**, so its spatial frequency is bounded by mesh density
rather than by texture resolution. A fragment shader would remove that limit
along with the rest of this; `opengl_graphics` has a fixed pipeline, but it is
only GL underneath.

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
