from collections import namedtuple
from itertools import izip

color = namedtuple('Color', ('hue', 'sat', 'val'))

def color_vector(hues, sats, val):
    return [color(h, s, val) for (h, s) in izip(hues, sats)]