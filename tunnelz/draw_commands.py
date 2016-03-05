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

def write_layer_to_file(layer, file):
    with open(file, 'w+') as draw_file:
        for arc in layer:
            draw_file.write(','.join(str(val) for val in arc) + '\n')