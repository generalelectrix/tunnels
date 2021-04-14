import copy
from itertools import tee
from time import monotonic
from .model_interface import ModelInterface, MiProperty, MiModelProperty


class ControllableClock (ModelInterface):
    """A clock with a complete set of controls."""
    # if True, reset the clock's phase to zero on every tap
    retrigger = MiProperty(False, 'set_retrigger')
    one_shot = MiModelProperty('one_shot', 'set_one_shot')
    submaster_level = MiModelProperty('submaster_level', 'set_submaster_level')

    # time between turning the tick indicator on and then off again, in ms
    min_tick_display_duration = 250

    def __init__(self):
        super(ControllableClock, self).__init__(Clock())
        self.sync = TapSync()
        # keep track of how long it has been since we turned a tick indicator on
        self._tick_age = None

    def initialize(self):
        super(ControllableClock, self).initialize()
        self._set_tick_indicator_off()

    def _set_tick_indicator_off(self):
        self.update_controllers('ticked', False)

    @property
    def curr_angle(self):
        """Proxy the regular clock interface."""
        return self.model.curr_angle

    def tap(self):
        if self.retrigger:
            self.model.reset_on_update = True
        else:
            self.sync.tap()

            # for now, crudely and immediately change the clock rate if we have
            # a new estimate of what it ought to be
            new_rate = self.sync.rate
            if new_rate is not None:
                self.model.rate = new_rate

    def update_state(self, delta_t):
        """Update clock state, and update UI state as well."""
        self.model.update_state(delta_t)

        # if the clock just ticked, reset the tick age counter
        if self.model.ticked:
            self.update_controllers('ticked', True)
            self._tick_age = 0
        # if we're waiting to reset the tick counter
        elif self._tick_age is not None:
            # age the tick time
            self._tick_age += delta_t
            if self._tick_age >= self.min_tick_display_duration:
                self._tick_age = None
                self._set_tick_indicator_off()

    def nudge(self, count):
        """Nudge the phase forward or backward by count/100 of a beat."""
        adjustment = count * (self.model.rate / 100.)
        new_value = self.model.curr_angle + adjustment
        self.model.curr_angle = new_value % 1.0
