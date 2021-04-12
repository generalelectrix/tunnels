import copy
from math import pi
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




class AnimationMI (ModelInterface):
    # radial units/ms; this is 3pi/sec
    # the negative sign is here so that turning the animation speed knob
    # clockwise makes the animation appear to run around the beam in the same
    # direction
    max_clock_rate = -1.5

    type = MiModelProperty('type', 'set_type')
    pulse = MiModelProperty('pulse', 'set_pulse')
    invert = MiModelProperty('invert', 'set_invert')
    n_periods = MiModelProperty('n_periods', 'set_n_periods')
    target = MiModelProperty('target', 'set_target')
    speed = MiModelProperty('speed', 'set_knob', knob='speed')
    weight = MiModelProperty('weight', 'set_knob', knob='weight')
    duty_cycle = MiModelProperty('duty_cycle', 'set_knob', knob='duty_cycle')
    smoothing = MiModelProperty('smoothing', 'set_knob', knob='smoothing')
    clock = MiModelProperty('clock_source', 'set_clock_source')

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

