from collections import namedtuple
import os
import tempfile

arc_args = (
    'level', # int 0-255
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

def write_layers_to_file(layers, file):
    with tempfile.NamedTemporaryFile(
            dir=os.path.dirname(file), delete=False) as tmpfile:
        for layer in layers:
            for arc in layer:
                tmpfile.write(','.join(str(val) for val in arc) + '\n')
    os.rename(tmpfile.name, file)