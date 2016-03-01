from itertools import izip
from .beam import Beam

class Look (Beam):
    """A look is a beam that is actually a composite of several beams."""

    def __init__(self, layers, levels, masks):
        """Construct a new look from the contents of a mixer.

        This constructor does not copy anything.

        layers, levels, and masks are all lists of the mixer channel values.
        """
        self.layers = layers
        self.levels = levels
        self.masks = masks

    def copy(self):
        """Return a deep copy of this look."""
        return Look(
            layers=[beam.copy() for beam in self.layers],
            levels=list(self.levels),
            masks=list(self.masks),
        )

    def display(self, level_scale, as_mask):
        """Draw all the Beams in this Look.

        level: int in [0, 255]
        as_mask: boolean
        """

        for layer, level, mask in izip(self.layers, self.levels, self.masks):
            # only draw a layer if it isn't off
            if level != 0:
                scaled_level = level_scale * level / 255
                layer.display(scaled_level, as_mask or mask)