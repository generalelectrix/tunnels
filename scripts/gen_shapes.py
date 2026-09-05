#!/usr/bin/env python3
"""Generate the procedural half of the demo shape library.

Big chunky geometry: radial mandalas, star polygons, bars, lattices. Everything
lives in a 1000x1000 viewBox centred on (500, 500) so the loader can normalise
without guessing. Several shapes are deliberately multi-contour with holes, to
exercise lyon's fill rules.
"""
import math, pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "svg_demo" / "shapes"
OUT.mkdir(parents=True, exist_ok=True)
C = 500.0

def write(name, body):
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000">'
        f'<path fill="#fff" fill-rule="evenodd" d="{body}"/></svg>'
    )
    (OUT / f"{name}.svg").write_text(svg)

def poly(pts, close=True):
    d = f"M {pts[0][0]:.2f} {pts[0][1]:.2f}"
    for x, y in pts[1:]:
        d += f" L {x:.2f} {y:.2f}"
    return d + (" Z" if close else "")

def circle(cx, cy, r, sweep=1):
    """Full circle as two arcs; sweep flips winding so it can cut a hole."""
    return (f"M {cx - r:.2f} {cy:.2f} "
            f"A {r:.2f} {r:.2f} 0 1 {sweep} {cx + r:.2f} {cy:.2f} "
            f"A {r:.2f} {r:.2f} 0 1 {sweep} {cx - r:.2f} {cy:.2f} Z")

def ring_pts(n, r, phase=0.0):
    return [(C + r * math.cos(phase + 2 * math.pi * i / n),
             C + r * math.sin(phase + 2 * math.pi * i / n)) for i in range(n)]

# --- star polygons {n/k}: the classic chunky radial figure -------------------
def star_polygon(n, k, r=460):
    pts = ring_pts(n, r, -math.pi / 2)
    order, i = [], 0
    for _ in range(n):
        order.append(pts[i]); i = (i + k) % n
    return poly(order)

for n, k in [(5, 2), (7, 2), (7, 3), (8, 3), (9, 4), (12, 5)]:
    write(f"star_{n}_{k}", star_polygon(n, k))

# --- pointed stars (alternating inner/outer radius) --------------------------
def spiky_star(n, r_out=470, r_in=190):
    pts = []
    for i in range(2 * n):
        r = r_out if i % 2 == 0 else r_in
        a = -math.pi / 2 + math.pi * i / n
        pts.append((C + r * math.cos(a), C + r * math.sin(a)))
    return poly(pts)

for n in (6, 8, 12, 16):
    write(f"spike_{n}", spiky_star(n))

# --- concentric rings: holes, and a real test of even-odd fill ---------------
def concentric(radii):
    return " ".join(circle(C, C, r, sweep=1 - (i % 2)) for i, r in enumerate(radii))

write("rings_3", concentric([470, 380, 290, 200, 110, 40]))
write("rings_wide", concentric([470, 300, 190, 70]))

# --- radial spokes ----------------------------------------------------------
def spokes(n, r_in=90, r_out=470, duty=0.45):
    half = math.pi / n * duty
    parts = []
    for i in range(n):
        a = 2 * math.pi * i / n
        parts.append(poly([
            (C + r_in * math.cos(a - half), C + r_in * math.sin(a - half)),
            (C + r_out * math.cos(a - half), C + r_out * math.sin(a - half)),
            (C + r_out * math.cos(a + half), C + r_out * math.sin(a + half)),
            (C + r_in * math.cos(a + half), C + r_in * math.sin(a + half)),
        ]))
    return " ".join(parts)

for n in (6, 8, 12, 24):
    write(f"spokes_{n}", spokes(n))

# --- sunburst: wedges all the way to the centre -----------------------------
write("sunburst_16", spokes(16, r_in=0, r_out=480, duty=0.5))
write("sunburst_32", spokes(32, r_in=0, r_out=480, duty=0.5))

# --- petal mandala: lens shapes on a ring -----------------------------------
def petals(n, r_ring=260, r_pet=200):
    parts = []
    for i in range(n):
        a = 2 * math.pi * i / n
        cx, cy = C + r_ring * math.cos(a), C + r_ring * math.sin(a)
        parts.append(circle(cx, cy, r_pet))
    return " ".join(parts)

write("mandala_petal_6", petals(6))
write("mandala_petal_8", petals(8))
write("mandala_petal_12", petals(12, r_ring=300, r_pet=170))

# --- bars and lattices ------------------------------------------------------
def slats(n, horizontal=True, duty=0.5):
    parts, pitch = [], 1000.0 / n
    for i in range(n):
        a = i * pitch
        b = a + pitch * duty
        parts.append(poly([(0, a), (1000, a), (1000, b), (0, b)] if horizontal
                          else [(a, 0), (a, 1000), (b, 1000), (b, 0)]))
    return " ".join(parts)

write("slats_8", slats(8))
write("slats_16", slats(16))
write("slats_v_8", slats(8, horizontal=False))

def grid(n, duty=0.35):
    return slats(n, True, duty) + " " + slats(n, False, duty)

write("grid_6", grid(6))
write("grid_12", grid(12))

# --- nested frames ----------------------------------------------------------
def frames(steps, thickness=45):
    parts = []
    for i in range(steps):
        inset = i * (500.0 / steps)
        o, s = inset, 1000 - 2 * inset
        parts.append(poly([(o, o), (o + s, o), (o + s, o + s), (o, o + s)]))
        o2, s2 = o + thickness, s - 2 * thickness
        if s2 > 0:
            parts.append(poly([(o2, o2), (o2, o2 + s2), (o2 + s2, o2 + s2), (o2 + s2, o2)]))
    return " ".join(parts)

write("frames_4", frames(4))
write("frames_6", frames(6, thickness=30))

# --- chevrons ---------------------------------------------------------------
def chevrons(n, thickness=55):
    parts, pitch = [], 1000.0 / n
    for i in range(n + 1):
        y = i * pitch
        parts.append(poly([(0, y), (C, y + 260), (1000, y),
                           (1000, y + thickness), (C, y + 260 + thickness), (0, y + thickness)]))
    return " ".join(parts)

write("chevron_5", chevrons(5))
write("chevron_8", chevrons(8, thickness=40))

# --- gear / cog -------------------------------------------------------------
def gear(n, r_root=330, r_tip=470, bore=120):
    pts, half = [], math.pi / n * 0.5
    for i in range(n):
        a = 2 * math.pi * i / n
        for r, off in ((r_root, -half * 1.6), (r_tip, -half), (r_tip, half), (r_root, half * 1.6)):
            pts.append((C + r * math.cos(a + off), C + r * math.sin(a + off)))
    return poly(pts) + " " + circle(C, C, bore, sweep=0)

write("gear_12", gear(12))
write("gear_20", gear(20, r_root=380, r_tip=470))

# --- cross / plus -----------------------------------------------------------
write("cross", poly([(390, 60), (610, 60), (610, 390), (940, 390), (940, 610),
                     (610, 610), (610, 940), (390, 940), (390, 610), (60, 610),
                     (60, 390), (390, 390)]))

# --- concentric polygon rings (mandala scaffold) ----------------------------
def poly_rings(n, radii):
    parts = []
    for i, r in enumerate(radii):
        pts = ring_pts(n, r, -math.pi / 2)
        parts.append(poly(pts if i % 2 == 0 else list(reversed(pts))))
    return " ".join(parts)

write("hex_rings", poly_rings(6, [470, 390, 310, 230, 150, 70]))
write("oct_rings", poly_rings(8, [470, 360, 250, 140]))

print(f"wrote {len(list(OUT.glob('*.svg')))} shapes to {OUT}")
