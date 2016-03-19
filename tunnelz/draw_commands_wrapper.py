#from .draw_commands import DrawCommands
import capnp
import draw_commands_capnp
from itertools import izip
from collections import namedtuple
import os
import tempfile
import msgpack

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

def arc_to_str(arc):
    return "%d,%f,%f,%f,%d,%d,%d,%d,%d,%f,%f\n" % arc

def write_layers_to_file(layers, file):
    with tempfile.NamedTemporaryFile(
            dir=os.path.dirname(file), delete=False) as tmpfile:
        for layer in layers:
            for arc in layer:
                tmpfile.write(','.join(str(val) for val in arc) + '\n')
    os.rename(tmpfile.name, file)

class DrawCommandAggregatorDumb (object):

    def __init__(self):
        self.arcs = []

    def draw_arc(self, arc_args):
        self.arcs.append(arc_args)

    def write_to_file(self, path):
        with tempfile.NamedTemporaryFile(
                dir=os.path.dirname(path), delete=False) as tmpfile:
            for arc in self.arcs:
                tmpfile.write(arc_to_str(arc))
        os.rename(tmpfile.name, path)


class DrawCommandAggregatorMsgpack (object):

    def __init__(self):
        self.arcs = []

    def draw_arc(self, arc_args):
        self.arcs.append(arc_args)

    def write_to_file(self, path):
        with tempfile.NamedTemporaryFile(
                dir=os.path.dirname(path), delete=False) as tmpfile:
            msgpack.pack(self.arcs, tmpfile, use_single_float=True)
        os.rename(tmpfile.name, path)


class DrawCommandAggregatorProtoBuf (object):

#    def __init__(self):
#        self.dc = DrawCommands()

    def draw_arc(
            self,
            level,
            stroke_weight,
            hue,
            sat,
            val,
            x,
            y,
            rad_x,
            rad_y,
            start,
            stop,):
        arc = self.dc.arcs.add()
        arc.level = level
        arc.stroke_weight = stroke_weight
        arc.hue = hue
        arc.sat = sat
        arc.val = val
        arc.x = x
        arc.y = y
        arc.rad_x = rad_x
        arc.rad_y = rad_y
        arc.start = start
        arc.stop = stop

    def write_to_file(self, path):
        with open(path, 'w+') as f:
            f.write(self.dc.SerializeToString())


class DrawCommandAggregatorCapnProto (object):

    def __init__(self):
        self.dc = draw_commands_capnp.DrawCommands.new_message()
        self.arcs = []

    def draw_arc(self, args):
        self.arcs.append(args)

    def write_to_file(self, path):
        n_arcs = len(self.arcs)
        arcs = self.dc.init('arcs', n_arcs)
        for arc, arc_tup in izip(arcs, self.arcs):

            arc.level = arc_tup[0]
            arc.strokeWeight = arc_tup[1]
            arc.hue = arc_tup[2]
            arc.sat = arc_tup[3]
            arc.val = arc_tup[4]
            arc.x = arc_tup[5]
            arc.y = arc_tup[6]
            arc.radX = arc_tup[7]
            arc.radY = arc_tup[8]
            arc.start = arc_tup[9]
            arc.stop = arc_tup[10]

        with tempfile.NamedTemporaryFile(
                dir=os.path.dirname(path), delete=False) as tmpfile:
            self.dc.write(tmpfile)
        os.rename(tmpfile.name, path)

DrawCommandAggregator = DrawCommandAggregatorMsgpack
