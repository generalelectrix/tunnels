"""helper functions for controlling APC LEDs"""
from .look import Look

KNOB_NUMS = [
    16, 17, 18, 19,
    20, 21, 22, 23,
    48, 49, 50, 51,
    52, 53, 54, 55,]

def update_knob_state(layer, beam):
    """update the state of the control knobs when we change channels

    layer: index of the currently selected layer
    beam: the instance of the beam in that layer
    """
    for knob_num in knob_nums:
        send_CC(0, knob_num, beam.get_MIDI_param(false, knob_num))

    print "set anim LEDs"

    if isinstance(beam, Look):
        set_is_look_LED(layer, True)
        set_anim_type_LED(-1)
        set_anim_periods_LED(-1)
        set_anim_target_LED(-1)
    else:
        anim = beam.get_current_animation()
        set_is_look_LED(layer, False)
        set_anim_type_LED(anim.typeI)
        set_anim_periods_LED(anim.n_periodsI)
        set_anim_target_LED(anim.targetI)

def set_bottom_LED_rings(channel, beam):
    """update the state of the bottom LED ring values

    channel: the currently selected channel
    beam: the beam in the currently selected layer
    """
    if isinstance(beam, Tunnel):
        send_CC(channel, 0x18, 1)
        send_CC(channel, 0x19, 1)
        send_CC(channel, 0x1A, 2)
        send_CC(channel, 0x1B, 2)

        send_CC(channel, 0x1C, 3)
        send_CC(channel, 0x1D, 2)
        send_CC(channel, 0x1E, 2)
        send_CC(channel, 0x1F, 1)
    elif isinstance(beam, Look):
        send_CC(channel, 0x18, 0);
        send_CC(channel, 0x19, 0);
        send_CC(channel, 0x1A, 0);
        send_CC(channel, 0x1B, 0);

        send_CC(channel, 0x1C, 0)
        send_CC(channel, 0x1D, 0)
        send_CC(channel, 0x1E, 0)
        send_CC(channel, 0x1F, 0)

def set_top_LED_rings(beam):
    """Set the top LED ring values.

    beam: the beam in the currently selected layer
    """
    if isinstance(beam, Tunnel):
        send_CC(0, 0x38, 3);
        send_CC(0, 0x39, 2);
        send_CC(0, 0x3A, 1);
        send_CC(0, 0x3B, 1);

        send_CC(0, 0x3C, 2);
        send_CC(0, 0x3D, 3);
        send_CC(0, 0x3E, 0);
        send_CC(0, 0x3F, 0);
    elif isinstance(beam, Look):
        send_CC(0, 0x38, 0);
        send_CC(0, 0x39, 0);
        send_CC(0, 0x3A, 0);
        send_CC(0, 0x3B, 0);

        send_CC(0, 0x3C, 0);
        send_CC(0, 0x3D, 0);
        send_CC(0, 0x3E, 0);
        send_CC(0, 0x3F, 0);

def set_anim_select_LED(which_anim):
    """adjust the animation LED state based on selected animation

    which_anim: index of currently selected animation
    """
    button_offset = 0x57

    for i in xrange(4):
        if which_anim == i:
            send_note(0, button_offset + i, 1)
        else:
            send_note(0, button_offset + i, 0)


void setAnimTypeLED(int whichType) {

    int buttonOffset = 24;

    for (int i = 0; i < 8; i++) {
        if (whichType == i + buttonOffset) {
            sendNote(0, buttonOffset + i, 1);
        }
        else {
            sendNote(0, buttonOffset + i, 0);
        }
    }

}

void setAnimPeriodsLED(int whichType) {

    int buttonOffset = 0;

    for (int i = 0; i < 16; i++) {
        if (whichType == i + buttonOffset) {
            sendNote(0, buttonOffset + i, 1);
        }
        else {
            sendNote(0, buttonOffset + i, 0);
        }
    }

}

void setAnimTargetLED(int whichType) {

    int buttonOffset = 35;

    println("setting anim LED " + whichType);

    for (int i = 0; i < 13; i++) {
        if (whichType == i + buttonOffset) {
            sendNote(0, buttonOffset + i, 1);
            println("setting the LED!");
        }
        else {
            sendNote(0, buttonOffset + i, 0);
        }
    }

}

// method to set the color state of a clip launch LED
// state is off=0, on=1, blink=2
// col is green=0, red=1, yellow=2
void setClipLaunchLED(int row, int column, int state, int col) {

    int val;

    if (0 == state) {
        val = 0;
    }
    else if (1 == state) {
        val = col*2 + 1;
    }
    else if (2 ==  state) {
        val = (col+1)*2;
    }
    else {
        val = 0;
    }

    // column is midi channel, row is note plus offset of 0x35
    sendNote(column, 0x35+row, val);

}

// method to set scene launch LED
// 0=off, 1=on, 2=blink
void setSceneLaunchLED(int row, int state) {
    sendNote(0, 0x52 + row, state);
}

void setBeamSaveLED(int state) {
    setSceneLaunchLED(0, state);
}

void setLookSaveLED(int state) {
    setSceneLaunchLED(1, state);
}

void setDeleteLED(int state) {
    setSceneLaunchLED(2, state);
}

void setLookEditLED(int state) {
    setSceneLaunchLED(4, state);
}

void setTrackSelectLED(int channel, int state) {
    sendNote(channel, 0x33, state);
}

void setTrackSelectLEDRadio(int channel) {
    for (int i=0; i<8; i++) {

        if (i == channel) {
            setTrackSelectLED(i,1);
        }
        else{
            setTrackSelectLED(i,0);
        }

    }

}

void setBumpButtonLED(int channel, boolean state) {
    if (state) {
        sendNote(channel, 0x32, 1);
    }
    else {
        sendNote(channel, 0x32, 0);
    }
}

void setMaskButtonLED(int channel, boolean state) {
    if (state) {
        sendNote(channel, 0x31, 1);
    }
    else {
        sendNote(channel, 0x31, 0);
    }
}

void setIsLookLED(int channel, boolean state) {
    if (state) {
        sendNote(channel, 0x30, 1);
    }
    else {
        sendNote(channel, 0x30, 0);
    }

}

