#!/usr/bin/env python3
"""Generate the tunnels sunburst icon SVG."""
import math
import colorsys
import os

cx, cy = 256, 256
r_circle = 256  # fills entire viewBox
r_beams = 362   # sqrt(256^2 + 256^2), reaches corners
r_hole = 10.0
hole_color = "#b3b3b3"  # 70% white, mimics finite projector contrast
n_triangles = 32
slice_deg = 360.0 / (n_triangles * 2)

paths = []
for i in range(n_triangles):
    a0 = math.radians(i * 2 * slice_deg)
    a1 = math.radians(i * 2 * slice_deg + slice_deg)
    x0 = cx + r_beams * math.cos(a0)
    y0 = cy + r_beams * math.sin(a0)
    x1 = cx + r_beams * math.cos(a1)
    y1 = cy + r_beams * math.sin(a1)

    hue = i / n_triangles
    r, g, b = colorsys.hsv_to_rgb(hue, 1.0, 1.0)
    color = f"#{int(r*255):02x}{int(g*255):02x}{int(b*255):02x}"

    paths.append(
        f'  <path d="M{cx},{cy}L{x0:.2f},{y0:.2f}L{x1:.2f},{y1:.2f}Z" fill="{color}"/>'
    )

svg = "\n".join([
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">',
    '  <rect width="512" height="512" fill="black"/>',
    *paths,
    f'  <circle cx="{cx}" cy="{cy}" r="{r_hole}" fill="{hole_color}"/>',
    "</svg>",
    "",
])

out = os.path.join(os.path.dirname(__file__), "..", "resources", "tunnels-icon.svg")
with open(out, "w") as f:
    f.write(svg)
print(f"Written to {out}")
