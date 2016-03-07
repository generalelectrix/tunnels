from .midi import midi_out

def set_anim_select_LED(which_anim):
    """adjust the animation LED state based on selected animation

    which_anim: index of currently selected animation
    """
    button_offset = 0x57

    for i in xrange(4):
        if which_anim == i:
            midi_out.send_note(0, button_offset + i, 1)
        else:
            midi_out.send_note(0, button_offset + i, 0)

def _set_LED_in_range(button_number, button_offset, n_buttons):
    """Set one LED in a button range on, all others off."""
    for n in xrange(button_offset, n_buttons+button_offset):
        if n == button_number:
            midi_out.send_note(0, n, 1)
        else:
            midi_out.send_note(0, n, 0)

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
    midi_out.send_note(column, 0x35+row, val)

def set_scene_launch_LED(row, state):
    """Set the state of an APC40 scene launch LED

    Args:
        row: the row of the target scene launch LED
        state (int): 0=off, 1=on, 2=blink
    """
    midi_out.send_note(0, 0x52 + row, state)

def set_beam_save_LED(state):
    set_scene_launch_LED(0, state)

def set_look_save_LED(state):
    set_scene_launch_LED(1, state)

def set_delete_LED(state):
    set_scene_launch_LED(2, state)

def set_look_edit_LED(state):
    set_scene_launch_LED(4, state)

def set_track_select_LED(channel, state):
    midi_out.send_note(channel, 0x33, state)

def set_track_select_LED_radio(channel):
    for chan in xrange(8):
        if chan == channel:
            set_track_select_LED(chan, 1)
        else:
            set_track_select_LED(chan, 0)

def _set_bool_button_LED(channel, CC, state):
    if state:
        midi_out.send_note(channel, CC, 1)
    else:
        midi_out.send_note(channel, CC, 0)

def set_bump_button_LED(channel, state):
    _set_bool_button_LED(channel, 0x32, state)

def set_mask_button_LED(channel, state):
    _set_bool_button_LED(channel, 0x31, state)

def set_is_look_LED(channel, state):
    _set_bool_button_LED(channel, 0x30, state)