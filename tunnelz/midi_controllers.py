from .animation import WaveformType, AnimationTarget
from .beam_matrix_minder import (
    Idle, BeamSave, LookSave, Delete, LookEdit,
    ButtonEmpty, ButtonBeam, ButtonLook,
    BeamMatrixMinder,)
from bidict import bidict
from collections import namedtuple
from .midi import NoteOnMapping, NoteOffMapping, ControlChangeMapping


def _build_grid_button_map():
    mapping = {}
    for row in xrange(BeamMatrixMinder.n_rows):
        for column in xrange(BeamMatrixMinder.n_columns):
            mapping[(row, column)] = NoteOnMapping(column, row + 0x35)
    return bidict(mapping)


class MidiController (object):
    """Base class for midi controllers."""

    def __init__(self, midi_in, midi_out):
        self.controls = {}
        self.midi_in = midi_in
        self.midi_out = midi_out

    def add_controls(self, control_map, callback):
        """Attach a control map to a specified callback.

        Returns the bidirectional version of the control map.
        """
        self.set_callback_for_mappings(control_map.itervalues(), callback)
        return bidict(control_map)

    def set_callback(self, mapping, callback):
        """Register a callback for a single mapping."""
        self.controls[mapping] = callback

    def set_callback_for_mappings(self, mappings, callback):
        """Manually register a callback for an iterable of mappings."""
        for mapping in mappings:
            self.controls[mapping] = callback

    def _set_radio_button(self, set_value, control_map):
        """Set only one out of a set of controls on."""
        for value, mapping in control_map.iteritems():
            self.midi_out.send_from_mapping(mapping, int(value == set_value))

    def register_callbacks(self):
        """Register the control mapping callbacks with the midi input service."""
        self.midi_in.register_mappings(self.controls)

    # --- helper functions for useful knobs ---

    @staticmethod
    def bipolar_from_midi(val):
        """MIDI knob to a bipolar float, with detent."""
        if val > 65:
            return -float(val - 65)/62
        elif val < 63:
            return -float(val - 63)/63
        else:
            return 0.0

    @staticmethod
    def bipolar_to_midi(val):
        """Bipolar float to a midi knob."""
        if val < 0.0:
            return min(int(-val * 62 + 65), 127)
        elif val > 0.0:
            return max(int(-val * 63 + 63), 0)
        else:
            return 64

    @staticmethod
    def unipolar_from_midi(val):
        return val / 127.0

    @staticmethod
    def unipolar_to_midi(val):
        return int(val * 127)


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

    state_to_led_state_map = {
        Idle: BeamMatrixLEDState(0, 0, 0, 0),
        BeamSave: BeamMatrixLEDState(2, 0, 0, 0),
        LookSave: BeamMatrixLEDState(0, 2, 0, 0),
        LookEdit: BeamMatrixLEDState(0, 0, 2, 0),
        Delete: BeamMatrixLEDState(0, 0, 0, 2),
    }

    grid_button_map = _build_grid_button_map()

    button_state_value_map = {
        ButtonEmpty: (0, 1), # off, red
        ButtonLook: (1, 1), # on, red
        ButtonBeam: (1, 2) # on, orange
    }

    def __init__(self, ui, midi_in, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        super(BeamMatrixMidiController, self).__init__(midi_in, midi_out)
        self.ui = ui
        ui.controllers.add(self)

        # the controls which will be registered with the midi service
        self.set_callback_for_mappings(
            self.grid_button_map.itervalues(), self.handle_grid_button)
        self.set_callback_for_mappings(
            self.control_map.itervalues(), self.handle_state_button)

        self.register_callbacks()

    def handle_grid_button(self, mapping, payload):
        row, col = self.grid_button_map.inv[mapping]
        self.ui.grid_button_press(row, col)

    def handle_state_button(self, mapping, payload):
        self.ui.state = self.control_map.inv[mapping]

    def set_beam_matrix_state(self, state):
        """Send UI update commands based on the beam matrix state."""
        led_state = self.state_to_led_state_map[state]
        message_mappings = tuple(
            (mapping, getattr(led_state, control))
            for control, mapping in self.control_map.iteritems())
        self.midi_out.send_from_mappings(message_mappings)

    def set_button_state(self, row, column, state):
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

class MetaControlMidiController (MidiController):

    def __init__(self, ui, midi_in, midi_out):
        super(MetaControlMidiController, self).__init__(midi_in, midi_out)
        self.ui = ui
        ui.controllers.add(self)

        self.track_select = self.add_controls(
            {chan: NoteOnMapping(chan, 0x33) for chan in xrange(ui.mixer_ui.mixer.n_layers)},
            self.handle_current_layer)

        # TODO: DRY out number of animators
        self.animation_select_buttons = self.add_controls({
            n: NoteOnMapping(0, 0x57+n) for n in xrange(4)},
            self.handle_current_animator)

        self.set_callback(NoteOnMapping(0, 0x65), self.handle_animation_copy)
        self.set_callback(NoteOnMapping(0, 0x64), self.handle_animation_paste)

        self.register_callbacks()

    def handle_current_layer(self, mapping, _):
        chan = mapping[0]
        self.ui.set_current_layer(chan)

    def set_current_layer(self, layer):
        """Emit the midi messages to change the selected mixer channel."""
        self._set_radio_button(layer, self.track_select)

    def handle_current_animator(self, mapping, _):
        n = self.animation_select_buttons.inv[mapping]
        self.ui.set_current_animator(n)

    def set_current_animator(self, anim_num):
        self._set_radio_button(anim_num, self.animation_select_buttons)

    def handle_animation_copy(self, _, val):
        self.ui.animation_copy()

    def handle_animation_paste(self, _, val):
        self.ui.animation_paste()

class MixerMidiController (MidiController):

    def __init__(self, ui, midi_in, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        super(MixerMidiController, self).__init__(midi_in, midi_out)
        self.ui = ui
        ui.controllers.add(self)

        self.channel_faders = bidict()
        self.bump_button_on = bidict()
        self.bump_button_off = bidict()
        self.mask_buttons = bidict()
        self.look_indicators = bidict()
        # add controls for all mixer channels
        for chan in xrange(ui.mixer.n_layers):
            self.channel_faders[chan] = ControlChangeMapping(chan, 0x7)
            self.bump_button_on[chan] = NoteOnMapping(chan, 0x32)
            self.bump_button_off[chan] = NoteOffMapping(chan, 0x32)
            self.mask_buttons[chan] = NoteOnMapping(chan, 0x31)
            self.look_indicators[chan] = NoteOnMapping(chan, 0x30)

        # update the controls
        self.set_callback_for_mappings(
            self.channel_faders.itervalues(), self.handle_channel_fader)
        self.set_callback_for_mappings(
            self.bump_button_on.itervalues(), self.handle_bump_button_on)
        self.set_callback_for_mappings(
            self.bump_button_off.itervalues(), self.handle_bump_button_off)
        self.set_callback_for_mappings(
            self.mask_buttons.itervalues(), self.handle_mask_button)

        # register input mappings
        self.register_callbacks()

    def handle_channel_fader(self, mapping, value):
        chan = self.channel_faders.inv[mapping]
        # map midi range to 255
        value = 0 if value == 0 else 2*value + 1
        self.ui.set_level(chan, value)

    def handle_bump_button_on(self, mapping, _):
        chan = self.bump_button_on.inv[mapping]
        self.ui.set_bump_button(chan, True)

    def handle_bump_button_off(self, mapping, _):
        chan = self.bump_button_off.inv[mapping]
        self.ui.set_bump_button(chan, False)

    def handle_mask_button(self, mapping, _):
        chan = self.mask_buttons.inv[mapping]
        self.ui.toggle_mask_state(chan)

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


class TunnelMidiController (MidiController):

    def __init__(self, ui, midi_in, midi_out):
        super(TunnelMidiController, self).__init__(midi_in, midi_out)
        self.ui = ui
        ui.controllers.add(self)

        self.unipolar_knobs = self.add_controls({
            'thickness': ControlChangeMapping(0, 21),
            'radius': ControlChangeMapping(0, 22),
            'col_center': ControlChangeMapping(0, 16),
            'col_width': ControlChangeMapping(0, 17),
            'col_spread': ControlChangeMapping(0, 18),
            'col_sat': ControlChangeMapping(0, 19),},
            self.handle_unipolar_knob)

        self.bipolar_knobs = self.add_controls({
            'rot_speed': ControlChangeMapping(0, 20),
            'ellipse_aspect': ControlChangeMapping(0, 23),},
            self.handle_bipolar_knob)

        self.segs_mapping = ControlChangeMapping(0, 52)
        self.blacking_mapping = ControlChangeMapping(0, 53)

        self.set_callback(self.segs_mapping, self.handle_segs)
        self.set_callback(self.blacking_mapping, self.handle_blacking)

        self.nudge_x_pos_mapping = NoteOnMapping(0, 0x60)
        self.nudge_x_neg_mapping = NoteOnMapping(0, 0x61)
        self.nudge_y_pos_mapping = NoteOnMapping(0, 0x5F)
        self.nudge_y_neg_mapping = NoteOnMapping(0, 0x5E)
        self.position_reset_mapping = NoteOnMapping(0, 0x62)

        self.set_callback(self.nudge_x_pos_mapping, self.handle_nudge_x_pos)
        self.set_callback(self.nudge_x_neg_mapping, self.handle_nudge_x_neg)
        self.set_callback(self.nudge_y_pos_mapping, self.handle_nudge_y_pos)
        self.set_callback(self.nudge_y_neg_mapping, self.handle_nudge_y_neg)
        self.set_callback(self.position_reset_mapping, self.handle_reset_beam_position)

        self.register_callbacks()

    def handle_unipolar_knob(self, mapping, val):
        knob = self.unipolar_knobs.inv[mapping]
        setattr(self.ui, knob, self.unipolar_from_midi(val))

    def set_unipolar(self, val, knob):
        mapping = self.unipolar_knobs[knob]
        self.midi_out.send_from_mapping(mapping, self.unipolar_to_midi(val))

    def handle_bipolar_knob(self, mapping, val):
        knob = self.bipolar_knobs.inv[mapping]
        setattr(self.ui, knob, self.bipolar_from_midi(val))

    def set_bipolar(self, val, knob):
        mapping = self.bipolar_knobs[knob]
        self.midi_out.send_from_mapping(mapping, self.bipolar_to_midi(val))

    def handle_segs(self, _, val):
        """Convert midi to number of segments."""
        self.ui.segments = (val + 1)

    def set_segs(self, segs):
        """Convert number of segs back to midi."""
        self.midi_out.send_from_mapping(self.segs_mapping, segs - 1)

    def handle_blacking(self, _, val):
        """Convert midi to blacking.

        Blacking is a bipolar knob on the range [-16, 16].
        """
        self.ui.blacking = int((2*(val / 127.0) - 1) * 16)

    def set_blacking(self, blacking):
        """Convert blacking back to midi.

        Blacking is a bipolar knob on the range [-16, 16].
        """
        midi_blacking = int(127*((blacking / 16.0) + 1) / 2)
        self.midi_out.send_from_mapping(self.blacking_mapping, midi_blacking)

    def handle_nudge_x_pos(self, _, val):
        self.ui.nudge_x_pos()

    def handle_nudge_x_neg(self, _, val):
        self.ui.nudge_x_neg()

    def handle_nudge_y_pos(self, _, val):
        self.ui.nudge_y_pos()

    def handle_nudge_y_neg(self, _, val):
        self.ui.nudge_y_neg()

    def handle_reset_beam_position(self, _, val):
        self.ui.reset_beam_position()

class AnimationMidiController (MidiController):

    def __init__(self, ui, midi_in, midi_out):
        """Fire up a fresh controller and register it with the UI."""
        super(AnimationMidiController, self).__init__(midi_in, midi_out)
        self.ui = ui
        ui.controllers.add(self)

        self.knobs = self.add_controls({
            'speed': ControlChangeMapping(0, 48),
            'weight': ControlChangeMapping(0, 49),
            #'duty_cycle': ControlChangeMapping(0, 50),
            'smoothing': ControlChangeMapping(0, 51),
            },
            self.handle_knob)

        self.knob_value_from_midi = {
            'speed': self.bipolar_from_midi,
            'weight': lambda w: w,
            #'duty_cycle': lambda d: d,
            'smoothing': lambda val: float(val)/127
        }

        self.knob_value_to_midi = {
            'speed': self.bipolar_to_midi,
            'weight': lambda w: w,
            #'duty_cycle': lambda d: d,
            'smoothing': lambda s: min(int(s * 127), 127)
        }

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

        self.n_periods_buttons = self.add_controls(
            {n: NoteOnMapping(0, n) for n in xrange(16)},
            self.handle_n_periods_button)

        # FIXME-NUMERIC TARGETS
        self.target_buttons = self.add_controls(
            {target: NoteOnMapping(0, target+34)for target in AnimationTarget.VALUES},
            self.handle_target_button)

        # register input mappings
        self.register_callbacks()

    def handle_knob(self, mapping, value):
        knob = self.knobs.inv[mapping]
        setattr(self.ui, knob, self.knob_value_from_midi[knob](value))

    def set_knob(self, value, knob):
        mapping = self.knobs[knob]
        self.midi_out.send_from_mapping(mapping, self.knob_value_to_midi[knob](value))

    def handle_type_button(self, mapping, _):
        self.ui.type = self.type_buttons.inv[mapping]

    def set_type(self, set_type):
        self._set_radio_button(set_type, self.type_buttons)

    def handle_n_periods_button(self, mapping, _):
        self.ui.n_periods = self.n_periods_buttons.inv[mapping]

    def set_n_periods(self, value):
        self._set_radio_button(value, self.n_periods_buttons)

    def handle_target_button(self, mapping, _):
        self.ui.target = self.target_buttons.inv[mapping]

    def set_target(self, target):
        self._set_radio_button(target, self.target_buttons)
