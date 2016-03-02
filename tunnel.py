# scale overall radius, set > 1.0 to enable larger shapes than screen size
from .constants import MAX_RADIUS, MAX_X_OFFSET, MAX_Y_OFFSET, X_CENTER, Y_CENTER
from copy import deepcopy
from .waveforms import sawtooth

MAX_RAD_MULT = 2.0
MAX_ELLIPSE_ASPECT = 2.0

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

        if (is1080):
            self.x_nudge = self.y_nudge = 15 if is1080 else 10
            self.thicknessScale = 4.05 if is1080 else 2.7

        self.anims = [Animation() for _ in xrange(self.n_anim)]
        self.curr_anim = 0;

        self.update_params()

    def update_params(self):
        """Called whenever a beam parameter is changed, by midi for example.

        This is where parameter scaling occurs.
        """
        # update internal parameters from integer values

        rot_speedI = self.rot_speedI
        if 65 < rot_speedI:
            self.rot_speed = float((rot_speedI-65)/self.rot_speed_scale)
        elif 63 > rot_speedI:
            self.rot_speed = -float((-rot_speedI+63)/self.rot_speed_scale)
        else:
            self.rot_speed = 0.0


        self.thickness = float(self.thicknessI*self.thickness_scale)

        # in pixels, I think
        self.radius = int(MAX_RAD_MULT * MAX_RADIUS * self.radiusI / 127)
        self.ellipse_aspect = float(MAX_ELLIPSE_ASPECT * self.ellipse_aspectI / 127.)

        self.col_center = self.col_centerI * 2
        self.col_width = self.col_widthI
        self.col_spread = self.col_spreadI / 8
        self.col_sat = (127 - colSatI) * 2 # we have a "desaturate" knob, not a saturate knob.

        # THIS IS A HACK.  This only works because the APC40 doesn't put out 0 for the bottom of the knob.
        segs = self.segs = self.segsI
        self.rot_interval = 2*pi / segs # ALSO A HACK, same reason

        self.blacking = (self.blackingI - 64) / self.blacking_scale

        # ensure we don't offset beyond the maximum
        self.x_offset = min(max(self.x_offset, -MAX_X_OFFSET), MAX_X_OFFSET)
        self.y_offset = min(max(self.y_offset, -MAX_Y_OFFSET), MAX_Y_OFFSET)

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


        # calulcate the rotation, wrap to -pi to +pi
        self.curr_angle = unwrap(
            self.curr_angle + self.rot_speed + rot_adjust/self.rot_speed_scale)

        raius = self.radius
        thickness = self.thickness

        rad_X = (
            radius*(self.ellipse_aspect + (MAX_ELLIPSE_ASPECT * ellipse_adjust / 127) ) -
            thickness/2)
        rad_Y = radius - thickness/2

        # FIXME-RENDERING: Processing draw call
        noFill()

        # loop over segments and draw arcs
        for i in xrange(self.segs):

            self.draw_segment_with_animation(
                rad_X, rad_Y, i, as_mask, level_scale)

    def draw_segment_with_animation(
            self, rad_X, rad_Y, seg_num, as_mask, level_scale):
        """actually draws a tunnel segment given animation parameters

        Args:
            float rad_X, float rad_Y, int seg_num, boolean as_mask, int level
        """
        # parameters that animations may modify
        rad_adjust = 0.
        thickness_adjust = 0.
        col_center_adjust = 0.
        col_width_adjust = 0.
        col_period_adjust = 0.
        col_sat_adjust = 0.
        x_adjust = 0
        y_adjust = 0

        # the angle of this particular segment
        seg_angle = self.rot_interval*seg_num+self.curr_angle;
        rel_angle = self.rot_interval*seg_num;

        for anim in self.anims:
            target = anim.target

            # what is this animation targeting?
            if target == 2: # thickness
                    thickness_adjust += anim.get_value(rel_angle);
            elif target == 3: # radius
                    rad_adjust += anim.get_value(rel_angle);
            elif target == 5: # color center
                    col_center_adjust += anim.get_value(0);
            elif target == 6: # color width
                    col_width_adjust += anim.get_value(0);
            elif target == 7: # color periodicity
                    col_period_adjust += anim.get_value(0) / 16;
            elif target == 8: # saturation
                    col_sat_adjust += anim.get_value(rel_angle);
            elif target == 11: # x offset
                    x_adjust += anim.get_value(0)*(width/2)/127;
            elif target == 12: # y offset
                    y_adjust += anim.get_value(0)*(height/2)/127;

        # FIXME-RENDERING: Processing draw call
        strokeWeight( abs(self.thickness*(1 + thickness_adjust/127)) )
        # the abs() is there to prevent negative width setting when using multiple animations.

        # now set the color

        # blacking_mode: True for standard, False for inverted
        blacking = self.blacking
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

            # FIXME-COLOR: use of Processing color object
            seg_color = color(hue, self.col_sat + col_sat_adjust, 255)
        # otherwise this is a blacked segment.
        else:
            # FIXME-COLOR
            seg_color = color(0)

        # only draw something if the segment color isn't black.
        # FIXME-COLOR
        if color(0) != seg_color:

            # if we're drawing this beam as a mask, make the segment black
            if as_mask:
                # FIXME-RENDERING
                stroke(0)
            # otherwise pick the color and set the level
            else:
                # FIXME-RENDERING
                stroke( blendColor(seg_color, color(0,0,level), MULTIPLY) )
        else:
            # FIXME-RENDERING
            noStroke()

        # draw pie wedge for this cell
        # FIXME-RENDERING
        arc(
            X_CENTER + self.x_offset + x_adjust,
            Y_CENTER+ self.y_offset + y_adjust,
            abs(rad_X + rad_adjust),
            abs(rad_Y+ rad_adjust),
            seg_angle,
            seg_angle + rot_interval,)


    # function to set the control parameter based on passed midi value
    void setMIDIParam(boolean isNote, int num, int val) {

        Animation anim;
        # define the mapping between APC40 and parameters and set values
        if (isNote) {

            # ipad animation type select
            if (num >= 24 && num <= 31) {

                # haven't implemented these waveforms yet
                if (num != 28 && num != 29 && num != 30 && num != 31) {
                    anim = getAnimation(currAnim);
                    anim.typeI = num;
                    anim.updateParams();
                }
            }

            # ipad periodicity select
            else if (num >= 0 && num <= 15) {
                anim = getAnimation(currAnim);
                anim.nPeriodsI = num;
                anim.updateParams();
            }

            # ipad target select
            else if (num >= 35 && num <= 47) {
                if (num != 47 && num != 44 && num != 43) {
                    anim = getAnimation(currAnim);
                    anim.targetI = num;
                    anim.updateParams();
                }
            }

            switch(num) {
                # animation control buttons, for iPad control



                # aniamtion select buttons:
                case 0x57: #anim 0
                    currAnim = 0;
                    setAnimSelectLED(0);
                    break;
                case 0x58: #anim 1
                    currAnim = 1;
                    setAnimSelectLED(1);
                    break;
                case 0x59: #anim 2
                    currAnim = 2;
                    setAnimSelectLED(2);
                    break;
                case 0x5A: #anim 3
                    currAnim = 3;
                    setAnimSelectLED(3);
                    break;

                # directional controls
                case 0x5E: # up on D-pad
                    yOffset -= yNudge;
                    break;
                case 0x5F: # down on D-pad
                    yOffset += yNudge;
                    break;
                case 0x60: # right on D-pad
                    xOffset += xNudge;
                    break;
                case 0x61: # left on D-pad
                    xOffset -= xNudge;
                    break;
                case 0x62: # "shift" - beam center
                    xOffset = 0;
                    yOffset = 0;
                    break;
            } # end of note num switch

        }

        else { # this is a control change
            switch(num) {
                # color parameters: top of lower bank
                case 16: # color center
                    colCenterI = val;
                    break;
                case 17: # color width
                    colWidthI = val;
                    break;
                case 18: # color spread
                    colSpreadI = val;
                    break;
                case 19: # saturation
                    colSatI = val;
                    break;

                # geometry parameters: bottom of lower bank
                case 20: # rotation speed
                    rotSpeedI = val;
                    break;
                case 21: # thickness
                    thicknessI = val;
                    break;
                case 22: # radius
                    radiusI = val;
                    break;
                case 23: # ellipse aspect ratio
                    ellipseAspectI = val;
                    break;

                # segments parameters: bottom of upper bank

                case 52: # number of segments
                    segsI = val;
                    break;
                case 53: # blacking
                    blackingI = val;
                    break;

                # animation parameters: top of upper bank
                # /* fix this code
                case 48:
                    anim = getAnimation(currAnim);
                    anim.speedI = val;
                    anim.updateParams();
                    break;
                case 49:
                    anim = getAnimation(currAnim);
                    anim.weightI = val;
                    anim.updateParams();
                    break;
                case 50:
                    anim = getAnimation(currAnim);
                    anim.dutyCycleI = val;
                    anim.updateParams();
                    break;
                case 51:
                    anim = getAnimation(currAnim);
                    anim.smoothingI = val;
                    anim.updateParams();
                    break;
                # */

            } # end of switch

        }

    } # end up update midi param method

    # method to get the midi-scaled value for a control parameter
    int getMIDIParam(boolean isNote, int num) {
        int theVal = 0;

        Animation anim;

        if (!isNote) {
            switch(num) {
                case 16: # color center
                    theVal = colCenterI;
                    break;
                case 17: # color width
                    theVal = colWidthI;
                    break;
                case 18: # color spread
                    theVal = colSpreadI;
                    break;
                case 19: # saturation
                    theVal = colSatI;
                    break;

                # geometry parameters: bottom of lower bank
                case 20: # rotation speed
                    theVal = rotSpeedI;
                    break;
                case 21: # thickness
                    theVal = thicknessI;
                    break;
                case 22: # radius
                    theVal = radiusI;
                    break;
                case 23: # ellipse aspect ratio
                    theVal = ellipseAspectI;
                    break;

                # segments parameters: bottom of upper bank

                case 52: # number of segments
                    theVal = segsI;
                    break;
                case 53: # blacking
                    theVal = blackingI;
                    break;

                # animation parameters: top of upper bank
                # /* fix this code
                case 48:
                    anim = getAnimation(currAnim);
                    theVal = anim.speedI;
                    break;
                case 49:
                    anim = getAnimation(currAnim);
                    theVal = anim.weightI;
                    break;
                case 50:
                    anim = getAnimation(currAnim);
                    theVal = anim.dutyCycleI;
                    break;
                case 51:
                    anim = getAnimation(currAnim);
                    theVal = anim.smoothingI;
                    break;
            }
        }

        return theVal;
    } # end of getMIDIParam method


    String toString() {
        return type + "\t" +
                     rotSpeedI + "\t" +
                     thicknessI + "\t" +
                     radiusI + "\t" +
                     ellipseAspectI + "\t" +
                     colCenterI + "\t" +
                     colWidthI + "\t" +
                     colSpreadI + "\t" +
                     colSatI + "\t" +
                     segsI + "\t" +
                     blackingI + "\t" +
                     curr_angle + "\t" +
                     xOffset + "\t" +
                     yOffset + "\t" +
                     currAnim  + "\t" +
                     theAnims[0].toString() + "\t" +
                     theAnims[1].toString() + "\t" +
                     theAnims[2].toString() + "\t" +
                     theAnims[3].toString();
    }

    void setupPixelParams() {

    }

} # end of Tunnel class
