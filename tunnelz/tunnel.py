from .animation import Animation
from .beam import Beam
from .color import color
from .draw_commands import Arc
from .geometry import geometry
from itertools import izip
from copy import deepcopy
from .button_LED import set_anim_select_LED
from math import pi
import numpy as np
from .ui import UserInterface
from .waveforms import sawtooth, sawtooth_vector

# scale overall radius, set > 1.0 to enable larger shapes than screen size
MAX_RAD_MULT = 2.0
MAX_ELLIPSE_ASPECT = 2.0

TWOPI = 2*pi


class TunnelUI (UserInterface):

    def __init__(self, tunnel):
        super(TunnelUI, self).__init__(model=tunnel)

        self.rot_speed = self.ui_model_property('rot_speed', 'set_knob', knob='rot_speed')
        self.thickness = self.ui_model_property('thickness', 'set_knob', knob='thickness')
        self.radius = self.ui_model_property('radius', 'set_knob', knob='radius')
        self.ellipse_aspect = self.ui_model_property('ellipse_aspect', 'set_knob', knob='ellipse_aspect')
        self.col_center = self.ui_model_property('col_center', 'set_knob', knob='col_center')
        self.col_width = self.ui_model_property('col_width', 'set_knob', knob='col_width')
        self.col_spread = self.ui_model_property('col_spread', 'set_knob', knob='col_spread')
        self.col_sat = self.ui_model_property('col_sat', 'set_knob', knob='col_sat')
        self.segs = self.ui_model_property('segs', 'set_knob', knob='segs')
        self.blacking = self.ui_model_property('blacking', 'set_knob', knob='blacking')

        self.x_nudge, self.y_nudge = geometry.x_nudge, geometry.y_nudge

    def nudge_x_pos(self):
        """Nudge the beam in the +x direction."""
        self.model.x_offset += self.x_nudge

    def nudge_x_neg(self):
        """Nudge the beam in the -x direction."""
        self.model.x_offset -= self.x_nudge

    def nudge_y_pos(self):
        """Nudge the beam in the +y direction."""
        self.model.y_offset += self.y_nudge

    def nudge_y_neg(self):
        """Nudge the beam in the -y direction."""
        self.model.y_offset -= self.y_nudge

    def reset_beam_position(self):
        """Reset the beam to center."""
        self.model.x_offset = 0
        self.model.y_offset = 0


class Tunnel (Beam):
    """Ellipsoidal tunnels.

    The workhorse.
    The one and only.

    Ladies and Gentlemen, presenting, The Beating Heart of The Circle Machine.

    Retained Java for doc purposes:

    # integer-valued parameters, derived from midi inputs and used to initialize the beam
    int rotSpeedI, thicknessI, radiusI, ellipseAspectI;
    int colCenterI, colWidthI, colSpreadI, colSatI;
    int segsI, blackingI;

    # scaled parameters used for drawing, set in updateParams() method.
    float rotSpeed, thickness, ellipseAspect;
    int radius;
    int colCenter, colWidth, colSpread, colSat;
    int segs, blacking;

    # rotation angles
    float currAngle, rotInterval;

    # location
    int xOffset, yOffset;

    # constants
    int rotSpeedScale = 400;
    int blackingScale = 4;

    # resolution-dependent parameters:
    int xNudge, yNudge;
    float thicknessScale;

    # array of colors for current parameters
    color[] segmentColors;

    # animations
    int nAnim = 4;
    Animation[] theAnims = new Animation[nAnim];
    int currAnim;
    """
    n_anim = 4
    rot_speed_scale = 0.155 # tunnel rotates this many rad/frame
    blacking_scale = 4

    def __init__(self):
        """Default tunnel constructor."""
        super(Tunnel, self).__init__()
        self.rot_speed = 0.0
        self.thickness = 0.25
        self.radius = 0.5
        self.ellipse_aspect = 0.5

        self.col_centerI = 0
        self.col_widthI = 0
        self.col_spreadI = 0
        self.col_satI = 127

        self.segsI = 126
        self.blackingI = 72

        self.curr_angle = 0.0

        self.x_offset, self.y_offset = 0, 0

        self.anims = [Animation() for _ in xrange(self.n_anim)]

        self.update_params()

    def update_params(self):
        """Called whenever a beam parameter is changed, by midi for example.

        This is where parameter scaling occurs.
        """
        # update internal parameters from integer values

        rot_speedI = self.rot_speedI
        if 65 < rot_speedI:
            self.rot_speed = float((rot_speedI-65))/self.rot_speed_scale
        elif 63 > rot_speedI:
            self.rot_speed = -float((-rot_speedI+63))/self.rot_speed_scale
        else:
            self.rot_speed = 0.0

        self.col_center = self.col_centerI * 2
        self.col_width = self.col_widthI
        self.col_spread = self.col_spreadI / 8
        self.col_sat = (127 - self.col_satI) * 2 # we have a "desaturate" knob, not a saturate knob.

        # THIS IS A HACK.  This only works because the APC40 doesn't put out 0 for the bottom of the knob.
        segs = self.segs = self.segsI
        self.rot_interval = 2*pi / segs # ALSO A HACK, same reason

        self.blacking = (self.blackingI - 64) / self.blacking_scale

        # ensure we don't offset beyond the maximum
        self.x_offset = min(max(self.x_offset, -geometry.max_x_offset), geometry.max_x_offset)
        self.y_offset = min(max(self.y_offset, -geometry.max_y_offset), geometry.max_y_offset)

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

    def display(self, level_scale, as_mask):
        """Draw the current state of the beam.

        Args:
            level_scale: int in [0, 255]
            as_mask (bool): draw this beam as a masking layer
        """

        rot_adjust, ellipse_adjust = 0.0, 0.0

        # update the state of the animations and get relevant values
        for anim in self.anims:

            anim.update_state()

            target = anim.target

            # what is this animation targeting?
            # at least for non-chicklet-level targets...
            if target == 1: # rotation speed
                rot_adjust += anim.get_value(0)
            if target == 4: # ellipsing
                ellipse_adjust += anim.get_value(0)


        # calulcate the rotation, wrap to 0 to 2pi
        self.curr_angle = (
            self.curr_angle +
            (self.rot_speed + rot_adjust)*self.rot_speed_scale) % TWOPI

        radius = int(MAX_RAD_MULT * geometry.max_radius * self.radius)
        thickness = self.thickness * geometry.thickness_scale

        rad_X = radius*(MAX_ELLIPSE_ASPECT * (self.ellipse_aspect + ellipse_adjust)) - thickness/2
        rad_Y = radius - thickness/2

        arcs = self.draw_segments_with_animations(
            rad_X, rad_Y, self.segs, as_mask, level_scale)

        # loop over segments and draw arcs
        # arcs = []
        # for i in xrange(self.segs):

        #     arcs.append(self.draw_segment_with_animation(
        #         rad_X, rad_Y, i, as_mask, level_scale))
        return arcs

    def draw_segments_with_animations(
        self, rad_X, rad_Y, n_segs, as_mask, level_scale):
        """Vectorized draw of all of the segments."""
        # first determine which segments are going to be drawn at all using the
        # blacking parameter
        seg_num = np.array(xrange(n_segs))

        # FIXME: negative blacking doesn't work
        # blacking_mode: True for standard, False for inverted

        blacking = self.blacking

        # remove the "all segments blacked" bug
        if blacking == -1:
            blacking = 0

        blacking_mode = blacking >= 0
        if blacking >= 0:
            # constrain min to 1 to avoid divide by zero error
            blacking = max(self.blacking, 1)

            draw_segment = seg_num % abs(blacking) == 0
        else:
            draw_segment = seg_num % abs(blacking) != 0

        seg_num = seg_num[draw_segment]
        shape = seg_num.shape

        # parameters that animations may modify
        rad_adjust = np.zeros(shape, float)
        thickness_adjust = np.zeros(shape, float)
        col_center_adjust = np.zeros(shape, float)
        col_width_adjust = np.zeros(shape, float)
        col_period_adjust = np.zeros(shape, float)
        col_sat_adjust = np.zeros(shape, float)
        x_adjust = 0
        y_adjust = 0

        # the angle of this particular segment
        seg_angle = self.rot_interval*seg_num+self.curr_angle
        rel_angle = self.rot_interval*seg_num

        for anim in self.anims:
            target = anim.target

            # what is this animation targeting?
            if target == 2: # thickness
                    thickness_adjust += anim.get_value_vector(rel_angle)
            elif target == 3: # radius
                    rad_adjust += anim.get_value_vector(rel_angle)
            elif target == 5: # color center
                    col_center_adjust += anim.get_value(0)
            elif target == 6: # color width
                    col_width_adjust += anim.get_value(0)
            elif target == 7: # color periodicity
                    col_period_adjust += anim.get_value(0) / 16
            elif target == 8: # saturation
                    col_sat_adjust += anim.get_value_vector(rel_angle)
            elif target == 11: # x offset
                    x_adjust += anim.get_value(0)*(geometry.x_size/2)/127
            elif target == 12: # y offset
                    y_adjust += anim.get_value(0)*(geometry.y_size/2)/127

        # the abs() is there to prevent negative width setting when using multiple animations.
        stroke_weight = abs(self.thickness*(1 + thickness_adjust/127))

        # geometry calculations
        x_center = geometry.x_center + self.x_offset + int(x_adjust)
        y_center = geometry.y_center + self.y_offset + int(y_adjust)
        rad_x_vec = abs(rad_X + rad_adjust)
        rad_y_vec = abs(rad_Y+ rad_adjust)
        stop = seg_angle + self.rot_interval

        arcs = []
        # now set the color and draw
        if as_mask:
            val_iter = izip(stroke_weight, rad_x_vec, rad_y_vec, seg_angle, stop)
            for strk, r_x, r_y, start_angle, stop_angle in val_iter:
                arcs.append(Arc(
                    level=255,
                    stroke_weight=strk,
                    hue=0.0,
                    sat=0.0,
                    val=0,
                    x=x_center,
                    y=y_center,
                    rad_x=r_x,
                    rad_y=r_y,
                    start=start_angle,
                    stop=stop_angle))
        else:
            hue = (
                self.col_center +
                col_center_adjust +
                (
                    (self.col_width+col_width_adjust) *
                    sawtooth_vector(rel_angle*(self.col_spread+col_period_adjust), 0))
                )

            hue = hue % 256

            sat = self.col_sat + col_sat_adjust

            level = level_scale

            val_iter = izip(hue, sat, stroke_weight, rad_x_vec, rad_y_vec, seg_angle, stop)

            for h, s, strk, r_x, r_y, start_angle, stop_angle in val_iter:
                arcs.append(Arc(
                    level=level,
                    stroke_weight=strk,
                    hue=h,
                    sat=s,
                    val=255,
                    x=x_center,
                    y=y_center,
                    rad_x=r_x,
                    rad_y=r_y,
                    start=start_angle,
                    stop=stop_angle))
        return arcs


    # def draw_segment_with_animation(
    #         self, rad_X, rad_Y, seg_num, as_mask, level_scale):
    #     """actually draws a tunnel segment given animation parameters

    #     Args:
    #         float rad_X, float rad_Y, int seg_num, boolean as_mask, int level_scale
    #     """



    #     # parameters that animations may modify
    #     rad_adjust = 0.
    #     thickness_adjust = 0.
    #     col_center_adjust = 0.
    #     col_width_adjust = 0.
    #     col_period_adjust = 0.
    #     col_sat_adjust = 0.
    #     x_adjust = 0
    #     y_adjust = 0

    #     # 90 fps
    #     # the angle of this particular segment
    #     seg_angle = self.rot_interval*seg_num+self.curr_angle
    #     rel_angle = self.rot_interval*seg_num

    #     for anim in self.anims:
    #         target = anim.target

    #         # what is this animation targeting?
    #         if target == 2: # thickness
    #                 thickness_adjust += anim.get_value(rel_angle)
    #         elif target == 3: # radius
    #                 rad_adjust += anim.get_value(rel_angle)
    #         elif target == 5: # color center
    #                 col_center_adjust += anim.get_value(0)
    #         elif target == 6: # color width
    #                 col_width_adjust += anim.get_value(0)
    #         elif target == 7: # color periodicity
    #                 col_period_adjust += anim.get_value(0) / 16
    #         elif target == 8: # saturation
    #                 col_sat_adjust += anim.get_value(rel_angle)
    #         elif target == 11: # x offset
    #                 x_adjust += anim.get_value(0)*(geometry.x_size/2)/127
    #         elif target == 12: # y offset
    #                 y_adjust += anim.get_value(0)*(geometry.y_size/2)/127

    #     # 45 fps
    #     # the abs() is there to prevent negative width setting when using multiple animations.
    #     stroke_weight = abs(self.thickness*(1 + thickness_adjust/127))
    #     # FIXME-RENDERING: Processing draw call
    #     # strokeWeight( stroke_weight )

    #     # now set the color

    #     # FIXME: negative blacking doesn't work
    #     # blacking_mode: True for standard, False for inverted
    #     # constrain min to 1 to avoid divide by zero error
    #     blacking = max(self.blacking, 1)
    #     blacking_mode = blacking >= 0

    #     # if no blacking at all or if this is not a blacked segment, draw it
    #     # the blacking == -1 is a hack to remove the "all segments blacked" bug
    #     black_this_segment = seg_num % abs(blacking) != 0
    #     if (blacking == 0 or blacking == -1 or
    #         blacking_mode != black_this_segment):

    #         hue = (
    #             self.col_center +
    #             col_center_adjust +
    #             (
    #                 (self.col_width+col_width_adjust) *
    #                 sawtooth(rel_angle*(self.col_spread+col_period_adjust), 0))
    #             )

    #         # wrap the hue index
    #         while hue > 255:
    #             hue = hue - 255
    #         while hue < 0:
    #             hue = hue + 255
    #     # 24 fps
    #         # FIXME-COLOR: use of Processing color object
    #         seg_color = color(hue, self.col_sat + col_sat_adjust, 255)
    #     # otherwise this is a blacked segment.
    #     else:
    #         # FIXME-COLOR
    #         seg_color = color(0, 0, 0)

    #     # only draw something if the segment color isn't black.
    #     # FIXME-COLOR
    #     # FIXME: this might be bugged
    #     if color(0, 0, 0) != seg_color:

    #         # if we're drawing this beam as a mask, make the segment black
    #         if as_mask:
    #             # FIXME-RENDERING
    #             # stroke(0)
    #             stroke = True
    #             seg_color = color(0, 0, 0)
    #             level = 255
    #         # otherwise pick the color and set the level
    #         else:
    #             # FIXME-RENDERING
    #             # stroke( blendColor(seg_color, color(0,0,level_scale), MULTIPLY) )
    #             stroke = True
    #             level = level_scale
    #     else:
    #         # FIXME-RENDERING
    #         # noStroke()
    #         stroke = False
    #         level = level_scale

    #     # 20 fps

    #     # draw pie wedge for this cell
    #     # FIXME-RENDERING
    #     #print "segment"
    #     return Arc(
    #         level=level,
    #         stroke=int(stroke),
    #         stroke_weight=stroke_weight,
    #         hue=seg_color.hue,
    #         sat=seg_color.sat,
    #         val=seg_color.val,
    #         x=geometry.x_center + self.x_offset + x_adjust,
    #         y=geometry.y_center + self.y_offset + y_adjust,
    #         rad_x=abs(rad_X + rad_adjust),
    #         rad_y=abs(rad_Y+ rad_adjust),
    #         start=seg_angle,
    #         stop=seg_angle + self.rot_interval,)
    #     # 17 fps
    #     # return Arc(
    #     #     level=255,
    #     #     stroke=1,
    #     #     stroke_weight=10.0,
    #     #     hue=100.,
    #     #     sat=100.,
    #     #     val=255,
    #     #     x=0,
    #     #     y=0,
    #     #     rad_x=500,
    #     #     rad_y=500,
    #     #     start=0.,
    #     #     stop=3.0)
    #     # arc(
    #     #     X_CENTER + self.x_offset + x_adjust,
    #     #     Y_CENTER+ self.y_offset + y_adjust,
    #     #     abs(rad_X + rad_adjust),
    #     #     abs(rad_Y+ rad_adjust),
    #     #     seg_angle,
    #     #     seg_angle + self.rot_interval,)


    def get_midi_param(self, is_note, num):
        """get the midi-scaled value for a control parameter"""

        if not is_note:
            if num == 16: # color center
                return self.col_centerI
            if num == 17: # color width
                return self.col_widthI
            if num == 18: # color spread
                return self.col_spreadI
            if num == 19: # saturation
                return self.col_satI

            # geometry parameters: bottom of lower bank
            if num == 20: # rotation speed
                return self.rot_speedI
            if num == 21: # thickness
                return self.thicknessI
            if num == 22: # radius
                return self.radiusI
            if num == 23: # ellipse aspect ratio
                return self.ellipse_aspectI

            # segments parameters: bottom of upper bank

            if num == 52: # number of segments
                return self.segsI
            if num == 53: # blacking
                return self.blackingI

            # animation parameters: top of upper bank
            # /* fix this code
            # FIXME: wtf was this comment about fix this code talking about...
            if num == 48:
                return self.get_current_animation().speedI
            if num == 49:
                return self.get_current_animation().weightI
            if num == 50:
                return self.get_current_animation().duty_cycleI
            if num == 51:
                return self.get_current_animation().smoothingI
        # if we asked for any other value, return 0
        return 0
