from collections import namedtuple, defaultdict
from functools import partial
import logging as log
from rtmidi import MidiIn, MidiOut
from rtmidi.midiutil import open_midiport
from Queue import Queue
import akai_apc40

NoteOn = namedtuple('NoteOn', ('channel', 'pitch', 'velocity'))
NoteOff = namedtuple('NoteOff', ('channel', 'pitch', 'velocity'))
ControlChange = namedtuple('ControlChange', ('channel', 'control', 'value'))

MidiMapping = namedtuple('MidiMapping', ('channel', 'control', 'kind'))

# use kind argument and partials to ensure that notes and CCs hash differently
NoteOnMapping = partial(MidiMapping, kind='NoteOn')
NoteOffMapping = partial(MidiMapping, kind='NoteOff')
ControlChangeMapping = partial(MidiMapping, kind='ControlChange')

message_type_to_event_type = {
    'NoteOff': 8 << 4,
    'NoteOn': 9 << 4,
    'ControlChange': 11 << 4,
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

        # TODO: this type of situation should not be special-cased here
        # individual controller initialization should be handled in a general-
        # purpose fashion.
        # FIXME: should only send to APC40, not everything
        if name == akai_apc40.DEVICE_NAME:
            for note, val in akai_apc40.KNOB_SETTINGS:
                mapping = ControlChangeMapping(0, note)
                b0 = message_type_to_event_type[mapping.kind] + mapping[0]
                event = (b0, mapping[1], val)
                log.debug("sending {}, {} to {}".format(mapping, val, name))
                port.send_message(event)

    def close_port(self, port_name):
        """Remove and close a port."""
        port = self.ports.pop(port_name)
        port.close_port()

    def send_from_mappings(self, messages):
        """Send an arbitrary number of midi messages.

        Messages should be passed in as tuples of (mapping, value).
        """
        for mapping, value in messages:
            self.send_from_mapping(mapping, value)

    def send_from_mapping(self, mapping, value):
        """Send a midi message from a mapping and a payload."""
        b0 = message_type_to_event_type[mapping.kind] + mapping[0]
        event = (b0, mapping[1], value)
        for name, p in self.ports.iteritems():
            log.debug("sending {}, {} to {}".format(mapping, value, name))
            p.send_message(event)

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
        #self.mappings = defaultdict(set)
        self.controllers = set()

    def register_controller(self, controller):
        """Register a midi controller with the input service."""
        self.controllers.add(controller)

    def unregister_controller(self, controller):
        """Unregister a midi controller from the input service."""
        self.controllers.discard(controller)

    # def register_mappings(self, mappings):
    #     """Register handlers for midi mappings.

    #     mappings is an iterable of tuples of (MidiMapping, handler_method).
    #     handler_method should be a callable that can handle a midi message.
    #     """
    #     for mapping, handler in mappings.iteritems():
    #         self.mappings[mapping].add(handler)

    # def unregister_mappings(self, mappings):
    #     """Unregister a handler for an iterable of midi mappings.

    #     mappings is an iterable of tuples of (MidiMapping, handler_method).
    #     """
    #     for mapping, handler in mappings.iteritems():
    #         self.mappings[mapping].discard(handler)

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
        """Block until a message appears on the queue, then dispatch it.

        Optionally specify a timeout in seconds.
        """
        message = self.queue.get(timeout=timeout)
        log.debug("received {}".format(message))
        self._dispatch(*message)
        return message

    def _dispatch(self, mapping, payload):
        """Dispatch a midi message to the registered handlers."""
        for controller in self.controllers:
            handler = controller.controls.get(mapping, None)
            if handler is not None:
                handler(mapping, payload)
