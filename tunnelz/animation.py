import copy
from math import sin, pi
from .clock import Clock
from .waveforms import (
    sine,
    triangle,
    square,
    sawtooth,
    sine_vector,
    triangle_vector,
    square_vector,
    sawtooth_vector,
)
import numpy as np
from .model_interface import ModelInterface, MiModelProperty, only_if_active

TWOPI = 2*pi
HALFPI = pi/2

# FIXME-NUMERIC TARGETS
class AnimationTarget (object):
    Rotation = 1#'rotation'
    Thickness = 2#'thickness'
    Size = 3#'size'
    AspectRatio = 4#'aspect_ratio'
    Color = 5#'color'
    ColorSpread = 6#'color_spread'
    ColorPeriodicity = 7#'color_periodicity'
    ColorSaturation = 8#'color_saturation'
    MarqueeRotation = 9#'marquee_rotation'
    Segments = 10#'segments'
    Blacking = 11#'blacking'
    PositionX = 12#'positionx'
    PositionY = 13#'positiony'
    PositionXY = 14#'positionxy'

    VALUES = (
        Rotation,
        Thickness,
        Size,
        AspectRatio,
        Color,
        ColorSpread,
        ColorPeriodicity,
        ColorSaturation,
        MarqueeRotation,
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

    VALUES = (Sine, Triangle, Square, Sawtooth)


class AnimationMI (ModelInterface):
    max_clock_rate = 0.0015 # radial units/ms; this is 3pi/sec

    type = MiModelProperty('type', 'set_type')
    pulse = MiModelProperty('pulse', 'set_pulse')
    invert = MiModelProperty('invert', 'set_invert')
    n_periods = MiModelProperty('n_periods', 'set_n_periods')
    target = MiModelProperty('target', 'set_target')
    speed = MiModelProperty('speed', 'set_knob', knob='speed')
    weight = MiModelProperty('weight', 'set_knob', knob='weight')
    duty_cycle = MiModelProperty('duty_cycle', 'set_knob', knob='duty_cycle')
    smoothing = MiModelProperty('smoothing', 'set_knob', knob='smoothing')

    def initialize(self):
        super(AnimationMI, self).initialize()
        self.update_controllers('set_pulse', self.model.pulse)
        self.update_controllers('set_invert', self.model.invert)
        self.update_controllers('set_knob', self._unit_speed, knob='speed')

    @property
    def _unit_speed(self):
        """Return the animator's internal clock's speed as a unit float."""
        return self.model.internal_clock.rate / self.max_clock_rate

    @property
    @only_if_active
    def speed(self):
        return self._unit_speed

    @speed.setter
    @only_if_active
    def speed(self, speed):
        self.model.internal_clock.rate = speed * self.max_clock_rate
        self.update_controllers('set_knob', speed, knob='speed')

    @only_if_active
    def toggle_pulse(self):
        val = self.model.pulse = not self.model.pulse
        self.update_controllers('set_pulse', val)

    @only_if_active
    def toggle_invert(self):
        val = self.model.invert = not self.model.invert
        self.update_controllers('set_invert', val)

scalar_waveforms = {
    WaveformType.Sine: sine,
    WaveformType.Triangle: triangle,
    WaveformType.Square: square,
    WaveformType.Sawtooth: sawtooth,
}

vector_waveforms = {
    WaveformType.Sine: sine_vector,
    WaveformType.Triangle: triangle_vector,
    WaveformType.Square: square_vector,
    WaveformType.Sawtooth: sawtooth_vector,
}

class Animation (object):
    """Generate values from a waveform given appropriate parameters."""

    wave_smoothing_scale = 0.25

    def __init__(self):
        """Start with default (benign) state for an animator."""
        self.type = WaveformType.Sine
        self.pulse = False
        self.invert = False
        self.n_periods = 0
        self.target = AnimationTarget.Size
        self.weight = 0.0 # unipolar float
        self.duty_cycle = 1.0
        self.smoothing = 0.25

        self.internal_clock = Clock()

    @property
    def clock(self):
        """Return the clock instance this animation is listening to.

        For now, always return the internal clock.
        """
        return self.internal_clock

    @property
    def active(self):
        return self.weight > 0.0

    def copy(self):
        """Return a deep copy of this animation."""
        copy_of_self = copy.copy(self)
        copy_of_self.internal_clock = self.internal_clock.copy()
        return copy_of_self

    def update_state(self, delta_t):
        if self.active:
            self.internal_clock.update_state(delta_t)

    def get_value(self, angle_offset):
        """Return the current value of the animation, with an offset."""
        if not self.active:
            return 0.

        angle = angle_offset*self.n_periods + self.clock.curr_angle
        func = scalar_waveforms[self.type]
        result = self.weight * func(angle, self.smoothing*self.wave_smoothing_scale, self.duty_cycle, self.pulse)
        if self.invert:
            return -1.0 * result
        else:
            return result

    def get_value_vector(self, angle_offsets):
        """Return the current value of the animation for an ndarray of offsets."""
        shape = angle_offsets.shape

        if not self.active:
            return np.zeros(shape, float)

        angle = angle_offsets*self.n_periods + self.clock.curr_angle
        func = vector_waveforms[self.type]

        result = self.weight * func(angle, self.smoothing*self.wave_smoothing_scale, self.duty_cycle, self.pulse)
        if self.invert:
            return -1.0 * result
        else:
            return result

