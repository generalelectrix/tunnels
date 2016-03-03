# FIXME-MIDI
def send_CC(channel, number, val):
    """wrapper method for sending midi control changes"""
    # FIXME-MIDI
    print "send CC {}, {}, {}".format(channel, number, val)
    # if use_midi:
    #     if use_APC:
    #         midi_busses[0].sendControllerChange(channel, number, val)
    #     if use_iPad:
    #         midi_busses[1].sendControllerChange(channel, number, val)

# FIXME-MIDI
def send_note(channel, number, velocity):
    """wrapper method for sending midi notes"""
    print "send note on {}, {}, {}".format(channel, number, velocity)
    # if use_midi:
    #     if use_APC:
    #         midi_busses[0].sendNoteOn(channel, number, velocity)
    #     if use_iPad:
    #         midi_busses[1].sendNoteOn(channel, number, velocity)