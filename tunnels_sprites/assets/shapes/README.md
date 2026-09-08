# The figure library

A family is a directory and a figure is a file inside one. Both are named
`<two digits>_<name>`, and both are read in the order those names sort, so
ordering the library is moving a file — there is no table anywhere that also
has to agree. The build fails on anything that does not carry a prefix, rather
than sorting it somewhere surprising.

The digits are stripped to give the name, and a figure's full name is
`family/figure` — `blossoms/snowflake`. Two digits because the largest family
holds ten.

Two knobs reach these. The first picks a family, the second picks within it,
so what a family is ordered *by* is what the second knob sweeps.

## The families, and what orders each

Family order runs sharp, round, petalled, interlaced, textured, bounded, then
representational. `99_novelty` is numbered to stay last whatever is inserted
before it.

| family | what it is | ordered by |
|---|---|---|
| `01_pinwheels` | sharp points carrying a rotational skew | point count; the lighter of a matched pair first |
| `02_stars` | sharp points standing square, without the skew | point count, then how much disc the points grow out of |
| `03_asterisks` | round lobes on spokes | spoke count, then the weight of the lobe: teardrop, balloon, club |
| `04_blossoms` | petals and crystal arms | petal or arm count, then solid before open |
| `05_rosettes` | interlaced vesica arcs | petal count, then how much of the frame the figure fills |
| `06_weaves` | crossings and fills | the number of openings the mesh carries |
| `07_emblems` | figures bounded by a ring | from a bare ring through more and more structure inside it |
| `08_hands` | hands | extended fingers, then the one gripping something |
| `99_novelty` | pictures with no geometric kin | ink: the share of the frame the figure paints, measured, 15% to 35% |

Two of those are easy to mistake for something they are not.

**Weaves is ordered by opening count, not by ink.** A fine basket carries many
openings but only middling coverage, so ink would sort it near the bare
crossing it is nothing like. The openings are what make the moiré, and the
moiré is the reason these are in the library.

**Rosettes runs hollow before solid within each pair.** The two framed
rosettes paint 75–79% of the frame against the unframed pair's 20–24%, and
hollow-first is the only direction in which "how much of the frame it fills"
runs monotone across all four.

Every other family runs low to high — more points, more spokes, more petals,
more openings, more ink — so turning the second knob up always adds.
