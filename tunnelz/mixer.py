from .beam import Beam
from .look import Look

class MixerUI (object):
    """Handle user interactions for the mixer.

    Owns a list of BeamUI, one for each mixer layer.
    """
    def __init__(self, mixer):
        self.mixer = mixer
        self._current_layer = 0
        self.controllers = set()

        # make fresh beam UIs
        self.beam_ui = [BeamUI(beam) for beam in mixer.layers]

    @property
    def current_layer(self):
        return self._current_layer

    @current_layer.setter
    def current_layer(self, layer):
        """If we are changing which layer is current, update UI."""
        if self._current_layer != layer:
            self._current_layer = layer
            for controller in self.controllers:
                controller.set_mixer_layer(layer)

    def get_current_beam(self):
        """Return the beam in the currently selected layer."""
        return self.mixer.get_beam_from_layer(self.current_layer)

    def replace_current_beam(self, beam):
        """Replace the beam in the currently selected layer with this beam.

        Also re-associated the beam UI for that mixer layer.
        """
        self.mixer.put_beam_in_layer(self.current_layer, beam)
        self.beam_ui[self.current_layer].associate_with(beam)


class Mixer (object):
    """Holds a collection of beams in layers, and understands how they are mixed."""
    def __init__(self, n_layers):
        self.n_layers = n_layers
        self.layers = [Beam() for _ in xrange(n_layers)]
        self.levels = [0 for _ in xrange(n_layers)]
        self.bump = [False for _ in xrange(n_layers)]
        self.mask = [False for _ in xrange(n_layers)]
        self.current_layer = 0

    def put_beam_in_layer(self, layer, beam):
        self.layers[layer] = beam

    def get_beam_from_layer(self, layer):
        return self.layers[layer]

    def get_current_beam(self):
        return self.layers[self.current_layer]

    def set_current_beam(self, beam):
        self.put_beam_in_layer(self.current_layer, beam)

    def set_level(self, layer, level):
        """level: int in [0, 255]"""
        self.levels[layer] = level

    def bump_on(self, layer):
        self.bump[layer] = True

    def bump_off(self, layer):
        self.bump[layer] = False

    def toggle_mask_state(self, layer):
        self.mask[layer] = mask_state = not self.mask[layer]
        return mask_state

    def draw_layers(self):
        draw_commands = []
        for i in xrange(self.n_layers):
            level = self.levels[i]
            bump = self.bump[i]

            if level > 0 or bump:
                beam = self.layers[i]
                if bump:
                    draw_commands.append(beam.display(255, self.mask[i]))
                else:
                    draw_commands.append(beam.display(level, self.mask[i]))
            else:
                draw_commands.append([])
        return draw_commands

    def get_copy_of_current_look(self):
        """Return a frozen copy of the entire current look."""
        return Look(self.layers, self.levels, self.mask)

    def set_look(self, look):
        """Unload a look into the mixer state, clobbering current state."""
        # It appears this method was ill-formed in the Java version, as a
        # incoming look's mask and level state does not clobber the mixer.
        # Seems like mask at least should clobber, or your ugly mask layer
        # becomes a positive.  Hell, here, I'll fix it right now.
        # TODO: should we clobber level as well?

        n_beams_in_look = len(look.layers)

        for i in xrange(self.n_layers):
            if i < n_beams_in_look:
                self.layers[i] = look.layers[i]
                self.mask[i] = look.mask[i]
            else:
                self.layers[i] = Beam()
                self.mask[i] = False
