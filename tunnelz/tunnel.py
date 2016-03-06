from .animation import Animation
from .beam import Beam
from .color import color
from .draw_commands import Arc
from .geometry import geometry
from copy import deepcopy
from .button_LED import set_anim_select_LED
from math import pi
import numpy as np
from .util import unwrap
from .waveforms import sawtooth

# scale overall radius, set > 1.0 to enable larger shapes than screen size
MAX_RAD_MULT = 2.0
MAX_ELLIPSE_ASPECT = 2.0

TWOPI = 2*pi

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
    rot_speed_scale = 400
    blacking_scale = 4

    def __init__(self):
        """Default tunnel constructor."""
        super(Tunnel, self).__init__()
        self.rot_speedI = 64
        self.thicknessI = 32
        self.radiusI = 64
        self.ellipse_aspectI = 64

        self.col_centerI = 0
        self.col_widthI = 0
        self.col_spreadI = 0
        self.col_satI = 127

        self.segsI = 126
        self.blackingI = 72

        self.curr_angle = 0.0

        self.x_offset, self.y_offset = 0, 0

        self.x_nudge, self.y_nudge = geometry.x_nudge, geometry.y_nudge
        self.thickness_scale = geometry.thickness_scale

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


        self.thickness = float(self.thicknessI*self.thickness_scale)

        # in pixels, I think
        self.radius = int(MAX_RAD_MULT * geometry.max_radius * self.radiusI / 127)
        self.ellipse_aspect = float(MAX_ELLIPSE_ASPECT * self.ellipse_aspectI / 127.)

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
                rot_adjust += anim.get_value(0);
            if target == 4: # ellipsing
                ellipse_adjust += anim.get_value(0);


        # calulcate the rotation, wrap to 0 to 2pi
        self.curr_angle = (
            self.curr_angle + self.rot_speed + rot_adjust/self.rot_speed_scale) % TWOPI

        radius = self.radius
        thickness = self.thickness

        rad_X = (
            radius*(self.ellipse_aspect + (MAX_ELLIPSE_ASPECT * ellipse_adjust / 127) ) -
            thickness/2)
        rad_Y = radius - thickness/2

        # FIXME-RENDERING: Processing draw call
        # noFill()

        # loop over segments and draw arcs
        arcs = []
        for i in xrange(self.segs):

            arcs.append(self.draw_segment_with_animation(
                rad_X, rad_Y, i, as_mask, level_scale))
        return arcs

    def draw_segments_with_animations(
        self, rad_X, rad_Y, n_segs, as_mask, level_scale):
        """Vectorized draw of all of the segments."""
        # parameters that animations may modify
        rad_adjust = np.zeros(n_segs, float)
        thickness_adjust = np.zeros(n_segs, float)
        col_center_adjust = np.zeros(n_segs, float)
        col_width_adjust = np.zeros(n_segs, float)
        col_period_adjust = np.zeros(n_segs, float)
        col_sat_adjust = np.zeros(n_segs, float)
        x_adjust = np.zeros(n_segs, int)
        y_adjust = np.zeros(n_segs, int)
        seg_num = np.array(xrange(n_segs))

        # 90 fps
        # the angle of this particular segment
        seg_angle = self.rot_interval*seg_num+self.curr_angle
        rel_angle = self.rot_interval*seg_num

        for anim in self.anims:
            target = anim.target

            # what is this animation targeting?
            if target == 2: # thickness
                    thickness_adjust += anim.get_value(rel_angle)
            elif target == 3: # radius
                    rad_adjust += anim.get_value(rel_angle)
            elif target == 5: # color center
                    col_center_adjust += anim.get_value(0)
            elif target == 6: # color width
                    col_width_adjust += anim.get_value(0)
            elif target == 7: # color periodicity
                    col_period_adjust += anim.get_value(0) / 16
            elif target == 8: # saturation
                    col_sat_adjust += anim.get_value(rel_angle)
            elif target == 11: # x offset
                    x_adjust += anim.get_value(0)*(geometry.x_size/2)/127
            elif target == 12: # y offset
                    y_adjust += anim.get_value(0)*(geometry.y_size/2)/127

        # 45 fps
        # the abs() is there to prevent negative width setting when using multiple animations.
        stroke_weight = abs(self.thickness*(1 + thickness_adjust/127))
        # FIXME-RENDERING: Processing draw call
        # strokeWeight( stroke_weight )

        # now set the color

        # FIXME: negative blacking doesn't work
        # blacking_mode: True for standard, False for inverted
        # constrain min to 1 to avoid divide by zero error
        blacking = max(self.blacking, 1)
        blacking_mode = blacking >= 0

        # if no blacking at all or if this is not a blacked segment, draw it
        # the blacking == -1 is a hack to remove the "all segments blacked" bug
        black_this_segment = seg_num % abs(blacking) != 0
        if (blacking == 0 or blacking == -1 or
            blacking_mode != black_this_segment):

            hue = (
                self.col_center +
                col_center_adjust +
                (
                    (self.col_width+col_width_adjust) *
                    sawtooth(rel_angle*(self.col_spread+col_period_adjust), 0))
                )

            # wrap the hue index
            while hue > 255:
                hue = hue - 255
            while hue < 0:
                hue = hue + 255
        # 24 fps
            # FIXME-COLOR: use of Processing color object
            seg_color = color(hue, self.col_sat + col_sat_adjust, 255)
        # otherwise this is a blacked segment.
        else:
            # FIXME-COLOR
            seg_color = color(0, 0, 0)

        # only draw something if the segment color isn't black.
        # FIXME-COLOR
        # FIXME: this might be bugged
        if color(0, 0, 0) != seg_color:

            # if we're drawing this beam as a mask, make the segment black
            if as_mask:
                # FIXME-RENDERING
                # stroke(0)
                stroke = True
                seg_color = color(0, 0, 0)
                level = 255
            # otherwise pick the color and set the level
            else:
                # FIXME-RENDERING
                # stroke( blendColor(seg_color, color(0,0,level_scale), MULTIPLY) )
                stroke = True
                level = level_scale
        else:
            # FIXME-RENDERING
            # noStroke()
            stroke = False
            level = level_scale

        # 20 fps

        # draw pie wedge for this cell
        # FIXME-RENDERING
        #print "segment"
        return Arc(
            level=level,
            stroke=int(stroke),
            stroke_weight=stroke_weight,
            hue=seg_color.hue,
            sat=seg_color.sat,
            val=seg_color.val,
            x=geometry.x_center + self.x_offset + x_adjust,
            y=geometry.y_center + self.y_offset + y_adjust,
            rad_x=abs(rad_X + rad_adjust),
            rad_y=abs(rad_Y+ rad_adjust),
            start=seg_angle,
            stop=seg_angle + self.rot_interval,)


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

    def set_midi_param(self, is_note, num, val):
        """set a control parameter based on passed midi message"""
        # define the mapping between APC40 and parameters and set values
        unimplemented_animation_waveforms = (28, 29, 30, 31)
        unimplemented_animation_targets = (43, 44, 47)

        if is_note:
            # ipad animation type select
            if num >= 24 and num <= 31:

                # haven't implemented these waveforms yet

                if num not in unimplemented_animation_waveforms:
                    anim = self.get_current_animation()
                    anim.typeI = num
                    anim.update_params()

            # ipad periodicity select
            elif num >= 0 and num <= 15:
                anim = self.get_current_animation()
                anim.n_periodsI = num
                anim.update_params()

            # ipad target select
            elif (
                num >= 35 and
                num <= 47 and
                num not in unimplemented_animation_targets
            ):
                anim = self.get_current_animation()
                anim.targetI = num
                anim.update_params()

            # animation control buttons, for iPad control

            # aniamtion select buttons:
            elif num == 0x57: #anim 0
                self.curr_anim = 0
                set_anim_select_LED(0)
            elif num == 0x58: #anim 1
                self.curr_anim = 1
                set_anim_select_LED(1)
            elif num == 0x59: #anim 2
                self.curr_anim = 2
                set_anim_select_LED(2)
            elif num == 0x5A: #anim 3
                self.curr_anim = 3
                set_anim_select_LED(3)

            # directional controls
            elif num == 0x5E: # up on D-pad
                self.y_offset -= self.y_nudge
            elif num == 0x5F: # down on D-pad
                self.y_offset += self.y_nudge
            elif num == 0x60: # right on D-pad
                self.x_offset += self.x_nudge
            elif num == 0x61: # left on D-pad
                self.x_offset -= self.x_nudge
            elif num == 0x62: # "shift" - beam center
                self.x_offset = 0
                self.y_offset = 0

        else: # this is a control change
            # color parameters: top of lower bank
            if num == 16: # color center
                self.col_centerI = val
            elif num == 17: # color width
                self.col_widthI = val
            elif num == 18: # color spread
                self.col_spreadI = val
            elif num == 19: # saturation
                self.col_satI = val

            # geometry parameters: bottom of lower bank
            elif num == 20: # rotation speed
                self.rot_speedI = val
            elif num == 21: # thickness
                self.thicknessI = val
            elif num == 22: # radius
                self.radiusI = val
            elif num == 23: # ellipse aspect ratio
                self.ellipse_aspectI = val

            # segments parameters: bottom of upper bank

            elif num == 52: # number of segments
                self.segsI = val
            elif num == 53: # blacking
                self.blackingI = val

            # animation parameters: top of upper bank
            # /* fix this code
            elif num == 48:
                anim = self.get_current_animation()
                anim.speedI = val
                anim.update_params()
            elif num == 49:
                anim = self.get_current_animation()
                anim.weightI = val
                anim.update_params()
            elif num == 50:
                anim = self.get_current_animation()
                anim.duty_cycleI = val
                anim.update_params()
            elif num == 51:
                anim = self.get_current_animation()
                anim.smoothingI = val
                anim.update_params()

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
