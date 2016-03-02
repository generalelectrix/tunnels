
class Beam (object):
    """Generic Beam base class."""

    def __init__(self):
        self.curr_anim = 0

    def copy(self):
        """Return a deep copy of this beam."""
        raise NotImplementedError("Beam subclasses must implement deep copy.")

    def update_params(self):
        """Update beam parameters based on current state.

        Subclasses may override this method.
        """
        pass

    def display(self, level_scale, as_mask):
        """Render this beam, using scaled level and masking parameter.

        Subclasses should override this method.
        """
        pass

    def get_current_animation(self):
        return None

    def replace_current_animation(self, new_anim):
        pass

    def set_midi_param(self, is_note, num, val):
        pass

    def get_midi_param(self, is_note, num):
        return 0