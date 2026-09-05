#!/usr/bin/env python3
"""Extract chunky geometric glyphs from Noto Sans Symbols 2 (OFL 1.1) as SVGs.

Dingbats, Ornamental Dingbats, Geometric Shapes and the Alchemical block are
full of exactly the vocabulary trees want: radial stars, florettes, pinwheels,
solid polygons. Glyphs are filtered to those that read as solid shapes rather
than hairlines, since fine line work vanishes at projection scale.
"""
import pathlib, unicodedata, re, json
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.areaPen import AreaPen
from fontTools.pens.boundsPen import BoundsPen

FONT = "/tmp/fonts/NotoSansSymbols2-Regular.ttf"
OUT = pathlib.Path(__file__).resolve().parent.parent / "svg_demo" / "shapes"
RANGES = [
    (0x25A0, 0x25FF),   # Geometric Shapes
    (0x2600, 0x26FF),   # Miscellaneous Symbols
    (0x2700, 0x27BF),   # Dingbats
    (0x1F650, 0x1F67F), # Ornamental Dingbats
    (0x1F700, 0x1F77F), # Alchemical Symbols
    (0x1F780, 0x1F7FF), # Geometric Shapes Extended
]
# Minimum fraction of the em box a glyph must span, and minimum ratio of filled
# area to bounding box — together these reject hairlines and tiny marks.
MIN_SPAN, MIN_SOLIDITY, MAX_KEEP = 0.42, 0.16, 60

# These blocks are mixed geometry and pictographs, so select on the glyph name.
# Trees want radial and rectilinear figures; a snowman or a pointing hand is
# just noise at projection scale.
KEEP_WORDS = re.compile(
    r"star|asterisk|florette|pinwheel|snowflake|sparkle|circle|square|triangle|"
    r"diamond|lozenge|pentagon|hexagon|octagon|polygon|cross|saltire|spoked|"
    r"quilt|checker|wheel|trigram|hexagram|pentagram|sun|ring|quadrant|"
    r"crosshatch|fill|shogi|bullseye|rosette|flower|sextile|compass")
DROP_WORDS = re.compile(
    r"hand|scissors|snowman|rocket|arrow|nib|pick|leaf|bud|vine|quotation|"
    r"entry|face|ball|index|writing|pencil|envelope|telephone|airplane|"
    r"skull|smiling|frowning|umbrella|comet|cloud|snow man|shogi|die |"
    r"yellow|blue|green|orange|purple|brown|red |large ")

font = TTFont(FONT)
upem = font["head"].unitsPerEm
gs = font.getGlyphSet()
cmap = font.getBestCmap()

candidates = []
for lo, hi in RANGES:
    for cp in range(lo, hi + 1):
        name = cmap.get(cp)
        if name is None:
            continue
        bounds = BoundsPen(gs)
        gs[name].draw(bounds)
        if bounds.bounds is None:
            continue
        x0, y0, x1, y1 = bounds.bounds
        w, h = x1 - x0, y1 - y0
        if w < MIN_SPAN * upem or h < MIN_SPAN * upem:
            continue
        area = AreaPen(gs)
        gs[name].draw(area)
        solidity = abs(area.value) / (w * h)
        if solidity < MIN_SOLIDITY:
            continue
        try:
            label = unicodedata.name(chr(cp)).lower()
        except ValueError:
            continue
        if not KEEP_WORDS.search(label) or DROP_WORDS.search(label):
            continue
        candidates.append((solidity, cp, name, (x0, y0, x1, y1)))

# Spread across solidity so the set carries both solid slabs and open figures
# rather than many near-identical filled blobs.
candidates.sort(key=lambda c: c[0])
step = max(1, len(candidates) // MAX_KEEP)
picked = candidates[::step][:MAX_KEEP] if len(candidates) > MAX_KEEP else candidates

manifest = []
for solidity, cp, name, (x0, y0, x1, y1) in picked:
    pen = SVGPathPen(gs)
    gs[name].draw(pen)
    d = pen.getCommands()
    if not d.strip():
        continue
    # Normalise into a 1000x1000 viewBox: centre the glyph and scale its longest
    # side to fill, flipping y since font space is y-up and SVG is y-down.
    w, h = x1 - x0, y1 - y0
    s = 1000.0 / max(w, h)
    tx = 500.0 - (x0 + w / 2) * s
    ty = 500.0 + (y0 + h / 2) * s
    label = unicodedata.name(chr(cp)).lower()
    slug = "db_" + re.sub(r"[^a-z0-9]+", "_", label).strip("_")[:46]
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000">'
        f'<g transform="matrix({s:.5f} 0 0 {-s:.5f} {tx:.3f} {ty:.3f})">'
        f'<path fill="#fff" fill-rule="nonzero" d="{d}"/></g></svg>'
    )
    (OUT / f"{slug}.svg").write_text(svg)
    manifest.append({"file": f"{slug}.svg", "codepoint": f"U+{cp:04X}",
                     "name": label, "solidity": round(solidity, 3)})

creds = json.loads((OUT / "CREDITS.json").read_text())
creds.append({"source": "Noto Sans Symbols 2, Google Fonts",
              "license": "SIL Open Font License 1.1", "glyphs": manifest})
(OUT / "CREDITS.json").write_text(json.dumps(creds, indent=2))
print(f"{len(candidates)} candidates -> kept {len(manifest)}")
for m in manifest:
    print(f"  {m['codepoint']}  {m['solidity']:.2f}  {m['name']}")
