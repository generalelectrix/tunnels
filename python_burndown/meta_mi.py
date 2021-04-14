"""MI handler for actions that involve multiple other MIs.

Ideally, MIs shouldn't need to know about each other, and this orchestrator
deals with the few actions that need to be coordinated across each of them.
"""
from weakref import WeakKeyDictionary
from .animation import Animation
from .beam_matrix_minder import BeamMatrixMI
from .model_interface import ModelInterface, MiProperty
import logging as log

class DefaultWeakKeyDictionary (object):
    """Combine the behavior of defaultdict and WeakKeyDictionary."""
    def __init__(self, defaultfunc):
        self.d = WeakKeyDictionary()
        self.defaultfunc = defaultfunc

    def __getitem__(self, key):
        try:
            return self.d[key]
        except KeyError:
            val = self.defaultfunc()
            self.d[key] = val
            return val

    def __setitem__(self, key, val):
        self.d[key] = val



class MetaMI (ModelInterface):




    def replace_current_beam(self, beam):
        """Replace the beam in the layer which is currently being edited."""
        self.mixer_mi.put_beam_in_layer(self.current_layer, beam)
        self._update_current_layer()

    def get_copy_of_current_look(self):
        """Get a copy of the current mixer state."""
        return self.mixer_mi.mixer.get_copy_of_current_look()

    def set_look(self, look):
        """Replace the entire mixer state with the contents of a look."""
        self.mixer_mi.set_look(look)
        self._update_current_layer()

    def animation_copy(self):
        """Copy the current animator to the clipboard."""
        if not self.get_current_beam().is_look:
            self.animation_clipboard = self.animator_mi.model.copy()

    def animation_paste(self):
        """Paste the clipboard into the current beam's current animator slot."""
        beam = self.get_current_beam()
        if not beam.is_look:
            animator = self.animation_clipboard.copy()
            if animator is not None:
                beam.replace_animation(self.active_animator_number, animator)
                self.animator_mi.swap_model(animator)
