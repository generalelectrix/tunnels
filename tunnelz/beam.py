from abc import ABCMeta, abstractmethod

#TODO: formalize as an ABC
class Beam (object):
    """Generic Beam base class."""
    __metaclass__ = ABCMeta

    # avoid a lot of isinstance checking.
    # TODO: refactor how looks work to avoid needing this!
    is_look = False

    @abstractmethod
    def copy(self):
        """Return a deep copy of this beam."""
        pass

    @abstractmethod
    def update_state(self, timestep):
        """Update beam parameters based on current state.

        Subclasses should override this method.
        """
        pass

    @abstractmethod
    def display(self, level_scale, as_mask):
        """Render this beam, using scaled level and masking parameter.

        Subclasses should override this method.
        """
        pass

    @abstractmethod
    def get_animation(self, num):
        return None

    @abstractmethod
    def replace_animation(self, anim_num, new_anim):
        pass