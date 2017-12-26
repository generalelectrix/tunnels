import copy
from itertools import izip, tee
from monotonic import monotonic

class Clock (object):

    def __init__(self):
        self.curr_angle = 0.0
        # in unit angle per second
        self.rate = 0.0

    def update_state(self, delta_t):
        self.curr_angle = (self.curr_angle - self.rate*delta_t) % 1.0

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