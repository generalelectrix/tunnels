from .beam_matrix_minder import (
    Idle, BeamSave, LookSave, Delete, LookEdit)
from bidict import bidict
from collections import namedtuple
from .midi import NoteOnMapping, NoteOffMapping, ControlChangeMapping



BeamMatrixLEDState = namedtuple(
    "BeamMatrixLEDState", ('beam_save', 'look_save', 'look_edit', 'delete'))

class BeamMatrixMidiController (object):

    def __init__(self, ui, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        self.ui = ui
        ui.controllers.add(self)
        self.midi_out = midi_out

    # maps named controls to midi messages
    control_map = bidict({
        'beam_save': NoteOnMapping(0, 0x52),
        'look_save': NoteOnMapping(0, 0x53),
        'delete': NoteOnMapping(0, 0x54),
        'look_edit': NoteOnMapping(0, 0x56)
    })

    state_to_led_state_map = dict(
        Idle=BeamMatrixLEDState(0, 0, 0, 0),
        BeamSave=BeamMatrixLEDState(2, 0, 0, 0),
        LookSave=BeamMatrixLEDState(0, 2, 0, 0),
        LookEdit=BeamMatrixLEDState(0, 0, 2, 0),
        Delete=BeamMatrixLEDState(0, 0, 0, 2)
    )

    def set_beam_matrix_state(self, state):
        """Send UI update commands based on the beam matrix state."""
        led_state = state_to_led_state_map[state]
        message_mappings = tuple(
            (mapping, getattr(led_state, control))
            for control, mapping in self.control_map.iteritems())
        self.midi_out.send_from_mapping(message_mappings)

class MixerMidiController (object):

    def __init__(self, ui, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        self.ui = ui
        ui.controllers.add(self)
        self.midi_out = midi_out

    def set_mixer_layer(self, layer):
        pass