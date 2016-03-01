import copy
from math import sin, pi
from .util import unwrap
from .waveforms import triangle, square, sawtooth

class Animation (object):
    """Wow, what a clusterfuck.

    midi-driven parameters:
    int typeI, speedI, weightI, targetI, nPeriodsI, dutyCycleI, smoothingI;

    scaled parameters
    int type; // 0 = sine, 1 = triangle, 2 = square, 3 = sawtooth
    int nPeriods;
    float speed;
    int weight;
    float dutyCycle;
    float smoothing;
    int target; // tricky to figure out how we want to do this...
    /*
        0 none
        1 rotation
        2 thickness
        3 radius
        4 ellipse
        5 color
        6 spread
        7 periodicity
        8 saturation
        9 segments
        10 blacking
        11 x
        12 y
        13 x + y
    */

    // internal variables
    float currAngle;
    boolean active;
    """

    speed_scale = 200
    wave_smoothing = pi/8.0

    def __init__(self):
        """Start with default (benign) state for an aniamtor."""
        self.typeI = 24
        self.speedI = 64
        self.weightI = 0
        self.targetI = 36
        self.n_periodsI = 0
        self.duty_cycleI = 0
        self.smoothingI = 32

        self.curr_angle = 0.0

        # at the moment we create some rather important attributes for the first
        # time in this method, due to differences in how Java and Python classes
        # are declared.  Unpythonic but not broken, for now.
        self.update_params()

    def copy(self):
        """At present, Animation only contains references to immutable types.

        We can thus just use shallow copy and everything is cool.

        In the future, when animations aren't a dumb pile of ints and floats,
        this method will need to be revisited.
        """
        return copy.copy(self)

    def update_params(self):
        self.type = self.typeI - 24
        self.n_periods = self.n_periodsI

        speedI = self.speedI
        if speedI > 65:
            self.speed = -float((speedI - 65)/self.speed_scale)
        elif speedI < 63:
            self.speed = float((-speedI + 63)/self.speed_scale)
        else:
            self.speed = 0.0

        self.weight = self.weightI
        self.active = True if self.weightI > 0 else False

        self.target = self.targetI - 35 + 1 # really need to do this mapping in a more explicit way...

        self.duty_cycle = float(self.duty_cycleI / 127)

        self.smoothing = float((pi/2) * (self.smoothingI / 127))

    def update_state(self):
        if self.active:
            self.curr_angle = unwrap(self.curr_angle + self.speed)

    def get_value(self, angle_offset):
        """Return the current value of the animation, with an offset."""
        if not self.active:
            return 0.

        angle = angle_offset*self.n_periods + self.curr_angle
        if self.type == 0:
            # sine wave
            return float(self.weight * sin(angle))
        elif self.type == 1:
            # triangle wave
            return float(self.weight * triangle(angle))
        elif self.type == 2:
            # square wave
            return float(self.weight * square(angle, self.smoothing))
        elif self.type == 3:
            # sawtooth wave
            return float(self.weight * sawtooth(angle, self.smoothing))


class AnimationClipboard (object):
    """Class for storing a deep copy of an animation to support copy/paste."""
    def __init__(self):
        self.anim = None
        self.has_data = False

    def copy(self, to_copy):
        self.anim = to_copy.copy()
        self.has_data = True

    def paste(self):
        return self.anim.copy()
