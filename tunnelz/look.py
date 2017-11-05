from .beam import Beam
from .shapes import ShapeCollection

class Look (Beam):
    """A look is a beam that is actually a composite of several beams."""

    is_look = True

    def __init__(self, layers):
        """Construct a new look from the contents of a mixer.

        This constructor copies everything handed to it.
        """
        super(Look, self).__init__()
        self.layers = [layer.copy() for layer in layers]

    def copy(self):
        """Return a copy of this look."""
        return Look(self.layers)

    def update_state(self, delta_t):
        """Update the state of all Beams in this Look."""
        for layer in self.layers:
            layer.beam.update_state(delta_t)

    def display(self, level_scale, as_mask):
        """Draw all the Beams in this Look.

        level_scale: unit float
        as_mask: boolean

        The individual sublayers are unpacked and returned as a single layer of
        many arc segment commands.
        """
        draw_cmds = []
        for layer in self.layers:
            # only draw a layer if it isn't off
            level = layer.level
            if level != 0:
                scaled_level = level_scale * level
                draw_cmds.extend(layer.beam.display(
                    scaled_level, as_mask or layer.mask))
        return draw_cmds

    def get_animation(self, _):
        raise TypeError("Cannot ask a look for animation.")

    def replace_animation(self, anim_num, anim):
        pass