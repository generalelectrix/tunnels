import copy
from math import sin, pi
from .waveforms import (
    triangle, square, sawtooth, triangle_vector, square_vector, sawtooth_vector)
import numpy as np
from .ui import UserInterface

TWOPI = 2*pi
HALFPI = pi/2

# FIXME-NUMERIC TARGETS
class AnimationTarget (object):
    Rotation = 1#'rotation'
    Thickness = 2#'thickness'
    Radius = 3#'radius'
    Ellipse = 4#'ellipse'
    Color = 5#'color'
    ColorSpread = 6#'colorspread'
    ColorPeriodicity = 7#'colorperiodicity'
    ColorDesaturation = 8#'colordesaturation'
    Segments = 9#'segments'
    Blacking = 10#'blacking'
    PositionX = 11#'positionx'
    PositionY = 12#'positiony'
    PositionXY = 13#'positionxy'

    VALUES = (
        Rotation,
        Thickness,
        Radius,
        Ellipse,
        Color,
        ColorSpread,
        ColorPeriodicity,
        ColorDesaturation,
        #Segments,
        #Blacking,
        PositionX,
        PositionY,
        #PositionXY,
    )


class WaveformType (object):
    Sine = 'sine'
    Triangle = 'triangle'
    Square = 'square'
    Sawtooth = 'sawtooth'


class AnimationUI (UserInterface):
    def __init__(self, anim):
        super(AnimationUI, self).__init__(model=anim)

        self.type = self.ui_model_property('type', 'set_type')
        self.n_periods = self.ui_model_property('n_periods', 'set_n_periods')
        self.target = self.ui_model_property('target', 'set_target')
        self.speed = self.ui_model_property('speed', 'set_knob', knob='speed')
        self.weight = self.ui_model_property('weight', 'set_knob', knob='weight')
        #self.duty_cycle = self.ui_model_property('duty_cycle', 'set_knob', knob='duty_cycle')
        self.smoothing = self.ui_model_property('smoothing', 'set_knob', knob='smoothing')


class Animation (object):
    """Generate values from a waveform given appropriate parameters."""

    max_speed = 0.31 # radians/frame; this is about 3pi/sec at 30 fps
    wave_smoothing = pi/8.0

    def __init__(self):
        """Start with default (benign) state for an animator."""
        self.type = WaveformType.Sine
        self.n_periods = 0
        self.target = AnimationTarget.Radius
        self.speed = 0.0
        self.weight = 0
        self.duty_cycle = 0.0
        self.smoothing = 0.25

        self.curr_angle = 0.0

    @property
    def active(self):
        return self.weight > 0

    def copy(self):
        """At present, Animation only contains references to immutable types.

        We can thus just use shallow copy and everything is cool.

        In the future, when animations aren't a dumb pile of ints and floats,
        this method will need to be revisited.
        """
        return copy.copy(self)

    def update_state(self):
        if self.active:
            self.curr_angle = (self.curr_angle + self.speed*self.max_speed) % TWOPI

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
            return float(self.weight * square(angle, self.smoothing*HALFPI))
        elif self.type == 3:
            # sawtooth wave
            return float(self.weight * sawtooth(angle, self.smoothing*HALFPI))

    def get_value_vector(self, angle_offsets):
        """Return the current value of the animation for an ndarray of offsets."""
        shape = angle_offsets.shape

        if not self.active:
            return np.zeros(shape, float)

        angle = angle_offsets*self.n_periods + self.curr_angle
        if self.type == 0:
            # sine wave
            return self.weight * np.sin(angle)
        elif self.type == 1:
            # triangle wave
            return self.weight * triangle_vector(angle)
        elif self.type == 2:
            # square wave
            return self.weight * square_vector(angle, self.smoothing*HALFPI)
        elif self.type == 3:
            # sawtooth wave
            return self.weight * sawtooth_vector(angle, self.smoothing*HALFPI)


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
