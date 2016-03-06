from .animation import AnimationClipboard
from .beam_matrix_minder import BeamMatrixMinder
from .draw_commands import write_layers_to_file
from .LED_control import (
    set_bump_button_LED,
    set_mask_button_LED,
    set_beam_save_LED,
    set_look_save_LED,
    set_look_edit_LED,
    set_is_look_LED,
    set_anim_select_LED,
    set_delete_LED,
    set_track_select_LED_radio,
    set_bottom_LED_rings,
    set_top_LED_rings,
    update_knob_state,)
from .midi import midi_in, NoteOn, NoteOff, ControlChange
from .mixer import Mixer
from Queue import Empty
import time
from .tunnel import Tunnel

# midi interface configuration

use_midi = False
midi_debug = False

use_APC = True
APCDeviceNumIn = 3
APCDeviceNumOut = 3

use_iPad = True
iPadDeviceNumIn = 2
iPadDeviceNumOut = 2

nMidiDev = 2


# the beam mixer
N_BEAMS = 8
mixer = Mixer(N_BEAMS)

# beam matrix minder
beam_matrix = BeamMatrixMinder()

# Animation clipboard
anim_clipboard = AnimationClipboard()


def setup():

    # FIXME-RENDERING
    # background(0) #black
    # #smooth() # anti-aliasing is SLOW
    # ellipseMode(RADIUS)
    # strokeCap(SQUARE)
    # frameRate(30)
    # colorMode(HSB)

    # open midi outputs
    if use_midi:
        pass
        # FIXME-MIDI LIBRARY
        # print all available midi devices

        # midiBusses = new MidiBus[nMidiDev]
        # MidiBus.list()

        # if (useAPC) {
        #     println("looking for an APC40 at input " + APCDeviceNumIn + ", output " + APCDeviceNumOut)
        #     midiBusses[0] = new MidiBus(this, APCDeviceNumIn, APCDeviceNumOut)
        # }

        # if (useiPad) {
        #     println("looking for an iPad at input " + iPadDeviceNumIn + ", output " + iPadDeviceNumOut)
        #     midiBusses[1] = new MidiBus(this, iPadDeviceNumIn, iPadDeviceNumOut)
        # }

    # open midi channels for each mixer channel and fill the mixer for now all tunnels.
    for i in xrange(mixer.n_layers):

        mixer.put_beam_in_layer(i, Tunnel())

        # can change defaults as the list is propagated here:
        if not use_midi:
            mixer.set_level(i, 255)

            tunnel = mixer.get_beam_from_layer(i)

            #tunnel.rot_speedI = 72
            tunnel.ellipse_aspectI = 64
            tunnel.col_widthI = 32
            tunnel.col_spreadI = 127
            tunnel.col_satI = 32

            tunnel.thicknessI = 40
            tunnel.radiusI = 64 - i*4
            #tunnel.radiusI = 50
            tunnel.rot_speedI = (i-4)*2+64
            tunnel.ellipse_aspectI = 64

            tunnel.blackingI = 20

            tunnel.update_params()

    # pretend we just pushed track select 1
    if use_midi:
        midi_input_handler(0, True, True, 0x33, 127)

    # save a copy of the default tunnel for sanity. Don't erase it!
    beam_matrix.put_beam(4, 7, Tunnel())

def run(framerate=30.0):
    frame_number = 0
    render_period = 1.0 / framerate
    last = time.time()
    while 1:
        process_control_events_until_render(render_period)
        draw()
        framenumber += 1
        if framenumber % 240 == 0:



# method called whenever processing draws a frame, basically the event loop
def draw(write=True, print_=False):

    # black out everything to remove leftover pixels
    # FIXME-RENDERING
    # background(0)

    layers = mixer.draw_layers()
    if print_:
        print layers
    if write:
        file = 'layer0.csv'
        write_layers_to_file(layers, file)


def controller_change(channel, number, value):
    """Callback from midi library.

    Args:
        int channel, int number, int value
    """
    if not keep_control_channel_data(number):
        channel = mixer.current_layer

    if midi_debug:
        fmt = "controller in\nchannel = {}\nnumber = {}\n value = {}"
        print fmt.format(channel, number, value)

    midi_input_handler(channel, False, False, number, value)

def keep_control_channel_data(num):
    return num == 7

def note_on(channel, pitch, velocity):
    """Callback from midi library.

    Args:
        int channel, int pitch, int velocity
    """
    channel_change = False

    # if this button is always channel 0
    if not keep_note_channel_data(pitch):
        channel = mixer.current_layer

    # if we pushed a track select button, not the master
    elif 0x33 == pitch and channel < 8:
        mixer.current_layer = channel
        channel_change = True

    if midi_debug:
        fmt = "note on\nchannel = {}\npitch = {}\n velocity = {}"
        print fmt.format(channel, pitch, velocity)

    midi_input_handler(channel, channel_change, True, pitch, 127)

def note_off(channel, pitch, velocity):
    """Callback from midi library.

    Args:
        int channel, int pitch, int velocity
    """
    # for now we're only using note off for bump buttons
    if 0x32 == pitch:
        midi_input_handler(channel, False, True, pitch, 0)

    if midi_debug:
        fmt = "note off\nchannel = {}\npitch = {}\n velocity = {}"
        print fmt.format(channel, pitch, velocity)

def keep_note_channel_data(num):
    """Does this note come from a button whose channel data we care about?"""
    return num >= 0x30 and num <= 0x39

message_dispatch = {
    NoteOn: note_on,
    NoteOff: note_off,
    ControlChange: controller_change,
}

def process_control_events_until_render(time_left):
    start = time.time()
    while True:
        time_until_render = time_left - (time.time() - start)
        # if it is time to render, stop the command loop
        if time_until_render <= 0.0:
            break

        # process control events
        try:
            # time out slightly before render time to improve framerate stability
            message = midi_in.receive(timeout=time_until_render*0.95)
        except Empty:
            # fine if we didn't get a control event
            pass
        else:
            # process the command
            message_dispatch[type(message)](*message)

def midi_input_handler(channel, chan_change, is_note, num, val):
    """Handle corrected incoming midi data.

    Args:
        int channel, boolean chan_change, boolean is_note, int num, int val
    """
    # ensure we don't retrieve null beams, make an exception for master channel
    if channel < mixer.n_layers:

        # --- mixer parameters ---

        # if the control is an upfader
        if 0x07 == num and not is_note:
            # special cases to allow scaling to 255
            if 0 == val:
                mixer.set_level(channel, 0)
            else:
                mixer.set_level(channel, 2*val + 1)

        # if a bump button
        elif 0x32 == num and is_note:
            if 127 == val:
                mixer.bump_on(channel)
                set_bump_button_LED(channel, True)
            else:
                mixer.bump_off(channel)
                set_bump_button_LED(channel, False)

        # if a mask button
        elif 0x31 == num and is_note:
            new_state = mixer.toggle_mask_state(channel)
            set_mask_button_LED(channel, new_state)

        # if not a mixer parameter
        else:

            # get the appropriate beam
            beam = mixer.get_current_beam()

            # if nudge+: animation paste
            if is_note and 0x64 == num:
                # ensure we don't paste null
                if anim_clipboard.has_data:
                    beam.replace_current_animation(anim_clipboard.paste())
                    beam.update_params()
                    update_knob_state(mixer.current_layer, beam)

            # if nudge-: animation copy
            elif is_note and 0x65 == num:
                anim_clipboard.copy(beam.get_current_animation())

            # beam save mode toggle
            elif is_note and 0x52 == num:

                # turn off look save mode
                beam_matrix.waiting_for_look_save = False
                set_look_save_LED(0)

                # turn off delete mode
                beam_matrix.waiting_for_delete = False
                set_delete_LED(0)

                # turn off look edit mode
                beam_matrix.waiting_for_look_edit = False
                set_look_edit_LED(0)

                # if we were already waiting for a beam save
                if beam_matrix.waiting_for_beam_save:

                    beam_matrix.waiting_for_beam_save = False
                    set_beam_save_LED(0)

                # we're activating beam save mode
                else:
                    # turn on beam save mode
                    beam_matrix.waiting_for_beam_save = True
                    set_beam_save_LED(2)
            # end beam save mode toggle

            # look save mode toggle
            elif is_note and 0x53 == num:

                # turn off beam save mode
                beam_matrix.waiting_for_beam_save = False
                set_beam_save_LED(0)

                # turn off delete mode
                beam_matrix.waiting_for_delete = False
                set_delete_LED(0)

                # turn off look edit mode
                beam_matrix.waiting_for_look_edit = False
                set_look_edit_LED(0)

                # if we were already waiting for a look save
                if beam_matrix.waiting_for_look_save:

                    beam_matrix.waiting_for_look_save = False
                    set_look_save_LED(0)

                # we're activating look save mode
                else:
                    beam_matrix.waiting_for_look_save = True
                    set_look_save_LED(2)
            # end look save mode toggle

            # delete saved element mode toggle
            elif is_note and 0x54 == num:

                # these buttons are radio
                beam_matrix.waiting_for_beam_save = False
                set_beam_save_LED(0)

                beam_matrix.waiting_for_look_save = False
                set_look_save_LED(0)

                # turn off look edit mode
                beam_matrix.waiting_for_look_edit = False
                set_look_edit_LED(0)

                if beam_matrix.waiting_for_delete:
                    beam_matrix.waiting_for_delete = False
                    set_delete_LED(0)

                # we're activating delete mode
                else:
                    beam_matrix.waiting_for_delete = True
                    set_delete_LED(2)

            # end delete element mode toggle

            # load look to edit mode toggle
            elif is_note and 0x56 == num:

                # these buttons are radio
                beam_matrix.waiting_for_beam_save = False
                set_beam_save_LED(0)

                beam_matrix.waiting_for_look_save = False
                set_look_save_LED(0)

                # turn off delete mode
                beam_matrix.waiting_for_delete = False
                set_delete_LED(0)

                # we're deactivating look edit
                if beam_matrix.waiting_for_look_edit:
                    beam_matrix.waiting_for_look_edit = False
                    set_look_edit_LED(0)

                # we're activating look edit mode
                else:
                    beam_matrix.waiting_for_look_edit = True
                    set_look_edit_LED(2)

            # end look edit mode toggle

            # if we just pushed a beam save matrix button
            elif is_note and num >= 0x35 and num <= 0x39 and channel < 8:

                # if we're in save mode
                if beam_matrix.waiting_for_beam_save:
                    beam_matrix.put_beam(
                        num - 0x35, channel, mixer.get_current_beam())
                    beam_matrix.waiting_for_beam_save = False
                    set_beam_save_LED(0)

                elif beam_matrix.waiting_for_look_save:
                    beam_matrix.put_look(
                        num - 0x35, channel, mixer.get_copy_of_current_look())
                    beam_matrix.waiting_for_look_save = False
                    set_look_save_LED(0)

                # if we're in delete mode
                elif beam_matrix.waiting_for_delete:
                    beam_matrix.clear_element(num - 0x35, channel)
                    beam_matrix.waiting_for_delete = False
                    set_delete_LED(0)

                # otherwise we're getting a thing from the minder
                else:

                    row = num - 0x35

                    if beam_matrix.element_has_data(row, channel):

                        saved_beam_vault = beam_matrix.get_element(row, channel)

                        is_look = beam_matrix.element_is_look(row, channel)

                        if is_look and beam_matrix.waiting_for_look_edit:
                            mixer.set_look(saved_beam_vault)

                        else:
                            mixer.set_current_beam(saved_beam_vault.retrieve_copy(0))

                        current_beam = mixer.get_current_beam()
                        update_knob_state(mixer.current_layer, current_beam)
                        set_anim_select_LED(current_beam.curr_anim)

                        if is_look and not beam_matrix.waiting_for_look_edit:
                            set_is_look_LED(mixer.current_layer, True)
                        else:
                            set_is_look_LED(mixer.current_layer, False)

                        beam_matrix.waiting_for_look_edit = False
                        set_look_edit_LED(0)

            # if beam-specific parameter:
            else:

                beam.set_midi_param(is_note, num, val)

                # update knob state if we've changed channel
                if chan_change:
                    set_track_select_LED_radio(mixer.current_layer)
                    set_bottom_LED_rings(mixer.current_layer, beam)
                    set_top_LED_rings(beam)
                    set_anim_select_LED(beam.curr_anim)

                # call the update method
                beam.update_params()
                update_knob_state(mixer.current_layer, beam)

