from .animation import WaveformType, AnimationTarget
from .beam_matrix_minder import (
    Idle,
    BeamSave,
    LookSave,
    Delete,
    LookEdit,
    ButtonEmpty,
    ButtonBeam,
    ButtonLook,
    BeamMatrixMinder,
)
from bidict import bidict
from collections import namedtuple
import logging
from .midi import NoteOnMapping, NoteOffMapping, ControlChangeMapping

def _build_grid_button_map(page):
    mapping = {}
    col_offset = BeamMatrixMinder.col_per_page * page
    for row in range(BeamMatrixMinder.n_rows):
        for column in range(BeamMatrixMinder.col_per_page):
            mapping[(row, column+col_offset)] = NoteOnMapping(column, row + 0x35)
    return bidict(mapping)


class MidiController (object):
    """Base class for midi controllers."""

    def __init__(self, mi, midi_out):
        self.controls = {}
        self.mi = mi
        mi.controllers.add(self)

        self.midi_out = midi_out

        self.setup_controls()

    def setup_controls(self):
        """Subclasses should override this method to wire up controls."""
        pass

    def add_controls(self, control_map, callback):
        """Attach a control map to a specified callback.

        Returns the bidirectional version of the control map.
        """
        self.set_callback_for_mappings(control_map.values(), callback)
        return bidict(control_map)

    def set_callback(self, mapping, callback):
        """Register a callback for a single mapping."""
        self.controls[mapping] = callback
        return mapping

    def set_callback_for_mappings(self, mappings, callback):
        """Manually register a callback for an iterable of mappings."""
        for mapping in mappings:
            self.controls[mapping] = callback

    def _set_radio_button(self, set_value, control_map):
        """Set only one out of a set of controls on."""
        for value, mapping in control_map.items():
            self.midi_out.send_from_mapping(mapping, int(value == set_value))

    # --- helper functions for useful knobs ---

    @staticmethod
    def bipolar_from_midi(val):
        """MIDI knob to a bipolar float."""
        if val > 64:
            return float(val - 64)/63
        elif val < 64:
            return float(val - 64)/64
        else:
            return 0.0
        # there are issues with adding a detent in this way, as the knob stays
        # snapped in place when you try to turn it away from 0!  Need to add the
        # detent logic at a lower level, unfortunately.
        # if val > 65:
        #     return -float(val - 65)/62
        # elif val < 63:
        #     return -float(val - 63)/63
        # else:
        #     return 0.0

    @staticmethod
    def bipolar_to_midi(val):
        """Bipolar float to a midi knob."""
        return min(int(((val + 1.0) / 2.0) * 128), 127)

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

    button_state_value_map = {
        ButtonEmpty: (0, 1), # off, red
        ButtonLook: (1, 1), # on, red
        ButtonBeam: (1, 2) # on, orange
    }

    def __init__(self, mi, midi_out, page=0):
        self.page = page
        super(BeamMatrixMidiController, self).__init__(mi, midi_out)

    def setup_controls(self):
        self.grid_button_map = _build_grid_button_map(self.page)
        # the controls which will be registered with the midi service
        self.set_callback_for_mappings(
            self.grid_button_map.values(), self.handle_grid_button)
        self.set_callback_for_mappings(
            self.control_map.values(), self.handle_state_button)

    def column_in_range(self, col):
        """Return True if this column is on the page assigned to this controller."""
        col_count = BeamMatrixMinder.col_per_page
        first_col = col_count * self.page
        last_col = first_col + col_count - 1
        return col >= first_col and col <= last_col

    def handle_grid_button(self, mapping, payload):
        row, col = self.grid_button_map.inv[mapping]
        self.mi.grid_button_press(row, col)

    def handle_state_button(self, mapping, payload):
        self.mi.state_toggle(self.control_map.inv[mapping])

    def set_beam_matrix_state(self, state):
        """Send UI update commands based on the beam matrix state."""
        led_state = self.state_to_led_state_map[state]
        message_mappings = tuple(
            (mapping, getattr(led_state, control))
            for control, mapping in self.control_map.items())
        self.midi_out.send_from_mappings(message_mappings)

    def set_button_state(self, row, column, state):
        # ignore updates for other pages
        if not self.column_in_range(column):
            return

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

    def __init__(self, mi, midi_out, page=0, page_size=8):
        """Set up metacontrol for a particular page of channels.

        This is all hardcoded for the APC20/40 and associated controls on the
        touchOSC interface.
        """
        self.page = page
        self.page_size = page_size
        self.channel_offset = page * page_size

        super(MetaControlMidiController, self).__init__(mi, midi_out)

    def setup_controls(self):
        # offset the internal channels, but not the midi channels, based on
        # what page we're controlling

        ts_mappings = {
            chan+self.channel_offset: NoteOnMapping(chan, 0x33)
            for chan in range(self.page_size)}

        self.track_select = self.add_controls(
            ts_mappings,
            self.handle_current_layer)

        # TODO: DRY out number of animators
        self.animation_select_buttons = self.add_controls({
            n: NoteOnMapping(0, 0x57+n) for n in range(4)},
            self.handle_current_animator)

        self.set_callback(NoteOnMapping(0, 0x65), self.handle_animation_copy)
        self.set_callback(NoteOnMapping(0, 0x64), self.handle_animation_paste)

    def handle_current_layer(self, mapping, _):
        chan = mapping[0] + self.channel_offset
        self.mi.set_current_layer(chan)

    def set_current_layer(self, layer):
        """Emit the midi messages to change the selected mixer channel."""
        self._set_radio_button(layer, self.track_select)

    def handle_current_animator(self, mapping, _):
        n = self.animation_select_buttons.inv[mapping]
        self.mi.set_current_animator(n)

    def set_current_animator(self, anim_num):
        self._set_radio_button(anim_num, self.animation_select_buttons)

    def handle_animation_copy(self, _, val):
        self.mi.animation_copy()

    def handle_animation_paste(self, _, val):
        self.mi.animation_paste()


def ignore_out_of_range(method):
    """Ignore a layer control action if it is out of range for this mixer."""
    def check_range(self, layer, *args, **kwargs):
        if self.layer_in_range(layer):
            return method(self, layer, *args, **kwargs)
    return check_range


class MixerMidiController (MidiController):

    def __init__(self, mi, midi_out, page=0, page_size=8):
        """Set up a mixer to control a particular page of channels.

        This is all hardcoded for the APC20/40 and associated controls on the
        touchOSC interface.
        """
        assert (page+1) * page_size <= mi.mixer.layer_count
        self.page = page
        self.page_size = page_size

        super(MixerMidiController, self).__init__(mi, midi_out)

    def layer_in_range(self, layer):
        """Return True if this layer is on the page assigned to this controller."""
        start_chan = self.page * self.page_size
        end_chan = start_chan + self.page_size - 1
        return layer >= start_chan and layer <= end_chan

    def setup_controls(self):

        self.channel_faders = bidict()
        self.bump_button_on = bidict()
        self.bump_button_off = bidict()
        self.mask_buttons = bidict()
        self.look_indicators = bidict()

        # add controls for all mixer channels for this page
        offset = self.page * self.page_size

        for chan in range(self.page_size):
            # tricky; need to offset the internal channel while keeping the midi
            # channel in the range 0-7 to match the APC layout.
            self.channel_faders[chan+offset] = ControlChangeMapping(chan, 0x7)
            self.bump_button_on[chan+offset] = NoteOnMapping(chan, 0x32)
            self.bump_button_off[chan+offset] = NoteOffMapping(chan, 0x32)
            self.mask_buttons[chan+offset] = NoteOnMapping(chan, 0x31)
            self.look_indicators[chan+offset] = NoteOnMapping(chan, 0x30)

        # update the controls
        self.set_callback_for_mappings(
            self.channel_faders.values(), self.handle_channel_fader)
        self.set_callback_for_mappings(
            self.bump_button_on.values(), self.handle_bump_button_on)
        self.set_callback_for_mappings(
            self.bump_button_off.values(), self.handle_bump_button_off)
        self.set_callback_for_mappings(
            self.mask_buttons.values(), self.handle_mask_button)

        # configure video channel select
        # broken out as a method as we will probably want to move this eventually
        self.setup_video_channel_select()


    def handle_channel_fader(self, mapping, value):
        chan = self.channel_faders.inv[mapping]
        # map midi range to 1.0
        self.mi.set_level(chan, self.unipolar_from_midi(value))

    def handle_bump_button_on(self, mapping, _):
        chan = self.bump_button_on.inv[mapping]
        self.mi.set_bump_button(chan, True)

    def handle_bump_button_off(self, mapping, _):
        chan = self.bump_button_off.inv[mapping]
        self.mi.set_bump_button(chan, False)

    def handle_mask_button(self, mapping, _):
        chan = self.mask_buttons.inv[mapping]
        self.mi.toggle_mask_state(chan)

    @ignore_out_of_range
    def set_level(self, layer, level):
        """Emit midi messages to update layer level."""
        mapping = self.channel_faders[layer]
        self.midi_out.send_from_mapping(mapping, self.unipolar_to_midi(level))

    @ignore_out_of_range
    def set_bump_button(self, layer, state):
        """Emit the midi messages to change the bump button state.

        Args:
            layer: which layer to set button
            state (boolean): on or off
        """
        mapping = self.bump_button_on[layer]
        self.midi_out.send_from_mapping(mapping, int(state))

    @ignore_out_of_range
    def set_mask_button(self, layer, state):
        """Emit the midi messages to change the mask button state."""
        mapping = self.mask_buttons[layer]
        self.midi_out.send_from_mapping(mapping, int(state))

    @ignore_out_of_range
    def set_look_indicator(self, layer, state):
        """Emit the midi messages to change the look indicator state."""
        mapping = self.look_indicators[layer]
        self.midi_out.send_from_mapping(mapping, int(state))

    # FIXME: this is a temporary hack until we improve the iPad interface to
    # be able to perform a generic page select action.
    def setup_video_channel_select(self):
        self.video_channel_selects = bidict()

        for chan in range(self.mi.mixer.layer_count):
            for video_chan in range(self.mi.mixer.n_video_channels):
                chan_0_midi_note = 66
                mapping = NoteOnMapping(chan, chan_0_midi_note + video_chan)
                self.video_channel_selects[(chan, video_chan)] = mapping

        self.set_callback_for_mappings(
            self.video_channel_selects.values(),
            self.handle_video_channel_select)


    def handle_video_channel_select(self, mapping, _):
        layer, video_chan = self.video_channel_selects.inv[mapping]
        self.mi.toggle_video_channel(layer, video_chan)

    # note that we don't need to check layer range here as the video selection
    # interface spans all 16 channels natively
    def set_video_channel(self, layer, video_chan, state):
        """Emit midi message to set layer select state."""
        mapping = self.video_channel_selects[(layer, video_chan)]
        self.midi_out.send_from_mapping(mapping, int(state))


class TunnelMidiController (MidiController):

    def setup_controls(self):

        self.unipolar_knobs = self.add_controls({
            'thickness': ControlChangeMapping(0, 21),
            'size': ControlChangeMapping(0, 22),
            'col_center': ControlChangeMapping(0, 16),
            'col_width': ControlChangeMapping(0, 17),
            'col_spread': ControlChangeMapping(0, 18),
            'col_sat': ControlChangeMapping(0, 19),
            'aspect_ratio': ControlChangeMapping(0, 23),
            },
            self.handle_unipolar_knob)

        self.bipolar_knobs = self.add_controls({
                'rot_speed': ControlChangeMapping(0, 52),
                'marquee_speed': ControlChangeMapping(0, 20),
                'blacking': ControlChangeMapping(0, 54),
            },
            self.handle_bipolar_knob)

        self.segs_mapping = ControlChangeMapping(0, 53)

        self.set_callback(self.segs_mapping, self.handle_segs)

        self.nudge_x_pos_mapping = NoteOnMapping(0, 0x60)
        self.nudge_x_neg_mapping = NoteOnMapping(0, 0x61)
        self.nudge_y_pos_mapping = NoteOnMapping(0, 0x5F)
        self.nudge_y_neg_mapping = NoteOnMapping(0, 0x5E)
        self.position_reset_mapping = NoteOnMapping(0, 0x62)
        self.rotation_reset_mapping = NoteOnMapping(0, 120)
        self.marquee_reset_mapping = NoteOnMapping(0, 121)

        self.set_callback(self.nudge_x_pos_mapping, self.handle_nudge_x_pos)
        self.set_callback(self.nudge_x_neg_mapping, self.handle_nudge_x_neg)
        self.set_callback(self.nudge_y_pos_mapping, self.handle_nudge_y_pos)
        self.set_callback(self.nudge_y_neg_mapping, self.handle_nudge_y_neg)
        self.set_callback(self.position_reset_mapping, self.handle_reset_beam_position)
        self.set_callback(self.rotation_reset_mapping, self.handle_reset_beam_rotation)
        self.set_callback(self.marquee_reset_mapping, self.handle_reset_beam_marquee)

    def handle_unipolar_knob(self, mapping, val):
        knob = self.unipolar_knobs.inv[mapping]
        setattr(self.mi, knob, self.unipolar_from_midi(val))

    def set_unipolar(self, val, knob):
        mapping = self.unipolar_knobs[knob]
        self.midi_out.send_from_mapping(mapping, self.unipolar_to_midi(val))

    def handle_bipolar_knob(self, mapping, val):
        knob = self.bipolar_knobs.inv[mapping]
        setattr(self.mi, knob, self.bipolar_from_midi(val))

    def set_bipolar(self, val, knob):
        mapping = self.bipolar_knobs[knob]
        self.midi_out.send_from_mapping(mapping, self.bipolar_to_midi(val))

    def handle_segs(self, _, val):
        """Convert midi to number of segments."""
        self.mi.segs = val + 1

    def set_segs(self, segs):
        """Convert number of segs back to midi."""
        self.midi_out.send_from_mapping(self.segs_mapping, segs - 1)

    def handle_nudge_x_pos(self, _, val):
        self.mi.nudge_x_pos()

    def handle_nudge_x_neg(self, _, val):
        self.mi.nudge_x_neg()

    def handle_nudge_y_pos(self, _, val):
        self.mi.nudge_y_pos()

    def handle_nudge_y_neg(self, _, val):
        self.mi.nudge_y_neg()

    def handle_reset_beam_position(self, _, val):
        self.mi.reset_beam_position()

    def handle_reset_beam_rotation(self, _, val):
        self.mi.reset_beam_rotation()

    def handle_reset_beam_marquee(self, _, val):
        self.mi.reset_beam_marquee()

class AnimationMidiController (MidiController):

    def setup_controls(self):

        self.knobs = self.add_controls({
            'speed': ControlChangeMapping(0, 48),
            'weight': ControlChangeMapping(0, 49),
            'duty_cycle': ControlChangeMapping(0, 50),
            'smoothing': ControlChangeMapping(0, 51),
            },
            self.handle_knob)

        self.knob_value_from_midi = {
            'speed': self.bipolar_from_midi,
            'weight': self.unipolar_from_midi,
            'duty_cycle': self.unipolar_from_midi,
            'smoothing': self.unipolar_from_midi
        }

        self.knob_value_to_midi = {
            'speed': self.bipolar_to_midi,
            'weight': self.unipolar_to_midi,
            'duty_cycle': self.unipolar_to_midi,
            'smoothing': self.unipolar_to_midi,
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
            {n: NoteOnMapping(0, n) for n in range(16)},
            self.handle_n_periods_button)

        # FIXME-NUMERIC TARGETS
        self.target_buttons = self.add_controls(
            {target: NoteOnMapping(0, target+34) for target in AnimationTarget.VALUES},
            self.handle_target_button)

        self.pulse_button = self.set_callback(NoteOnMapping(1, 0), self.handle_pulse_button)
        self.invert_button = self.set_callback(NoteOnMapping(1, 1), self.handle_invert_button)

        # map external clock select
        clock_buttons = {i: NoteOnMapping(0, 112+i) for i in range(8)}
        clock_buttons[None] = NoteOnMapping(0, 111)

        self.clock_buttons = self.add_controls(clock_buttons, self.handle_clock_button)

    def handle_knob(self, mapping, value):
        knob = self.knobs.inv[mapping]
        setattr(self.mi, knob, self.knob_value_from_midi[knob](value))

    def set_knob(self, value, knob):
        mapping = self.knobs[knob]
        self.midi_out.send_from_mapping(mapping, self.knob_value_to_midi[knob](value))

    def handle_type_button(self, mapping, _):
        self.mi.type = self.type_buttons.inv[mapping]

    def set_type(self, set_type):
        self._set_radio_button(set_type, self.type_buttons)

    def handle_pulse_button(self, mapping, _):
        self.mi.toggle_pulse()

    def set_pulse(self, val):
        self.midi_out.send_from_mapping(self.pulse_button, int(val))

    def handle_invert_button(self, mapping, _):
        self.mi.toggle_invert()

    def set_invert(self, val):
        self.midi_out.send_from_mapping(self.invert_button, int(val))

    def handle_n_periods_button(self, mapping, _):
        self.mi.n_periods = self.n_periods_buttons.inv[mapping]

    def set_n_periods(self, value):
        self._set_radio_button(value, self.n_periods_buttons)

    def handle_target_button(self, mapping, _):
        self.mi.target = self.target_buttons.inv[mapping]

    def set_target(self, target):
        self._set_radio_button(target, self.target_buttons)

    def handle_clock_button(self, mapping, _):
        self.mi.clock = self.clock_buttons.inv[mapping]

    def set_clock_source(self, clock):
        self._set_radio_button(clock, self.clock_buttons)


class ClockMidiController (MidiController):
    """Wire up controls for a single clock."""

    def __init__(self, mi, midi_out, channel):
        """Initialize this clock controller to use a particular midi channel."""
        self.channel = channel
        super(ClockMidiController, self).__init__(mi, midi_out)


    def setup_controls(self):
        self.set_callback(NoteOnMapping(self.channel, 110), self.handle_tap)
        self.set_callback(ControlChangeMapping(self.channel, 1), self.handle_nudge)

        self.retrigger_control = ControlChangeMapping(self.channel, 0)
        self.set_callback(self.retrigger_control, self.handle_retrigger)

        self.one_shot_control = ControlChangeMapping(self.channel, 2)
        self.set_callback(self.one_shot_control, self.handle_one_shot)

        self.tick_on = NoteOnMapping(self.channel, 109)
        self.tick_off = NoteOffMapping(self.channel, 109)

        self.submaster_level_control = ControlChangeMapping(self.channel, 3)
        self.set_callback(self.submaster_level_control, self.handle_submaster_level)

    def handle_tap(self, mapping, _):
        self.mi.tap()

    def handle_nudge(self, mapping, value):
        """Nudge knob is an infinite encoder.

        Values > 64 indicate positive nudge, <64 negative nudge.
        """
        self.mi.nudge(value - 64)

    def ticked(self, ticked):
        self.midi_out.send_from_mapping(
            self.tick_on if ticked else self.tick_off, 127 if ticked else 0)

    def handle_retrigger(self, mapping, value):
        self.mi.retrigger = bool(value)

    def set_retrigger(self, value):
        self.midi_out.send_from_mapping(self.retrigger_control, 127 if value else 0)

    def handle_one_shot(self, mapping, value):
        self.mi.one_shot = bool(value)

    def set_one_shot(self, value):
        self.midi_out.send_from_mapping(self.one_shot_control, 127 if value else 0)

    def handle_submaster_level(self, mapping, value):
        scaled = self.unipolar_from_midi(value)
        self.mi.submaster_level = scaled

    def set_submaster_level(self, value):
        unscaled = self.unipolar_to_midi(value)
        self.midi_out.send_from_mapping(self.submaster_level_control, unscaled)
