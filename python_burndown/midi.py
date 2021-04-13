from collections import namedtuple, deque
from functools import partial
import logging as log
import weakref

from rtmidi import MidiIn, MidiOut
from rtmidi.midiutil import open_midiport


# mapping between event type and constructor
event_type_to_mapping = {
    8: NoteOffMapping,
    9: NoteOnMapping,
    11: ControlChangeMapping,
}

class MidiInput (object):
    """A queue-based system for dealing with receiving midi messages.

    Each input parses and buffers its own midi messages.  Whenever a message is
    received, we put this input into a queue of inputs to be serviced in order
    of message arrival by the main thread.  This allows each input to be bound
    to its own controller while still providing a serialization checkpoint.
    """

    def __init__(self, port_number, service_queue):
        """Initialize a midi output from a port number.

        Hold onto a reference to the queue that will service this input during
        main thread control processing.
        """
        self._message_buffer = message_buffer = deque()

        self._controllers = set()

        port, name = open_midiport(port_number)
        self.name = name
        self._port = port

        # pass weak references to message handler to avoid accidentally keeping
        # this input alive
        handler_ref = weakref.ref(self)

        def parse(event, _):
            """Callback called by the thread handling midi receipt.

            Parse the message into a more useful type, and queue up the message
            as well as the input to be serviced.
            """
            (b0, b1, b2), _ = event
            event_type, channel = b0 >> 4, b0 & 15
            message = (event_type_to_mapping[event_type](channel, b1), b2)

            # put the message into the buffer to be handled by this input
            message_buffer.appendleft(message)
            # queue this input up for servicing
            service_queue.put(handler_ref)

        port.set_callback(parse)

    def register_controller(self, controller):
        """Register a midi controller with the input service."""
        self._controllers.add(controller)

    def handle_message(self):
        """Dispatch a message from our message buffer if it isn't empty."""
        try:
            message = self._message_buffer.pop()
            log.debug("Input {} handling {}".format(self.name, message))
        except IndexError:
            log.debug(
                "Midi input {} had no message yet was called to handle one."
                .format(self.name))
            return

        log.debug("Input {} received {}".format(self.name, message))
        self._dispatch(*message)

    def _dispatch(self, mapping, payload):
        """Dispatch a midi message to the registered handlers."""
        for controller in self._controllers:
            handler = controller.controls.get(mapping, None)
            if handler is not None:
                handler(mapping, payload)
