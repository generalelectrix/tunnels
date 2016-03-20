"""MI handler for actions that involve multiple other MIs.

Ideally, MIs shouldn't need to know about each other, and this orchestrator
deals with the few actions that need to be coordinated across each of them.
"""
from .animation import AnimationClipboard
from .beam_matrix_minder import BeamMatrixMI
from .model_interface import ModelInterface, MiProperty

class MetaMI (ModelInterface):

    current_layer = MiProperty(0, 'set_current_layer')

    def __init__(self, mixer_mi, tunnel_mi, animator_mi, beam_matrix):
        super(MetaMI, self).__init__(None)
        self.mixer_mi = mixer_mi
        self.tunnel_mi = tunnel_mi
        self.animator_mi = animator_mi
        self.beam_matrix_mi = BeamMatrixMI(beam_matrix, self)
        self.animation_clipboard = AnimationClipboard()

    def initialize(self):
        super(MetaMI, self).initialize()
        self.beam_matrix_mi.initialize()
        self._update_current_layer()

    def set_current_layer(self, layer):
        """Set which layer is the current layer being edited.

        Do nothing if the active layer already is this layer.

        Args:
            layer (int): the numeric layer to select.  Should be in the range
                of layers defined in the mixer.
        """
        if layer != self.current_layer:
            self.current_layer = layer
            self._update_current_layer()

    def get_current_beam(self):
        """Return the beam which is currently being edited."""
        return self.mixer_mi.mixer.get_beam_from_layer(self.current_layer)

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

    def _update_current_layer(self):
        """If we're done something that requires a full UI redraw, call this."""
        # TODO: alter mixer MI to add this interface?  Do I care?
        active_beam = self.get_current_beam()

        # if we're putting a look, we need to temporarily deactive the tunnel
        # and animator UI
        islook = active_beam.is_look

        self.tunnel_mi.active = not islook
        self.animator_mi.active = not islook

        self.tunnel_mi.swap_model(active_beam)

        # TODO: do we want to store this in the beam?  It implies that our
        # stateful meta-UI abstraction is a bit leaky, but it may provide a
        # nicer user experience.
        active_animator = active_beam.get_current_animation()
        if not islook:
            self.animator_mi.swap_model(active_animator)

    def set_current_animator(self, anim_num):
        """Set which animator is being edited.

        Do nothing if this is already the active animator.

        Args:
            anim_num (int): the numeric animator to select.
        """
        beam = self.get_current_beam()
        if not beam.is_look:
            if anim_num != beam.curr_anim:
                beam.curr_anim = anim_num
                animator = beam.get_current_animation()
                self.animator_mi.swap_model(animator)
                self.update_controllers('set_current_animator', anim_num)

    def animation_copy(self):
        """Copy the current animator to the clipboard."""
        # TODO: which animator is authoritatively the current one - the current
        # beam's current animator, or the one loaded into the animator ui?
        if not self.get_current_beam().is_look:
            self.animation_clipboard.copy(self.animator_mi.model)

    def animation_paste(self):
        """Paste the clipboard into the current beam's current animator slot."""
        if not self.get_current_beam().is_look:
            animator = self.animation_clipboard.paste()
            if animator is not None:
                beam = self.get_current_beam()
                beam.replace_current_animation(animator)
                self.animator_mi.swap_model(animator)
