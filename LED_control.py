"""helper functions for controlling APC and touch OSC LEDs and buttons"""
from .look import Look
from .midi import send_CC, send_note
from .tunnel import Tunnel

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
    for knob_num in KNOB_NUMS:
        send_CC(0, knob_num, beam.get_MIDI_param(is_note=False, num=knob_num))

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

def _set_LED_in_range(button_number, button_offset, n_buttons):
    """Set one LED in a button range on, all others off."""
    for n in xrange(buttom_offset, n_buttons+button_offset):
        if n == button_number:
            send_note(0, n, 1)
        else:
            send_note(0, n, 0)

def set_anim_type_LED(which_type):
    _set_LED_in_range(which_type, 24, 8)

def set_anim_periods_LED(which_type):
    _set_LED_in_range(which_type, 0, 16)

def set_anim_target_LED(which_type):
    _set_LED_in_range(which_type, 35, 13)

def set_clip_launch_LED(row, column, state, color):
    """Set the color state of an APC40 clip launch LED

    Args:
        row, column: the indices of the clip launch LED
        state (int): off=0, on=1, blink=2
        color (int): green=0, red=1, yellow=2
    """
    if state == 0:
        val = 0
    elif state == 1:
        val = color*2 + 1
    elif state == 2:
        val = (color + 1)*2
    else:
        val = 0

    # column is midi channel, row is note plus offset of 0x35
    send_note(column, 0x35+row, val);

def set_scene_launch_LED(row, state):
    """Set the state of an APC40 scene launch LED

    Args:
        row: the row of the target scene launch LED
        state (int): 0=off, 1=on, 2=blink
    """
    send_note(0, 0x52 + row, state)

def set_beam_save_LED(state):
    set_scene_launch_LED(0, state)

def set_look_save_LED(state):
    set_scene_launch_LED(1, state)

def set_delete_LED(state):
    set_scene_launch_LED(2, state)

def set_look_edit_LED(state):
    set_scene_launch_LED(4, state)

def set_track_select_LED(channel, state):
    send_note(channel, 0x33, state)

def set_track_select_LED_radio(channel):
    for chan in xrange(8):
        if chan == channel:
            set_track_select_LED(chan, 1)
        else:
            set_track_select_LED(chan, 0)

def _set_bool_button_LED(channel, CC, state):
    if state:
        send_note(channel, CC, 1)
    else:
        send_note(channel, CC, 0)

def set_bump_buttom_LED(channel, state):
    _set_bool_button_LED(channel, 0x32, state)

def set_mask_buttom_LED(channel, state):
    _set_bool_button_LED(channel, 0x31, state)

def set_is_look_LED(channel, state):
    _set_bool_button_LED(channel, 0x30, state)

