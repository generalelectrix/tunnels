"""Simple service to roughly synchronize clocks between this server and clients."""

from time import monotonic
import msgpack
import zmq
from multiprocessing import Process
import logging

PORT = 8989

def create_rep_socket(port):
    """Create a zmq REP socket on a given port."""
    # TODO: learn about zmq context and if we should only have one of these.
    context = zmq.Context()
    socket = context.socket(zmq.REP)
    addr = "tcp://*:%d" % port
    socket.bind(addr)
    return socket

# FIXME this needs a quit mechanism
def run_service(port=PORT):
    """Run timestamp reply service.

    This service waits to receive a request, and replies with the current time.
    The content of a request packet is completely ignored.
    """
    def run():
        socket = create_rep_socket(port)
        while True:
            try:
                msg = socket.recv()
                now = monotonic()
                socket.send(msgpack.dumps(now))
            except Exception as err:
                logging.error(err)
    proc = Process(target=run)
    proc.start()

def test_receive():
    context = zmq.Context()
    socket = context.socket(zmq.REQ)
    addr = "tcp://localhost:%d" % PORT
    socket.connect(addr)
    socket.send("hello")
    msg = socket.recv()
    print("Received {}".format(msgpack.loads(msg)))