# Sprite attribution

The 62 figures baked into this build come from two sources. Both carry terms
that travel with the binary, so this notice and the licence text beside it are
compiled in rather than left in the repository — a render client is pushed to a
machine during a show, and a licence that stayed behind would not have shipped.

## Noto Sans Symbols 2 — 61 figures (`db_*`)

Copyright 2022 The Noto Project Authors (https://github.com/notofonts/symbols)

Licensed under the SIL Open Font License, Version 1.1. The full licence text is
in `noto-sans-symbols-2-OFL.txt` beside this file and is also compiled into the
binary. The font declares no Reserved Font Name.

Each `db_*` figure is one glyph's outline, extracted with `scripts/gen_dingbats.py`
and normalised into a unit box. That makes it a Modified Version of the Font
Software, which the licence permits and which carries three conditions this
project meets: the notice above and the licence travel with it, no figure is
sold on its own, and the derived outlines stay under the OFL.

## Wikimedia Commons — 1 figure (`wm_*`)

"Flower of Life rosette (petals filled)" by Karl432, from Wikimedia Commons:
https://commons.wikimedia.org/wiki/File:Flower_of_Life_rosette_(petals_filled).svg

Licensed under Creative Commons Attribution-ShareAlike 3.0 Unported:
https://creativecommons.org/licenses/by-sa/3.0/

The baked contours are an adaptation, so ShareAlike applies to them: they are
offered under CC BY-SA 3.0, and this is the notice that goes with them.

`CREDITS.json` beside this file records the source and licence of every figure
individually, including each glyph's codepoint.
