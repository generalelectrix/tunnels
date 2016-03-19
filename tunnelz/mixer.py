from .draw_commands import DrawCommandAggregator
from .tunnel import Tunnel
from .look import Look
from .ui import UserInterface

class MixerUI (UserInterface):
    """Handle user interactions for the mixer."""
    def __init__(self, mixer):
        super(MixerUI, self).__init__(mixer)
        self.mixer = mixer

    def initialize(self):
        super(MixerUI, self).initialize()
        for i, layer in enumerate(self.mixer.layers):
            self.update_controllers('set_look_indicator', i, isinstance(layer.beam, Look))
            self.update_controllers('set_level', i, layer.level)
            self.update_controllers('set_bump_button', i, layer.bump)
            self.update_controllers('set_mask_button', i, layer.mask)

    def put_beam_in_layer(self, layer, beam):
        """Replace the beam in numbered layer with this beam."""
        self.mixer.put_beam_in_layer(layer, beam)
        self.update_controllers('set_look_indicator', layer, isinstance(beam, Look))

    def set_look(self, look):
        """Set the current look, clobbering mixer state."""
        self.mixer.set_look(look)
        self.initialize()

    def set_level(self, layer, level):
        self.mixer.set_level(layer, level)
        self.update_controllers('set_level', layer, level)

    def set_bump_button(self, layer, state):
        if state:
            self.mixer.bump_on(layer)
        else:
            self.mixer.bump_off(layer)
        self.update_controllers('set_bump_button', layer, state)

    def toggle_mask_state(self, layer):
        state = self.mixer.toggle_mask_state(layer)
        self.update_controllers('set_mask_button', layer, state)


class MixerLayer (object):
    """Data bag for the contents of a mixer channel."""
    def __init__(self, beam, level=0, bump=False, mask=False):
        self.beam = beam
        self.level = level
        self.bump = bump
        self.mask = mask

    def copy(self):
        return MixerLayer(
            beam=self.beam.copy(),
            level=self.level,
            bump=self.bump,
            mask=self.mask)


class Mixer (object):
    """Holds a collection of beams in layers, and understands how they are mixed."""
    def __init__(self, n_layers):
        self.n_layers = n_layers
        self.layers = [MixerLayer(Tunnel()) for _ in xrange(n_layers)]

    def put_beam_in_layer(self, layer, beam):
        self.layers[layer].beam = beam

    def get_beam_from_layer(self, layer):
        return self.layers[layer].beam

    def set_level(self, layer, level):
        """level: int in [0, 255]"""
        self.layers[layer].level = level

    def bump_on(self, layer):
        self.layers[layer].bump = True

    def bump_off(self, layer):
        self.layers[layer].bump = False

    def toggle_mask_state(self, layer):
        self.layers[layer].mask = mask_state = not self.layers[layer].mask
        return mask_state

    def draw_layers(self):
        dc_agg = DrawCommandAggregator()
        for layer in self.layers:
            level = layer.level
            bump = layer.bump

            if level > 0 or bump:
                if bump:
                    layer.beam.display(255, layer.mask, dc_agg)
                else:
                    layer.beam.display(level, layer.mask, dc_agg)
        return dc_agg

    def get_copy_of_current_look(self):
        """Return a frozen copy of the entire current look."""
        return Look(self.layers)

    def set_look(self, look):
        """Unload a look into the mixer state, clobbering current state."""
        # It appears this method was ill-formed in the Java version, as a
        # incoming look's mask and level state does not clobber the mixer.
        # Seems like mask at least should clobber, or your ugly mask layer
        # becomes a positive.  Hell, here, I'll fix it right now.
        # TODO: should we clobber level as well?

        self.layers = look.copy().layers
