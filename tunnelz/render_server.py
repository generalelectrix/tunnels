from collections import namedtuple, deque
import logging as log
from time import time, sleep
from multiprocessing import Process, Queue
from Queue import Empty
import msgpack
import zmq
import sys, traceback

arc_args = (
    'level', # int 0-255
    'stroke_weight', # float
    'hue',
    'sat',
    'val',
    'x', # int
    'y', # int
    'rad_x', #int
    'rad_y', #int
    'start', #float
    'stop' #float
    'rot_angle' #float
    )

Arc = namedtuple('Arc', arc_args)


line_args = (
    'level', # int 0-255
    'stroke_weight', # float
    'hue',
    'sat',
    'val',
    'x', # int
    'y', # int
    'length', #int
    'start', #float
    'stop' #float
    'rot_angle' #float
    )

Line = namedtuple('Line', line_args)

class DrawCommandAggregator (object):
    """Collect and flag draw commands."""
    def __init__(self):
        self.draw_calls = []
        self.draw_flags = []

    def add_draw_call(self, flag, draw_args):
        self.draw_calls.append(draw_args)
        self.draw_flags.append(flag)

def create_pub_socket(port):
    """Create a zmq PUB socket on a given port."""
    # TODO: learn about zmq context and if we should only have one of these.
    context = zmq.Context()
    socket = context.socket(zmq.PUB)
    addr = "tcp://*:%d" % port
    socket.bind(addr)
    return socket

# server commands
FRAME = "FRAME"
QUIT = "QUIT"

# server responses
RUNNING = "RUNNING"
FRAME_REQ = "FRAME_REQ"
FATAL_ERROR = "FATAL_ERROR"

class RenderServerError (Exception):
    pass

class RenderServer (object):
    """Responsible for launching and communicating with a render server."""

    def __init__(self, port=6000, framerate=30.0, report=False):
        self.port = port
        self.framerate = framerate
        self.running = False
        self.command = None
        self.response = None
        self.server_proc = None
        self.report = report
        self.last_draw = 0.0

    def start(self):
        """Launch an instance of the render server."""
        if not self.running:
            self.command = command = Queue()
            self.response = response = Queue()

            self.server_proc = server_proc = Process(
                target=run_server,
                args=(command, response, self.port, self.framerate, self.report))

            server_proc.start()
            # wait for server to succeed or fail
            resp, payload = response.get()

            if resp == FATAL_ERROR:
                raise Exception(payload[0], payload[1])
            elif resp == FRAME_REQ:
                # unclear how this happened.  kill the server and raise an error
                self._stop()
                raise Exception(
                    "Render server asked for a frame before reporting RUNNING.")
            elif resp != RUNNING:
                self._stop()
                raise Exception(
                    "Render server returned an unknown response: {}".format(resp))

            self.running = True
            self.last_draw = time()

    def _stop(self):
        """Kill the server."""
        if self.command is not None:
            self.command.put((QUIT, None))
            self.command = None
            self.response = None
            if self.server_proc is not None:
                self.server_proc.join()
            self.running = False

    def stop(self):
        """Stop the server if it is running."""
        if self.running:
            self._stop()

    def pass_frame_if_requested(self, mixer):
        """Pass the render server a frame if we have a request pending."""
        if self.running:
            try:
                req, payload = self.response.get(block=False)
            except Empty:
                return False
            else:
                if req == FRAME_REQ:
                    now = time()
                    # update the state of the beams
                    for layer in mixer.layers:
                        layer.beam.update_state(now - self.last_draw)

                    self.command.put((FRAME, mixer))
                    self.last_draw = now
                    return True
                elif req == FATAL_ERROR:
                    self._stop()
                    raise Exception(payload[0], payload[1])
        return False

def run_server(command, response, port, framerate, report):
    """Run the frame drawing service.

    The server runs as fast as it can, up to a specified framerate limit.  When
    it completes rendering a frame and sending it on the network, it waits until
    it is ready to draw another frame.  It then requests the model from the
    control process, renders it, sends it, and repeats.  The server keeps track
    of a smoothed processing time to help keep a constant framerate.

    The control protocol for the server's command queue is as follows:
    (command, payload)
    Examples are
    (FRAME, mixer) -> data payload to draw a frame
    (QUIT, _) -> quit the server thread

    The server communicates with the control thread over the response queue.
    It requests a frame with
    (FRAME_REQ, _)
    and reports a fatal, thread-death error with
    (FATAL_ERROR, err)
    """
    try:
        socket = create_pub_socket(port)

        # initialize the buffer to smooth render time
        # smooth five frames
        buffer_size = 5
        render_times = deque(maxlen=buffer_size)
        for _ in xrange(buffer_size):
            render_times.append(0.0)

        frame_period = 1.0 / framerate

        frame_number = 0

        # we're ready to render
        response.put((RUNNING, None))

        log_time = time()

        while 1:
            # time how long it takes to render this frame
            start = time()

            # request a frame from the controller
            response.put((FRAME_REQ, None))

            # wait for a reply
            action, payload = command.get()

            # check if time to quit
            if action == QUIT:
                return
            # no other valid commands besides FRAME
            elif action != FRAME:
                # blow up with fatal error
                # we could try again, but who knows how we even got here
                raise RenderServerError("Unrecognized command: {}".format(action))

            # render the payload we received
            draw_agg = payload.draw_layers()

            serialized = msgpack.dumps(
                (draw_agg.draw_flags, draw_agg.draw_calls),
                use_single_float=True)

            socket.send(serialized)

            dur = time() - start
            render_times.append(dur)

            # now wait until it is approximately time to start this process again
            sleep(frame_period - sum(render_times) / buffer_size)

            # debugging purposes
            frame_number += 1

            if report:# and frame_number % 1 == 0:
                now = time()
                log.debug("Framerate: {}".format(1 / (now - log_time)))
                log_time = now

    except Exception as err:
        # some exception we didn't catch
        _, _, tb = sys.exc_info()
        response.put((FATAL_ERROR, (err, traceback.format_tb(tb))))
        return
