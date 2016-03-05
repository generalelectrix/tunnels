from collections import namedtuple
from rtmidi import MidiIn, MidiOut
from rtmidi.midiutil import open_midiport
from Queue import Queue

NoteOn = namedtuple('NoteOn', ('channel', 'pitch', 'velocity'))
NoteOff = namedtuple('NoteOff', ('channel', 'pitch', 'velocity'))
ControlChange = namedtuple('ControlChange', ('channel', 'control', 'value'))

message_type_to_event_type = {
    NoteOff: 8 << 4,
    NoteOn: 9 << 4,
    ControlChange: 11 << 4
}

def list_ports():
    """Print the available ports."""
    print "Available input ports:"
    MidiIn().get_ports()
    print "\nAvailable output ports:"
    MidiOut().get_ports()


class MidiOutput (object):
    """Aggregate multiple midi outputs into one front end."""
    def __init__(self):
        self.ports = {}

    def open_port(self, port_number):
        """Add a new port to send messages to."""
        port, name = open_midiport(port_number, type_="output")
        self.ports[name] = port

    def close_port(self, port_name):
        """Remove and close a port."""
        port = self.ports.pop(port_name)
        port.close_port()

    def send(self, *messages):
        """Send an arbitrary number of midi messages."""
        for message in messages:
            b0 = message_type_to_event_type[type(message)] + message[0]
            event = (b0, message[1], message[2])
            for port in self.ports.itervalues():
                port.send_message(event)

# mapping between event type and constructor
event_type_to_constructor = {
    8: NoteOff,
    9: NoteOn,
    11: ControlChange
}

class MidiInput (object):
    """A queue-based system for dealing with receiving midi messages."""

    def __init__(self):
        """Initialize the message queue."""
        self.queue = Queue()
        self.ports = {}

    def open_port(self, port_number):
        """Open a new midi port to feed the message queue."""
        port, name = open_midiport(port_number)
        self.ports[name] = port

        queue = self.queue
        def parse(event, data):
            b0, b1, b2 = event
            event_type, channel = b0 >> 4, b0 & 7
            message = event_dispatch[event_type](channel, b1, b2)
            queue.put((event_type, message))
        port.set_callback(parse)

    def close_port(self, port_name):
        port = self.ports.pop(port_name)
        port.cancel_callback()
        port.close_port()

    def receive(self, timeout=None):
        """Block until a message appears on the queue.

        Optionally specify a timeout in seconds.
        """
        return self.queue.get(timeout=timeout)

# FIXME-GLOBAL BULLSHIT
midi_in = MidiInput()
midi_in.open_port(1)
midi_out = MidiOutput()
midi_out.open_port(1)


# FIXME-GLOBAL
def send_CC(channel, number, val):
    """wrapper method for sending midi control changes"""
    midi_out.send(ControlChange(channel, number, val))

# FIXME-GLOBAL
def send_note(channel, number, velocity):
    """wrapper method for sending midi notes"""
    midi_out.send(NoteOn(channel, number, velocity))