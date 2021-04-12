from .animation import Animation, AnimationTarget
from .beam import Beam
from .geometry import geometry

from copy import deepcopy
from math import pi
import numpy as np
from .model_interface import ModelInterface, MiModelProperty, only_if_active
from .waveforms import sawtooth_vector, clamp_to_unit

# scale overall size, set > 1.0 to enable larger shapes than screen size
MAX_SIZE_MULT = 2.0
MAX_ASPECT = 2.0

TWOPI = 2*pi


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

class Tunnel (Beam):

    def copy(self):
        """Use deep_copy to recursively copy this Tunnel."""
        return deepcopy(self)

    def get_animation(self, anim):
        """Get an animation by index."""
        return self.anims[anim]

    def display(self, level_scale, as_mask, external_clocks):
        """Return the current state of the beam.

        Args:
            level_scale: unit float
            as_mask (bool): draw this beam as a masking layer
            external_clocks: collection of clocks that animations may be bound
                to.
        """
        size = geometry.max_size * self.size
        thickness = self.thickness

        segs = self.segs
        # for artistic reasons/convenience, eliminate odd numbers of segments
        # above 40.
        segs = segs + 1 if segs > 40 and segs % 2 else segs

        seg_num = np.array(range(segs))

        blacking = self.blacking_integer

        if blacking > 0:
            draw_segment = seg_num % abs(blacking) == 0
        else:
            draw_segment = seg_num % abs(blacking) != 0

        # use fancy indexing to only pick out the segments numbers we will draw
        seg_num = seg_num[draw_segment]
        shape = seg_num.shape

        # parameters that animations may modify
        aspect_ratio_adjust = np.zeros(shape, float)
        size_adjust = np.zeros(shape, float)
        thickness_adjust = np.zeros(shape, float)
        col_center_adjust = np.zeros(shape, float)
        col_width_adjust = np.zeros(shape, float)
        col_period_adjust = np.zeros(shape, float)
        col_sat_adjust = np.zeros(shape, float)
        x_adjust = np.zeros(shape, float)
        y_adjust = np.zeros(shape, float)

        marquee_interval = 1.0 / segs
        # the angle of this particular segment
        seg_angle = (marquee_interval*seg_num+self.curr_marquee_angle) % 1.0
        rel_angle = marquee_interval*seg_num

        # accumulate animation adjustments based on targets
        for anim in self.anims:
            target = anim.target

            anim_values = anim.get_value_vector(rel_angle, external_clocks)

            # TODO: refactor away this massive chain
            if target == AnimationTarget.Thickness:
                thickness_adjust += anim_values
            elif target == AnimationTarget.Size:
                size_adjust += anim_values * 0.5 # limit adjustment
            elif target == AnimationTarget.AspectRatio:
                aspect_ratio_adjust += anim_values
            elif target == AnimationTarget.Color:
                col_center_adjust += anim_values * 0.5
            elif target == AnimationTarget.ColorSpread:
                col_width_adjust += anim_values
            elif target == AnimationTarget.ColorPeriodicity:
                col_period_adjust += anim_values * 8
            elif target == AnimationTarget.ColorSaturation:
                col_sat_adjust += anim_values * 0.5 # limit adjustment
            elif target == AnimationTarget.PositionX:
                x_adjust += anim_values
            elif target == AnimationTarget.PositionY:
                y_adjust += anim_values

        # the abs() is there to prevent negative width setting when using multiple animations.
        stroke_weight = abs(thickness*(1 + thickness_adjust))

        thickness_allowance = thickness*geometry.thickness_scale/2

        # geometry calculations
        x_center = self.x_offset + x_adjust
        y_center = self.y_offset + y_adjust

        # this angle may exceed 1.0
        stop = (seg_angle + marquee_interval)

        rot_angle = self.curr_rot_angle
        # now set the color and draw

        draw_calls = []

        rad_x = abs((
            size*(MAX_ASPECT * (self.aspect_ratio + aspect_ratio_adjust))
            - thickness_allowance) + size_adjust)
        rad_y = abs(size - thickness_allowance + size_adjust)

        if as_mask:
            val_iter = zip(stroke_weight, x_center, y_center, rad_x, rad_y, seg_angle, stop)
            for strk, x, y, r_x, r_y, start_angle, stop_angle in val_iter:
                draw_calls.append((
                    1.0,
                    strk,
                    0.0,
                    0.0,
                    0.0,
                    x,
                    y,
                    r_x,
                    r_y,
                    start_angle,
                    stop_angle,
                    rot_angle))
        else:
            hue = (
                (self.col_center + col_center_adjust) +
                (
                    0.5*(self.col_width+col_width_adjust) *
                    sawtooth_vector(rel_angle*(int(16*self.col_spread)+col_period_adjust), 0.0, 1.0, False)
                ))

            hue = hue % 1.0

            sat = clamp_to_unit(self.col_sat + col_sat_adjust)

            val_iter = zip(hue, sat, stroke_weight, x_center, y_center, rad_x, rad_y, seg_angle, stop)

            for h, s, strk, x, y, r_x, r_y, start_angle, stop_angle in val_iter:
                draw_calls.append((
                    level_scale,
                    strk,
                    h,
                    s,
                    1.0,
                    x,
                    y,
                    r_x,
                    r_y,
                    start_angle,
                    stop_angle,
                    rot_angle))

        return draw_calls
