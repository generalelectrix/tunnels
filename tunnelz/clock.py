import copy
from itertools import izip, tee
from monotonic import monotonic
from .model_interface import ModelInterface, MiProperty


class ControllableClock (ModelInterface):
    """A clock with a complete set of controls."""
    # if True, reset the clock's phase to zero on every tap
    retrigger = MiProperty(False, 'retrigger')

    def __init__(self):
        super(ControllableClock, self).__init__(Clock())
        self.sync = TapSync()

    @property
    def curr_angle(self):
        """Proxy the regular clock interface."""
        return self.model.curr_angle

    def tap(self):
        if self.retrigger:
            self.model.curr_angle = 0.0
        self.sync.tap()

        # for now, crudely and immediately change the clock rate if we have
        # a new estimate of what it ought to be
        new_rate = self.sync.rate
        if new_rate is not None:
            self.model.rate = new_rate

    def update_state(self, delta_t):
        """Update clock state, and update UI state as well."""
        prev_ticked_state = self.model.ticked
        self.model.update_state(delta_t)
        # if ticked state has changed, update the controllers:
        if prev_ticked_state != self.model.ticked:
            self.update_controllers('ticked', self.model.ticked)

    def nudge(self, count):
        """Nudge the phase forward or backward by count/100 of a beat."""
        adjustment = count * (self.model.rate / 100.)
        new_value = self.model.curr_angle + adjustment
        self.model.curr_angle = new_value % 1.0


class Clock (object):

    def __init__(self):
        self.curr_angle = 0.0
        # in unit angle per second
        self.rate = 0.0

        # did the clock tick on its most recent update?
        self.ticked = True

    def update_state(self, delta_t):
        # delta_t has units of ms, need to divide by 1000
        new_angle = self.curr_angle + (self.rate*delta_t/1000.)

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
        print "reset"
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
            print "new estimate: ", self._rate

    def tap(self):
        """Register a new tap event, and compute a new estimated bpm."""
        now = monotonic()
        print "tap"

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