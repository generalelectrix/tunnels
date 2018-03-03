from .beam import Beam

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

    def update_state(self, delta_t, external_clocks):
        """Update the state of all Beams in this Look."""
        for layer in self.layers:
            layer.beam.update_state(delta_t, external_clocks)

    def display(self, level_scale, as_mask, external_clocks):
        """Draw all the Beams in this Look.

        Args:
            level_scale: unit float
            as_mask: boolean
            external_clocks: collection of clocks

        The individual sublayers are unpacked and returned as a single layer of
        many arc segment commands.  The layer punch-ins are ignored and every
        active layer is drawn.
        """
        draw_cmds = []
        for layer in self.layers:
            # only draw a layer if it isn't off
            level = layer.level
            if level != 0:
                scaled_level = level_scale * level
                draw_cmds.extend(layer.beam.display(
                    scaled_level, as_mask or layer.mask, external_clocks))
        return draw_cmds

    def get_animation(self, _):
        raise TypeError("Cannot ask a look for animation.")

    def replace_animation(self, anim_num, anim):
        pass