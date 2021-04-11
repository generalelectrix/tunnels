from abc import ABCMeta, abstractmethod

#TODO: formalize as an ABC
class Beam (object, metaclass=ABCMeta):
    """Generic Beam base class."""

    # avoid a lot of isinstance checking.
    # TODO: refactor how looks work to avoid needing this!
    is_look = False

    @abstractmethod
    def copy(self):
        """Return a deep copy of this beam."""
        pass

    @abstractmethod
    def update_state(self, timestep, external_clocks):
        """Update beam parameters based on current state.

        Args:
            timestep: duration of time over which to evolve the state of this
                beam.
            external_clocks: collection of clocks that may be referenced by
                this beam.
        """
        pass

    @abstractmethod
    def display(self, level_scale, as_mask, external_clocks):
        """Render this beam, using scaled level and masking parameter.

        External clock collection is passed in to be used by animations.
        """
        pass

    @abstractmethod
    def get_animation(self, num):
        return None

    @abstractmethod
    def replace_animation(self, anim_num, new_anim):
        pass