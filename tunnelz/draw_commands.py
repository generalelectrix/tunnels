from .draw_commands_pb2 import DrawCommands

class DrawCommandAggregator (object):

    def __init__(self):
        self.dc = DrawCommands()

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