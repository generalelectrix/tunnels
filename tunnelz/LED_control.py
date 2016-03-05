"""helper functions for controlling APC and touch OSC LEDs and buttons"""
from .button_LED import *
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
        send_CC(0, knob_num, beam.get_midi_param(is_note=False, num=knob_num))

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
        send_CC(channel, 0x18, 0)
        send_CC(channel, 0x19, 0)
        send_CC(channel, 0x1A, 0)
        send_CC(channel, 0x1B, 0)

        send_CC(channel, 0x1C, 0)
        send_CC(channel, 0x1D, 0)
        send_CC(channel, 0x1E, 0)
        send_CC(channel, 0x1F, 0)

def set_top_LED_rings(beam):
    """Set the top LED ring values.

    beam: the beam in the currently selected layer
    """
    if isinstance(beam, Tunnel):
        send_CC(0, 0x38, 3)
        send_CC(0, 0x39, 2)
        send_CC(0, 0x3A, 1)
        send_CC(0, 0x3B, 1)

        send_CC(0, 0x3C, 2)
        send_CC(0, 0x3D, 3)
        send_CC(0, 0x3E, 0)
        send_CC(0, 0x3F, 0)
    elif isinstance(beam, Look):
        send_CC(0, 0x38, 0)
        send_CC(0, 0x39, 0)
        send_CC(0, 0x3A, 0)
        send_CC(0, 0x3B, 0)

        send_CC(0, 0x3C, 0)
        send_CC(0, 0x3D, 0)
        send_CC(0, 0x3E, 0)
        send_CC(0, 0x3F, 0)

