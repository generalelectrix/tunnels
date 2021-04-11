from .tunnel import Tunnel
from .look import Look
from .model_interface import ModelInterface
import logging

class MixerMI (ModelInterface):
    """Handle model interactions for the mixer."""
    def __init__(self, mixer):
        super(MixerMI, self).__init__(mixer)
        self.mixer = mixer

    def initialize(self):
        super(MixerMI, self).initialize()
        for i, layer in enumerate(self.mixer.layers):
            self.update_controllers('set_look_indicator', i, isinstance(layer.beam, Look))
            self.update_controllers('set_level', i, layer.level)
            self.update_controllers('set_bump_button', i, layer.bump)
            self.update_controllers('set_mask_button', i, layer.mask)
            for chan in range(self.mixer.n_video_channels):
                self.update_controllers(
                    'set_video_channel', i, chan, chan in layer.video_outs)

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

    def toggle_video_channel(self, layer, channel):
        state = self.mixer.toggle_video_channel(layer, channel)
        self.update_controllers('set_video_channel', layer, channel, state)


class MixerLayer (object):
    """Data bag for the contents of a mixer channel.

    By default, a mixer channel outputs to video feed 0.
    """
    def __init__(self, beam, level=0.0, bump=False, mask=False, video_outs=None):
        self.beam = beam
        self.level = level
        self.bump = bump
        self.mask = mask
        self.video_outs = {0} if video_outs is None else video_outs

    def copy(self):
        return MixerLayer(
            beam=self.beam.copy(),
            level=self.level,
            bump=self.bump,
            mask=self.mask,
            video_outs=self.video_outs.copy())


class Mixer (object):
    """Holds a collection of beams in layers, and understands how they are mixed.

    Args:
        n_layers: Create this many mixer channges.
        n_video_channels: Make this number of virtual video channels available.
        test_mode: If True, disable high-level exception handling to allow
            exceptioons to bubble up rather than catching and logging.
    """
    def __init__(self, n_layers, n_video_channels, test_mode):
        self.test_mode = test_mode
        self.n_video_channels = n_video_channels
        self.layers = [MixerLayer(Tunnel()) for _ in range(n_layers)]

    def update_state(self, delta_t, external_clocks):
        """Update the state of all of the beams contained in this mixer."""
        for i, layer in enumerate(self.layers):
            # catch update errors from individual beams to avoid crashing the
            # entire console if one beam has an error.
            try:
                layer.beam.update_state(delta_t, external_clocks)
            except Exception:
                if self.test_mode:
                    raise
                else:
                    logging.exception(
                        "Exception while updating beam in layer %d.",
                        i)

    @property
    def layer_count(self):
        return len(self.layers)

    def put_beam_in_layer(self, layer, beam):
        self.layers[layer].beam = beam

    def get_beam_from_layer(self, layer):
        return self.layers[layer].beam

    def set_level(self, layer, level):
        """level: unit float"""
        self.layers[layer].level = level

    def bump_on(self, layer):
        self.layers[layer].bump = True

    def bump_off(self, layer):
        self.layers[layer].bump = False

    def toggle_mask_state(self, layer):
        self.layers[layer].mask = mask_state = not self.layers[layer].mask
        return mask_state

    def toggle_video_channel(self, layer, channel):
        """Toggle the whether layer is drawn to video channel.

        Return the new state of display of this channel.
        """
        assert channel < self.n_video_channels
        layer_video_outs = self.layers[layer].video_outs
        if channel in layer_video_outs:
            layer_video_outs.remove(channel)
            return False
        else:
            layer_video_outs.add(channel)
            return True

    def video_channel_in(self, layer, channel):
        """Draw this layer on the specified channel."""
        self.layers[layer].video_outs.add(channel)

    def video_channel_out(self, layer, channel):
        """Do not draw this layer on the specified channel."""
        assert channel < self.n_video_channels
        self.layers[layer].video_outs.discard(channel)

    def draw_layers(self, external_clocks):
        """Return a list of lists of draw commands.

        Each inner list represents one virtual video channel.

        Args:
            external_clocks: collection of external clocks that animators may be
                bound to.
        """
        video_outs = [[] for _ in range(self.n_video_channels)]

        for i, layer in enumerate(self.layers):
            level = layer.level
            bump = layer.bump

            try:
                if level > 0 or bump:
                    if bump:
                        draw_cmd = layer.beam.display(1.0, layer.mask, external_clocks)
                    else:
                        draw_cmd = layer.beam.display(level, layer.mask, external_clocks)
                else:
                    draw_cmd = []
            except Exception:
                if self.test_mode:
                    raise
                else:
                    logging.exception(
                        "Exception while displaying beam in layer %d.",
                        i)
                    draw_cmd = []

            for video_chan in layer.video_outs:
                video_outs[video_chan].append(draw_cmd)

        return video_outs

    def get_copy_of_current_look(self):
        """Return a frozen copy of the entire current look."""
        return Look(self.layers)

    def set_look(self, look):
        """Unload a look into the mixer state, clobbering current state."""
        # It appears this method was ill-formed in the Java version, as a
        # incoming look's mask and level state does not clobber the mixer.
        # Seems like mask at least should clobber, or your ugly mask layer
        # becomes a positive.  Hell, here, I'll fix it right now.

        self.layers = look.copy().layers
