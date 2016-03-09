"""UI handler for actions that involve multiple other UIs.

Ideally, UIs shouldn't need to know about each other, and this orchestrator
deals with the few actions that need to be coordinated across each of them.
"""
from .beam_matrix_minder import BeamMatrixUI
from .ui import UserInterface, ui_method

class MetaUI (UserInterface):

    def __init__(self, mixer_ui, beam_ui, animator_ui, beam_matrix):
        super(MetaUI, self).__init__(None)
        self.mixer_ui = mixer_ui
        self.beam_ui = beam_ui
        self.animator_ui = animator_ui
        self.beam_matrix_ui = BeamMatrixUI(beam_matrix, self)

        self.current_layer = self.ui_property(0, 'set_current_layer')

        self.initialize()

    def initialize(self):
        super(MetaUI, self).initialize()
        # TODO: initialize useful properties

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
        return self.mixer_ui.mixer.get_beam_from_layer(self.current_layer)

    def replace_current_beam(self, beam):
        """Replace the beam in the layer which is currently being edited."""
        self.mixer_ui.put_beam_in_layer(self.current_layer, beam)
        self._update_current_layer()

    def set_look(self, look):
        """Replace the entire mixer state with the contents of a look."""
        self.mixer_ui.set_look(look)
        self._update_current_layer()

    def _update_current_layer(self):
        """If we're done something that requires a full UI redraw, call this."""
        # TODO: alter mixer UI to add this interface?  Do I care?
        active_beam = self.get_current_beam()
        self.beam_ui.swap_model(active_beam)

        # TODO: do we want to store this in the beam?  It implies that our
        # stateful meta-UI abstraction is a bit leaky, but it may provide a
        # nicer user experience.
        active_animator = active_beam.get_current_animation()
        self.animator_ui.swap_model(active_animator)

    def set_current_animator(self, anim_num):
        """Set which animator is being edited.

        Do nothing if this is already the active animator.

        Args:
            anim_num (int): the numeric animator to select.
        """
        beam = self.get_current_beam()
        if anim_num != beam.curr_anim:
            beam.curr_anim = anim_num
            animator = beam.get_current_animation()
            self.animator_ui.swap_model(animator)
            self.update_controllers('set_current_animator', anim_num)

