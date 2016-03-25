from .animation import Animation, AnimationTarget
from .beam import Beam
from .geometry import geometry
from itertools import izip
from copy import deepcopy
from math import pi
import numpy as np
from .model_interface import ModelInterface, MiModelProperty, only_if_active
from .waveforms import sawtooth_vector

# scale overall radius, set > 1.0 to enable larger shapes than screen size
MAX_RAD_MULT = 2.0
MAX_ELLIPSE_ASPECT = 2.0

TWOPI = 2*pi


class TunnelMI (ModelInterface):

    rot_speed = MiModelProperty('rot_speed', 'set_bipolar', knob='rot_speed')
    thickness = MiModelProperty('thickness', 'set_unipolar', knob='thickness')
    radius = MiModelProperty('radius', 'set_unipolar', knob='radius')
    ellipse_aspect = MiModelProperty('ellipse_aspect', 'set_unipolar', knob='ellipse_aspect')
    col_center = MiModelProperty('col_center', 'set_unipolar', knob='col_center')
    col_width = MiModelProperty('col_width', 'set_unipolar', knob='col_width')
    col_spread = MiModelProperty('col_spread', 'set_unipolar', knob='col_spread')
    col_sat = MiModelProperty('col_sat', 'set_unipolar', knob='col_sat')
    segs = MiModelProperty('segs', 'set_segs')
    blacking = MiModelProperty('blacking', 'set_blacking')

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


class Tunnel (Beam):
    """Ellipsoidal tunnels.

    The workhorse.
    The one and only.

    Ladies and Gentlemen, presenting, The Beating Heart of The Circle Machine.

    TODO: docstring
    """
    n_anim = 4
    rot_speed_scale = 0.15 # tunnel rotates this many rad/frame
    blacking_scale = 4

    def __init__(self):
        """Default tunnel constructor."""
        super(Tunnel, self).__init__()
        self.rot_speed = 0.0 # bipolar float
        self.thickness = 0.25 # unipolar float
        self.radius = 0.5 # unipolar float
        self.ellipse_aspect = 0.5 # unipolar float

        self.col_center = 0.0 # unipolar float
        self.col_width = 0.0 # unipolar float
        self.col_spread = 0.0 # unipolar float
        self.col_sat = 0.0 # unipolar float

        self.segs = 126 # positive int; could be any number, but previously [0,127]

        # TODO: regularize segs and blacking interface into regular float knobs
        self.blacking = 2 # number of segments to black; int on range [-16, 16]

        self.curr_angle = 0.0

        self.x_offset, self.y_offset = 0.0, 0.0

        self.anims = [Animation() for _ in xrange(self.n_anim)]

    def copy(self):
        """Use deep_copy to recursively copy this Tunnel."""
        return deepcopy(self)

    def get_animation(self, anim):
        """Get an animation by index."""
        return self.anims[anim]

    def get_current_animation(self):
        return self.get_animation(self.curr_anim)

    def replace_current_animation(self, new_anim):
        """Replace the current animation with another."""
        self.anims[self.curr_anim] = new_anim

    def update_state(self):
        """Update the state of this tunnel in preparation for drawing a frame."""
        # ensure we don't exceed the set bounds of the screen
        self.x_offset = min(max(self.x_offset, -geometry.max_x_offset), geometry.max_x_offset)
        self.y_offset = min(max(self.y_offset, -geometry.max_y_offset), geometry.max_y_offset)

        rot_adjust = 0.0

        # update the state of the animations and get relevant values
        for anim in self.anims:

            anim.update_state()
            target = anim.target

            # what is this animation targeting?
            # at least for non-chicklet-level targets...
            if target == AnimationTarget.Rotation: # rotation speed
                rot_adjust += anim.get_value(0)

        # calulcate the rotation, wrap to 0 to 2pi
        self.curr_angle = (
            self.curr_angle +
            (self.rot_speed + rot_adjust)*self.rot_speed_scale) % TWOPI


    def display(self, level_scale, as_mask, dc_agg):
        """Draw the current state of the beam.

        Args:
            level_scale: int in [0, 255]
            as_mask (bool): draw this beam as a masking layer
            dc_agg (list to aggregate draw commands)
        """
        radius = geometry.max_radius * self.radius
        thickness = self.thickness

        seg_num = np.array(xrange(self.segs))

        blacking = self.blacking
        # remove the "all segments blacked" bug
        if blacking == -1:
            blacking = 0

        if blacking >= 0:
            # constrain min to 1 to avoid divide by zero error
            blacking = max(self.blacking, 1)

            draw_segment = seg_num % abs(blacking) == 0
        else:
            draw_segment = seg_num % abs(blacking) != 0

        # use fancy indexing to only pick out the segments numbers we will draw
        seg_num = seg_num[draw_segment]
        shape = seg_num.shape

        # parameters that animations may modify
        ellipse_adjust = np.zeros(shape, float)
        rad_adjust = np.zeros(shape, float)
        thickness_adjust = np.zeros(shape, float)
        col_center_adjust = np.zeros(shape, float)
        col_width_adjust = np.zeros(shape, float)
        col_period_adjust = np.zeros(shape, float)
        col_sat_adjust = np.zeros(shape, float)
        x_adjust = np.zeros(shape, float)
        y_adjust = np.zeros(shape, float)

        rot_interval = TWOPI / self.segs
        # the angle of this particular segment
        seg_angle = rot_interval*seg_num+self.curr_angle
        rel_angle = rot_interval*seg_num

        for anim in self.anims:
            target = anim.target

            # what is this animation targeting?
            if target == AnimationTarget.Thickness:
                thickness_adjust += anim.get_value_vector(rel_angle)
            elif target == AnimationTarget.Radius:
                rad_adjust += anim.get_value_vector(rel_angle)
            if target == AnimationTarget.Ellipse: # ellipsing
                ellipse_adjust += anim.get_value_vector(rel_angle)
            elif target == AnimationTarget.Color:
                col_center_adjust += anim.get_value_vector(rel_angle)
            elif target == AnimationTarget.ColorSpread:
                col_width_adjust += anim.get_value_vector(rel_angle)
            elif target == AnimationTarget.ColorPeriodicity:
                col_period_adjust += anim.get_value_vector(rel_angle) / 16
            elif target == AnimationTarget.ColorSaturation:
                col_sat_adjust += anim.get_value_vector(rel_angle)
            elif target == AnimationTarget.PositionX:
                x_adjust += anim.get_value_vector(rel_angle)/127
            elif target == AnimationTarget.PositionY:
                y_adjust += anim.get_value_vector(rel_angle)/127

        # the abs() is there to prevent negative width setting when using multiple animations.
        stroke_weight = abs(thickness*(1 + thickness_adjust/127))

        thickness_allowance = thickness*geometry.thickness_scale/2

        rad_x = abs((
            radius*(MAX_ELLIPSE_ASPECT * (self.ellipse_aspect + ellipse_adjust/127))
            - thickness_allowance) + rad_adjust/255)
        rad_y = abs(radius - thickness_allowance + rad_adjust/255)

        # geometry calculations
        x_center = self.x_offset + x_adjust
        y_center = self.y_offset + y_adjust
        stop = seg_angle + rot_interval

        arcs = []
        # now set the color and draw
        if as_mask:
            val_iter = izip(stroke_weight, x_center, y_center, rad_x, rad_y, seg_angle, stop)
            for strk, x, y, r_x, r_y, start_angle, stop_angle in val_iter:
                dc_agg.append((
                    255,
                    strk,
                    0.0,
                    0.0,
                    0,
                    x,
                    y,
                    r_x,
                    r_y,
                    start_angle,
                    stop_angle))
        else:
            hue = (
                255*self.col_center +
                col_center_adjust +
                (
                    (127*self.col_width+col_width_adjust) *
                    sawtooth_vector(rel_angle*(16*self.col_spread+col_period_adjust), 0.0))
                )

            hue = hue % 256

            sat = 255*self.col_sat + col_sat_adjust

            val_iter = izip(hue, sat, stroke_weight, x_center, y_center, rad_x, rad_y, seg_angle, stop)

            for h, s, strk, x, y, r_x, r_y, start_angle, stop_angle in val_iter:
                dc_agg.append((
                    level_scale,
                    strk,
                    h,
                    s,
                    255,
                    x,
                    y,
                    r_x,
                    r_y,
                    start_angle,
                    stop_angle))
        return arcs
