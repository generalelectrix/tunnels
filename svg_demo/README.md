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

## Running it on another machine

The shape library is found via `SVG_DEMO_SHAPES`, then a `shapes` directory
beside the executable, then the crate's own — so a release binary and the
`shapes` folder copied together are enough. Nothing is baked into the build.

```
cargo build --release --target x86_64-apple-darwin -p svg_demo
scp target/x86_64-apple-darwin/release/svg_demo mini:
scp -r svg_demo/shapes mini:
ssh mini './svg_demo profile --shapes 72,83,75'
```

## Profiling

`stats` measures the CPU stages with no window. To measure the real thing —
actual GL, actual GPU, on your hardware — use `profile`:

```
cargo run --release -p svg_demo -- profile
cargo run --release -p svg_demo -- profile --layers 4 --size 1920x1080 --seconds 10
cargo run --release -p svg_demo -- profile --shapes 72,83,75
```

Useful knobs when the target hardware is slow:

| flag | why |
|---|---|
| `--target-px N` | on-screen triangle size. Triangle count goes as its inverse square, so this is the strongest lever there is. Densities are bucketed to powers of two, so 7 and 10 land on the same mesh. |
| `--samples N` | multisampling. Free on a modern GPU; on an integrated part sharing system memory it may dominate. `--samples 0` to find out. |
| `--size WxH` | output resolution, which also selects the mesh level. |
| `--budget-hz N` | frame rate the budget is measured against. Defaults to 120, matching `tunnelclient`'s own `max_fps` cap — which exists because vsync is unreliable on some machines, and on those the cap is what paces the loop. Pass 60 where vsync works. |

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

## Animation

The demo drives the show's real animation system rather than a copy of it:
`anim.rs` builds a `tunnels_model::animation::Animation`, sends it the same
`ControlMessage`s the console sends, and ticks it with the same `update_state`.
`waveforms` is crate-private in `tunnels_model`, but `Animation` is not, so
nothing here reimplements a waveform.

Each layer carries three animation slots, mirroring `TargetedAnimation` on a
`Tunnel`. What a slot drives decides where it is resolved, and that decides what
it costs:

| target | resolved | cost |
|---|---|---|
| hue, brightness, saturation | in the ramp texture | 1024 evaluations and one texture write, however fast it moves |
| radial, twist, squash | per vertex, at draw time | one pass over the mesh, which is never re-refined |

Every target in `Tunnel` modulates a value that also has its own knob —
`AnimationTarget::Size` adds to `size`, `AspectRatio` to `aspect_ratio`. Held
against that pattern, two of the geometry targets here are not new: a constant
radial scale is `size`, and a constant aspect animation is `aspect_ratio`. **Spin** is the same knob `Tunnel` already has, doing the analogous thing in a
different medium. On a segmented beam it turns each drawn mark about its own
centroid. A fill has no marks — an infinitesimal point has no orientation to
turn — so the same intent, orientation varying from place to place, arrives as a
shear growing with radius: centre pinned, rim carrying the full turn. Flip a
beam between marks and fill and the knob keeps meaning what it meant.

One thing does not carry over. Rotation, marquee and a segmented spin integrate
an angle forever, because turning a mark five times looks like turning it once.
Winding does not work that way: five turns at the rim stays five turns tighter
than one. So on a fill this is a bounded amount with an animator on it rather
than a speed, and `LayerParams::spin` is the base value it modulates.

With radial as the target, the animation's own knobs turn out to be shape
parameters:

| animation knob | shape meaning | superformula |
|---|---|---|
| `n_periods` | lobe count / symmetry order | m |
| `smoothing` | pointy vs. rounded lobes | n |
| `size` | deformation depth | amplitude |
| `duty_cycle` | lobe width against the flat gap | — |
| `pulse` | lobes push out only, never in | — |
| `standing` vs travelling | lobes pulse in place vs. rotate | — |

Which is to say the parametric-shape idea arrives through the animation system
rather than as a new `PathShape`, and costs no new control surface, because
those knobs already exist on the animation page.

Geometry targets are the interesting half. **Radial** scales each point's
distance from the centre — run along the angle, that deforms a disc into petals,
which is the superformula's whole trick arriving free on top of any shape in the
library. Run along the radius it pinches into rings. **Twist** rotates by an
amount that varies across the shape; along the radius that is a vortex.
**Squash** stretches one axis while squeezing the other.

Phase for the colour comes from the *undeformed* position, so a colour pattern
stays glued to the shape while a warp moves it rather than sliding across it.

See them all with `-- anim <shape index>`: rows are targets, columns are
waveforms. The last row holds the colour flat while sweeping the spin knob,
because a uniform layer takes a different path through the renderer and geometry
has to survive it.

### Colour on more than one axis

The ramp is a one-dimensional table, so only one coordinate can index it. A
colour animation on the layer's own axis is baked in, and gets the ramp's
per-fragment resolution — a discontinuity cuts cleanly. Animations on any other
axis reach the fragment through the vertices instead:

- **hue** shifts where in the ramp a vertex looks
- **brightness** rides the per-vertex tint that multiplies the sample, via
  `tri_list_uv_c`

So hue can sweep around the angle while brightness rings by radius. Saturation
is ramp-only: a multiply can darken but cannot desaturate.

The two paths differ in more than sharpness. A ramp animation is a fixed
thousand evaluations however many there are; a second-axis one is evaluated per
vertex per frame, so it scales with the mesh. The sharp option is also the cheap
one, which is a happy alignment.

Measured on the heaviest shape in the library at the default density (43k
vertices), with `-- stats`:

| | total | per vertex |
|---|---|---|
| nothing animated | 257us | 6ns |
| one colour animation, 2nd axis | 386us | 9ns |
| two colour animations, 2nd axis | 493us | 11ns |
| one geometry animation (radial) | 397us | 9ns |
| one geometry animation (spin) | 733us | 17ns |
| **one noise colour animation** | **1,502us** | **35ns** |

Noise is the outlier, and it is the one waveform that cannot be tabulated —
it reads the sample index as a second axis, so every vertex is a genuinely
different question. Spin costs more than radial because a rotation is the only
thing that forces the round trip through polar coordinates.

## Profiling the CPU pipeline

`-- bench <seconds> <layers>` runs exactly the per-frame CPU pipeline with the
rasteriser replaced by a sink, so a sampling profiler sees the work without a
GPU or a display in the way:

```
perf record --call-graph fp -F 999 -- svg_demo bench 8 3
perf report --stdio --no-children
```

Three findings from doing that, in the order they mattered:

- **Waveform evaluation was 44% of the frame**, nearly all of it `__sin` and
  friends inside `libm`. Every waveform but noise depends only on spatial phase,
  so `LiveWave` samples one into a 1024-entry table once a frame and
  interpolates. 6.0ms to 3.3ms.
- **`atan2` was then 35%**, because three separate passes each computed it for
  the same vertex. They are one pass now — the ramp coordinate is an angle, a
  spin rotates about the same centre, and a radial animation scales the same
  radius, so they all want the same polar coordinates. 3.3ms to 3.0ms.
- **`atan2` was still 26%.** `libm` is accurate to under one ULP; an angle here
  ends up as a ramp position or a vertex rotation. `fastmath::atan2` trades that
  for 2.1e-4 radians, which moves a point on the rim of a thousand-pixel shape
  by 0.15 of a pixel. The test measures the bound rather than trusting the
  comment. 3.0ms to 2.5ms.

What is left is 47% gathering and projecting vertices and 38% the per-vertex
pass. The gather is there because `tri_list_uv_c` takes pre-transformed
vertices: a vertex shader would move it to the GPU, which is the same conclusion
the fragment-shader argument reaches from the other side.

### Other limits### Other limits

- The mesh is not resubdivided under a warp, so a deformation finer than the
  triangles carrying it facets rather than curves. Visible in the noise column
  of the animation sheet, and the same ceiling that bounds noise frequency.
- A discontinuous waveform on a second colour axis bands at mesh resolution
  rather than cutting cleanly, for the same reason.

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
