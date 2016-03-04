from collections import namedtuple

arc_args = (
    'level', # int 0-255
    'stroke', # bool as 0 or 1
    'stroke_weight', # float
    'hue',
    'sat',
    'val',
    'x', # int
    'y', # int
    'rad_x', #int
    'rad_y', #int
    'start', #float
    'stop' #float
    )

Arc = namedtuple('Arc', arc_args)