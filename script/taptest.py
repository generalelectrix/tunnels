from tunnelz.midi import *
from tunnelz.clock import TapSync

ts = TapSync()

def handle_tap(event, _):
    (b0, b1, b2), _ = event
    event_type, channel = b0 >> 4, b0 & 15

    if event_type == 9:
        ts.tap()

if __name__ == "__main__":
    port, name = open_midiport(1)

    print name

    port.set_callback(handle_tap)

    while 1:
        pass