#!/usr/bin/env python
from tunnelz.draw_commands import Arc, write_layer_to_file
from tunnelz.geometry import geometry as geo
from math import pi

test_arc = Arc(
    level=255,
    stroke=1,
    stroke_weight=10.0,
    hue=180.0,
    sat=255,
    val=255,
    x=geo.x_center,
    y=geo.y_center,
    rad_x=geo.max_radius*0.8,
    rad_y=geo.max_radius*0.8,
    start=0.,
    stop=pi/2)

write_layer_to_file([test_arc], 'testpattern.csv')