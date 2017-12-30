import copy
from itertools import izip, tee
from monotonic import monotonic
from .model_interface import ModelInterface, MiProperty, MiModelProperty


class ControllableClock (ModelInterface):
    """A clock with a complete set of controls."""
    # if True, reset the clock's phase to zero on every tap
    retrigger = MiProperty(False, 'set_retrigger')
    one_shot = MiModelProperty('one_shot', 'set_one_shot')

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


class TickTimer (object):
    """Determine when to turn on and off a tick indicator.

    Has some time-specific latching behavior to make the ticks longer than a
    single frame.
    """
    # minimum time between tick on and tick off, in ms
    min_duration = 100

    def __init__(self):
        # if None, then we are idling.
        # otherwise, we turned the tick on and we are waiting to turn it off
        # again.
        self.tick_age = None

    def update_state(self, delta_t):
        if self.tick_age is not None:
            self.tick_age += delta_t

    def emit_update(self):
        if self.tick_age is None:
            return None
        elif self.tick_age >= self.min_duration:
            self.tick_age = None
            return False
        else:
            return



class Clock (object):

    def __init__(self):
        self.curr_angle = 0.0
        # in unit angle per second
        self.rate = 0.0

        # did the clock tick on its most recent update?
        self.ticked = True

        # is this clock running in "one-shot" mode?
        # the clock runs for one cycle when triggered then waits for another
        # trigger event
        self.one_shot = False

        # should this clock reset and tick on its next update?
        self.reset_on_update = False

    def update_state(self, delta_t):

        if self.reset_on_update:
            self.ticked = True
            self.curr_angle = 0.0
            self.reset_on_update = False
        else:
            # delta_t has units of ms, need to divide by 1000
            new_angle = self.curr_angle + (self.rate*delta_t/1000.)

            # if we're running in one-shot mode, clamp the angle at 1.0
            if self.one_shot and new_angle >= 1.0:
                self.curr_angle = 1.0
                self.ticked = False
            else:
                # if the phase just escaped our range, we ticked this frame
                self.ticked = new_angle >= 1.0 or new_angle < 0.0
                self.curr_angle = new_angle % 1.0

    def copy(self):
        return copy.copy(self)


class TapSync (object):
    """Handle estimation of phase and rate from a series of taps."""
    # fractional threshold at which we'll discard the current tap buffer and
    # start a new one
    reset_threshold = 0.1

    def __init__(self):
        self._tap_times = []
        self._rate = None
        self._period = None

    @property
    def rate(self):
        return self._rate

    def _reset_buffer(self, tap_time):
        self._tap_times = [tap_time]
        self._rate = None
        self._period = None

    def _add_tap(self, tap_time):
        """Add a tap time to the buffer and update our value estimates if possible."""
        self._tap_times.append(tap_time)
        if len(self._tap_times) > 1:
            # iterate over all pairs of times to get differences
            first_iter, second_iter = tee(self._tap_times)
            next(second_iter, None)
            deltas = list(second - first for first, second in izip(first_iter, second_iter))

            self._period = sum(deltas) / len(deltas)
            self._rate = 1.0 / self._period

    def tap(self):
        """Register a new tap event, and compute a new estimated bpm."""
        now = monotonic()

        # if the tap buffer isn't empty, determine elapsed time from the last
        # tap to this one
        if self._period is not None:
            dt = now - self._tap_times[-1]

            # if this single estimate of tempo is within +-10% of current, use it
            # otherwise, empty the buffer and start over
            fractional_difference = (self._period - dt) / self._period

            if abs(fractional_difference) > 0.1:
                # outlier, empty the buffer
                self._reset_buffer(now)
            else:
                # append to buffer and update
                self._add_tap(now)
        else:
            self._add_tap(now)