import time
from tunnelz import tunnelz
from tunnelz.midi import NoteOn, ControlChange

def run():
    tunnelz.setup()
    n_ave = 10
    last = time.time()
    for n in xrange(100):
        tunnelz.draw()
        if (n + 1) % n_ave == 0:

            now = time.time()
            print "{} fps".format(n_ave / (now - last))
            last = now

def test_message_performance():
    tunnelz.setup()
    ch1select = NoteOn(0, 0x33, 127)
    for _ in xrange(100000):
        tunnelz.midi_in.queue.put(ch1select)
    tunnelz.run(n_frames=100)