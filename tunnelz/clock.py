import copy

class Clock (object):

    def __init__(self):
        self.curr_angle = 0.0
        # in unit angle per second
        self.rate = 0.0

    def update_state(self, delta_t):
        self.curr_angle = (self.curr_angle - self.rate*delta_t) % 1.0

    def copy(self):
        return copy.copy(self)
