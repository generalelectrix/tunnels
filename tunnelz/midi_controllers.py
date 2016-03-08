from .animation import WaveformType, AnimationTarget
from .beam_matrix_minder import (
    BeamSave, LookSave, Delete, LookEdit,
    ButtonEmpty, ButtonBeam, ButtonLook,
    BeamMatrixMinder,)
from bidict import bidict
from collections import namedtuple
from .midi import NoteOnMapping, NoteOffMapping, ControlChangeMapping


def _build_grid_button_map():
    mapping = {}
    for row in BeamMatrixMinder.n_rows:
        for column in BeamMatrixMinder.n_columns:
            mapping[(row, column)] = NoteOnMapping(column, row + 0x35)
    return bidict(mapping)


class MidiController (object):
    """Base class for midi controllers."""

    def __init__(self):
        self.controls = {}

    def add_controls(self, control_map, callback):
        """Attach a control map to a specified callback.

        Returns the bidirectional version of the control map.
        """
        self.set_callback(control_map.itervalues(), callback)
        return bidict(control_map)

    def set_callback(self, mappings, callback):
        """Manually register a callback for an iterable of mappings."""
        for mapping in mappings:
            self.controls[mapping] = callback


class BeamMatrixMidiController (MidiController):

    # maps named controls to midi messages
    control_map = bidict({
        BeamSave: NoteOnMapping(0, 0x52),
        LookSave: NoteOnMapping(0, 0x53),
        Delete: NoteOnMapping(0, 0x54),
        LookEdit: NoteOnMapping(0, 0x56)
    })

    BeamMatrixLEDState = namedtuple(
    "BeamMatrixLEDState", (BeamSave, LookSave, LookEdit, Delete))

    state_to_led_state_map = dict(
        Idle=BeamMatrixLEDState(0, 0, 0, 0),
        BeamSave=BeamMatrixLEDState(2, 0, 0, 0),
        LookSave=BeamMatrixLEDState(0, 2, 0, 0),
        LookEdit=BeamMatrixLEDState(0, 0, 2, 0),
        Delete=BeamMatrixLEDState(0, 0, 0, 2)
    )

    grid_button_map = _build_grid_button_map()

    button_state_value_map = {
        ButtonEmpty: (0, 1), # off, red
        ButtonLook: (1, 1), # on, red
        ButtonBeam: (1, 2) # on, orange
    }

    def __init__(self, ui, midi_in, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        super(BeamMatrixMidiController, self).__init__()
        self.ui = ui
        ui.controllers.add(self)
        self.midi_out = midi_out

        # the controls which will be registered with the midi service
        self.set_callback(self.grid_button_map.itervalues(), self.handle_grid_button)
        self.set_callback(self.control_map.itervalues(), self.handle_state_button)

        midi_in.register_mappings(self.controls)

    def handle_grid_button(self, mapping, payload):
        row, col = self.grid_button_map.inv[mapping]
        self.ui.grid_button_press(row, col)

    def handle_state_button(self, mapping, payload):
        self.ui.state = self.control_map.inv[mapping]

    def set_beam_matrix_state(self, state):
        """Send UI update commands based on the beam matrix state."""
        led_state = state_to_led_state_map[state]
        message_mappings = tuple(
            (mapping, getattr(led_state, control))
            for control, mapping in self.control_map.iteritems())
        self.midi_out.send_from_mapping(message_mappings)

    def set_button_state(row, column, state):
        control_map = self.grid_button_map[(row, column)]
        status, color = self.button_state_value_map[state]
        if status == 0:
            val = 0
        elif status == 1:
            val = color*2 + 1
        elif state == 2:
            val = (color + 1)*2
        else:
            val = 0

        self.midi_out.send_from_mapping(control_map, val)

class MixerMidiController (MidiController):

    def __init__(self, ui, midi_in, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        super(MixerMidiController, self).__init__()
        self.ui = ui
        ui.controllers.add(self)
        self.midi_out = midi_out

        self.channel_faders = bidict()
        self.bump_buttons = bidict()
        self.mask_buttons = bidict()
        self.track_select = bidict()
        self.look_indicators = bidict()
        # add controls for all mixer channels
        for chan in xrange(ui.mixer.n_layers):
            self.channel_faders[chan] = ControlChangeMapping(chan, 0x7)
            self.bump_button_on[chan] = NoteOnMapping(chan, 0x32)
            self.bump_button_off[chan] = NoteOffMapping(chan, 0x32)
            self.mask_buttons[chan] = NoteOnMapping(chan, 0x31)
            self.track_select[chan] = NoteOnMapping(chan, 0x33)
            self.look_indicators[chan] = NoteOnMapping(chan, 0x30)

        # update the controls
        self.set_callback(self.channel_faders.itervalues(), self.handle_channel_fader)
        self.set_callback(self.bump_button_on.itervalues(), self.handle_bump_button_on)
        self.set_callback(self.bump_button_off.itervalues(), self.handle_bump_button_off)
        self.set_callback(self.mask_buttons.itervalues(), self.handle_mask_button)
        self.set_callback(self.track_select.itervalues(), self.handle_track_select)

        # register input mappings
        midi_in.register_mappings(self.controls)

    def handle_channel_fader(self, mapping, value):
        chan = self.channel_faders[mapping]
        # map midi range to 255
        value = 0 if value == 0 else 2*value + 1
        self.ui.set_level(chan, value)

    def handle_bump_button_on(self, mapping, _):
        chan = self.bump_button_on[mapping]
        self.ui.set_bump_button(chan, True)

    def handle_bump_button_off(self, mapping, _):
        chan = self.bump_button_off[mapping]
        self.ui.set_bump_button(chan, False)

    def handle_mask_button(self, mapping, _):
        chan = self.mask_buttons[mapping]
        self.ui.toggle_mask_state(chan)

    def set_mixer_layer(self, layer):
        """Emit the midi messages to change the selected mixer channel."""
        for chan, mapping in self.track_select.iteritems():
            if chan == layer:
                self.midi_out.send_from_mapping(mapping, 0)
            else:
                self.midi_out.send_from_mapping(mapping, 1)

    def set_level(self, layer, level):
        """Emit midi messages to update layer level."""
        # map level on 255 back into midi range
        level = level if level == 0 else int((level - 1)/2)
        mapping = self.channel_faders[layer]
        self.midi_out.send_from_mapping(mapping, level)

    def set_bump_button(self, layer, state):
        """Emit the midi messages to change the bump button state.

        Args:
            layer: which layer to set button
            state (boolean): on or off
        """
        mapping = self.bump_button_on[layer]
        self.midi_out.send_from_mapping(mapping, int(state))

    def set_mask_button(self, layer, state):
        """Emit the midi messages to change the mask button state."""
        mapping = self.mask_buttons[layer]
        self.midi_out.send_from_mapping(mapping, int(state))

    def set_look_indicator(self, layer, state):
        """Emit the midi messages to change the look indicator state."""
        mapping = self.look_indicators[layer]
        self.midi_out.send_from_mapping(mapping, int(state))


class AnimationMidiController (MidiController):

    def __init__(self, ui, midi_in, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        super(AnimationMidiController, self).__init__()
        self.ui = ui
        ui.controllers.add(self)
        self.midi_out = midi_out
        self.knobs = self.add_controls({
            'speed': ControlChangeMapping(0, 48),
            'weight': ControlChangeMapping(0, 49),
            #'duty_cycle': ControlChangeMapping(0, 50),
            'smoothing': ControlChangeMapping(0, 51),
            },
            self.handle_knob)

        self.type_buttons = self.add_controls({
            WaveformType.Sine: NoteOnMapping(0, 24),
            WaveformType.Triangle: NoteOnMapping(0, 25),
            WaveformType.Square: NoteOnMapping(0, 26),
            WaveformType.Sawtooth: NoteOnMapping(0, 27),
            #WaveformType.Lorentz: NoteOnMapping(0, 28),
            #WaveformType.Strange2: NoteOnMapping(0, 29),
            #WaveformType.Random: NoteOnMapping(0, 30),
            #WaveformType.Happiness: NoteOnMapping(0, 31),
            },
            self.handle_type_button)

        self.periodicity_buttons = self.add_controls(
            {n: NoteOnMapping(0, n) for n in xrange(16)},
            self.handle_periodicity_button)

        # FIXME-NUMERIC TARGETS
        self.target_buttons = self.add_controls(
            {target: NoteOnMapping(0, target+34)for target in AnimationTarget.VALUES},
            self.handle_target_button)

        self.clipboard_buttons = self.add_controls({
            'copy': NoteOnMapping(0, 0x65),
            'paste': NoteOnMapping(0, 0x64)},
            self.handle_clipboard_button)

        self.animation_select_buttons = self.add_controls({
            n: NoteOnMapping(0, 0x57+n) for n in xrange(4)},
            self.handle_animation_select_button)

        # register input mappings
        midi_in.register_mappings(self.controls)

    @staticmethod
    def speed_from_midi(val):
        if val > 65:
            return -float(val - 65)/62
        elif val < 63:
            return -float(val - 63)/63
        else:
            return 0.0

    @staticmethod
    def speed_to_midi(speed):
        if speed < 0.0:
            return min(int(-speed * 62 + 65), 127)
        elif speed > 0.0:
            return max(int(-speed * 63 + 63), 0)
        else:
            return 64

    knob_value_from_midi = {
        'speed': self.speed_from_midi,
        'weight': lambda w: w,
        #'duty_cycle': lambda d: d,
        'smoothing': lambda val: float(val)/127
    }

    knob_value_to_midi = {
        'speed': self.speed_to_midi,
        'weight': lambda w: w,
        #'duty_cycle': lambda d: d,
        'smoothing': lambda s: min(int(s * 127), 127)
    }

    def handle_knob(self, mapping, value):
        knob = self.knobs.inv[mapping]
        self.anim.set_control_value(knob, self.knob_value_from_midi[knob](value))
