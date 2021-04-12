from .animation import Animation, AnimationTarget
from .beam import Beam
from .geometry import geometry

from copy import deepcopy
from math import pi
import numpy as np
from .model_interface import ModelInterface, MiModelProperty, only_if_active
from .waveforms import sawtooth_vector, clamp_to_unit


class TunnelMI (ModelInterface):

    marquee_speed = MiModelProperty('marquee_speed', 'set_bipolar', knob='marquee_speed')
    rot_speed = MiModelProperty('rot_speed', 'set_bipolar', knob='rot_speed')
    thickness = MiModelProperty('thickness', 'set_unipolar', knob='thickness')
    size = MiModelProperty('size', 'set_unipolar', knob='size')
    aspect_ratio = MiModelProperty('aspect_ratio', 'set_unipolar', knob='aspect_ratio')
    col_center = MiModelProperty('col_center', 'set_unipolar', knob='col_center')
    col_width = MiModelProperty('col_width', 'set_unipolar', knob='col_width')
    col_spread = MiModelProperty('col_spread', 'set_unipolar', knob='col_spread')
    col_sat = MiModelProperty('col_sat', 'set_unipolar', knob='col_sat')
    segs = MiModelProperty('segs', 'set_segs')
    blacking = MiModelProperty('blacking', 'set_bipolar', knob='blacking')

    def __init__(self, tunnel):
        super(TunnelMI, self).__init__(model=tunnel)

        self.x_nudge, self.y_nudge = geometry.x_nudge, geometry.y_nudge

    @only_if_active
    def nudge_x_pos(self):
        """Nudge the beam in the +x direction."""
        self.model.x_offset += self.x_nudge

    @only_if_active
    def nudge_x_neg(self):
        """Nudge the beam in the -x direction."""
        self.model.x_offset -= self.x_nudge

    @only_if_active
    def nudge_y_pos(self):
        """Nudge the beam in the +y direction."""
        self.model.y_offset += self.y_nudge

    @only_if_active
    def nudge_y_neg(self):
        """Nudge the beam in the -y direction."""
        self.model.y_offset -= self.y_nudge

    @only_if_active
    def reset_beam_position(self):
        """Reset the beam to center."""
        self.model.x_offset = 0.0
        self.model.y_offset = 0.0

    @only_if_active
    def reset_beam_rotation(self):
        """Reset beam rotation offset angle to 0 and rotation speed to 0."""
        self.rot_speed = 0.0
        self.model.curr_rot_angle = 0.0

    @only_if_active
    def reset_beam_marquee(self):
        """Reset beam marquee offset angle to 0 and marquee speed to 0."""
        self.marquee_speed = 0.0
        self.model.curr_marquee_angle = 0.0
