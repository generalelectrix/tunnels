"""Control device settings and initialization."""
import logging as log
from .midi import ControlChangeMapping

__all__ = ('initialize_device',)

def init_apc40(midi_out):

    knob_off = 0
    knob_single = 1
    knob_volume = 2
    knob_pan = 3

    top_knob_led_ring_settings = [
        (0x38, knob_pan),
        (0x39, knob_volume),
        (0x3A, knob_single),
        (0x3B, knob_single),

        (0x3C, knob_volume),
        (0x3D, knob_pan),
        (0x3E, knob_off),
        (0x3F, knob_off),
    ]

    bot_knob_led_ring_settings = [
        (0x18, knob_single),
        (0x19, knob_single),
        (0x1A, knob_volume),
        (0x1B, knob_volume),
        (0x1C, knob_pan),
        (0x1D, knob_volume),
        (0x1E, knob_volume),
        (0x1F, knob_single),
    ]

    knob_settings = top_knob_led_ring_settings + bot_knob_led_ring_settings

    # put into ableton (full control) mode
    log.debug("Sending sysex mode command.")
    midi_out.port.send_message(
        [0xF0, 0x47, 0x00, 0x73, 0x60, 0x00, 0x04, 0x41, 0x08, 0x04, 0x01, 0xF7])

    log.debug("Setting LED behaviors.")
    for note, val in knob_settings:
        midi_out.send_from_mapping(ControlChangeMapping(0, note), val)

def init_apc20(midi_out):
    log.debug("Sending sysex mode command.")
    # put into ableton (full control) mode
    midi_out.port.send_message(
        [0xF0, 0x47, 0x7F, 0x7B, 0x60, 0x00, 0x04, 0x41, 0x08, 0x02, 0x01, 0xF7])

def initialize_device(midi_out):
    """Perform any device-specific midi initialization."""
    device_name = midi_out.name
    try:
        initializer = {
            "Akai APC40": init_apc40,
            "Akai APC20": init_apc20,
        }[device_name]
    except KeyError:
        log.debug("No initializer found for {}.".format(device_name))
    else:
        log.info("Initializing {}.".format(device_name))
        initializer(midi_out)
