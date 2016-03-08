from collections import namedtuple, defaultdict
from functools import partial
import logging as log
from rtmidi import MidiIn, MidiOut
from rtmidi.midiutil import open_midiport
from Queue import Queue

NoteOn = namedtuple('NoteOn', ('channel', 'pitch', 'velocity'))
NoteOff = namedtuple('NoteOff', ('channel', 'pitch', 'velocity'))
ControlChange = namedtuple('ControlChange', ('channel', 'control', 'value'))

MidiMapping = namedtuple('MidiMapping', ('channel', 'control', 'kind'))

# use kind argument and partials to ensure that notes and CCs hash differently
NoteOnMapping = partial(MidiMapping, kind='NoteOn')
NoteOffMapping = partial(MidiMapping, kind='NoteOff')
ControlChangeMapping = partial(MidiMapping, kind='ControlChange')

message_type_to_event_type = {
    NoteOff: 8 << 4,
    NoteOffMapping: 8 << 4,
    NoteOn: 9 << 4,
    NoteOnMapping: 9 << 4,
    ControlChange: 11 << 4,
    ControlChangeMapping: 11 << 4,
}

def list_ports():
    """Print the available ports."""
    log.info("Available input ports:\n{}".format(MidiIn().get_ports()))
    log.info("Available output ports:\n{}".format(MidiOut().get_ports()))

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
            log.debug("sending {}".format(message))
            b0 = message_type_to_event_type[type(message)] + message[0]
            event = (b0, message[1], message[2])
            for port in self.ports.itervalues():
                port.send_message(event)

    def send_from_mapping(self, *messages):
        """Send an arbitrary number of midi messages.

        Messages should be passed in as tuples of (mapping, value).
        """
        for mapping, value in messages:
            log.debug("sending {}".format(mapping, value))
            b0 = message_type_to_event_type[type(mapping)] + mapping[0]
            event = (b0, mapping[1], value)
            for port in self.ports.itervalues():
                port.send_message(event)

    def send_note(self, channel, pitch, velocity):
        """Send a note on message."""
        for name, port in self.ports.iteritems():
            log.debug("sending note on to {}: {}, {}, {}".format(name, channel, pitch, velocity))
            port.send_message((144 + channel, pitch, velocity))

    def send_cc(self, channel, control, value):
        """Send a control change message."""
        for name, port in self.ports.iteritems():
            log.debug("sending cc to {}: {}, {}, {}".format(name, channel, control, value))
            port.send_message((176 + channel, control, value))

# mapping between event type and constructor
event_type_to_mapping = {
    8: NoteOffMapping,
    9: NoteOnMapping,
    11: ControlChangeMapping,
}

class MidiInput (object):
    """A queue-based system for dealing with receiving midi messages."""

    def __init__(self):
        """Initialize the message queue."""
        self.queue = Queue()
        self.ports = {}
        self.mappings = defaultdict(set)

    def register_mappings(self, mappings):
        """Register handlers for midi mappings.

        mappings is an iterable of tuples of (MidiMapping, handler_method).
        handler_method should be a callable that can handle a midi message.
        """
        for mapping, handler in mappings:
            self.mappings[mapping].add(handler)

    def unregister_mappings(self, mappings):
        """Unregister a handler for an iterable of midi mappings.

        mappings is an iterable of tuples of (MidiMapping, handler_method).
        """
        for mapping, handler in mappings:
            self.mappings[mapping].discard(handler)

    def open_port(self, port_number):
        """Open a new midi port to feed the message queue."""
        port, name = open_midiport(port_number)
        self.ports[name] = port

        queue = self.queue
        def parse(event, data):
            (b0, b1, b2), _ = event
            event_type, channel = b0 >> 4, b0 & 7
            message = (event_type_to_mapping[event_type](channel, b1), b2)
            queue.put(message)
        port.set_callback(parse)

    def close_port(self, port_name):
        port = self.ports.pop(port_name)
        port.cancel_callback()
        port.close_port()

    def receive(self, timeout=None):
        """Block until a message appears on the queue.

        Optionally specify a timeout in seconds.
        """
        message = self.queue.get(timeout=timeout)
        log.debug("received {}".format(message))
        return message

    def dispatch(self, mapping, payload):
        """Dispatch a midi message to the registered handlers."""
        handlers = self.mappings.get(mapping, tuple())
        for handler in handlers:
            handler.handle_message(mapping, payload)


# FIXME-GLOBAL BULLSHIT
midi_in = MidiInput()
midi_out = MidiOutput()
