from collections import namedtuple
import msgpack
import zmq

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

# temporary hack, refactor this later
context = zmq.Context()
socket = context.socket(zmq.PUB)
socket.bind("tcp://*:6000")


class DrawCommandAggregator (object):

    def __init__(self):
        self.arcs = []

    def draw_arc(self, arc_args):
        self.arcs.append(arc_args)

    def write_to_socket(self):
        socket.send(msgpack.dumps(self.arcs, use_single_float=True))