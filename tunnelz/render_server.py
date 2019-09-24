import logging as log
from monotonic import monotonic
from multiprocessing import Process, Queue
from queue import Empty
import msgpack
import zmq
import sys
import traceback

# this is unused but serves as documentation
# TODO: use a schematized zero-copy serialization format, maybe cap'n proto
arc_args = (
    'level', # unipolar float
    'stroke_weight', # float
    'hue', # unipolar float
    'sat', # unipolar float
    'val', # unipolar float
    'x', # float
    'y', # float
    'rad_x', # float
    'rad_y', # float
    'start', # float
    'stop', # float
    'rot_angle', # float
)


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
    """Responsible for launching and communicating with a render server process."""

    def __init__(self, port=6000, report=False):
        """Create a new render server handle.

        Args:
            port: port to open the server on
            report (bool): have the render process print debugging information
        """
        self.port = port
        self.running = False
        self.command = None
        self.response = None
        self.server_proc = None
        self.report = report

    def start(self):
        """Launch an instance of the render server."""
        if not self.running:
            self.command = command = Queue()
            self.response = response = Queue()

            self.server_proc = server_proc = Process(
                target=run_server,
                args=(command, response, self.port, self.report))

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

    def pass_frame_if_ready(self, update_number, update_time, mixer, clocks):
        """Pass the render server a frame if it is ready to draw one.

        Returns a boolean indicating if a frame was drawn or not.
        """
        if self.running:
            try:
                req, payload = self.response.get(block=False)
            except Empty:
                return False
            else:
                if req == FRAME_REQ:
                    # just pass the underlying clock objects, not the whole
                    # clock command wrapper
                    bare_clocks = [clock.model for clock in clocks]
                    self.command.put((
                        FRAME,
                        (update_number, update_time, mixer, bare_clocks),
                    ))
                    return True
                elif req == FATAL_ERROR:
                    self._stop()
                    raise RenderServerError(payload[0], payload[1])
                else:
                    raise RenderServerError(
                        "Unknown response: {}, {}".format(req, payload))
        return False

def run_server(command, response, port, report):
    """Run the frame drawing service.

    The control protocol for the server's command queue is as follows:
    (command, payload)
    Examples are
    (FRAME, update_number, frame_time, mixer) -> data payload to draw a frame
    (QUIT, _) -> quit the server thread

    The server communicates with the control thread over the response queue.
    It requests a frame with
    (FRAME_REQ, _)
    and reports a fatal, thread-death error with
    (FATAL_ERROR, err)
    """
    try:
        socket = create_pub_socket(port)

        # we're ready to render
        response.put((RUNNING, None))

        log_time = monotonic()

        while 1:
            # ready to draw a frame
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

            frame_number, frame_time, mixer, clocks = payload

            # render the payload we received
            video_outs = mixer.draw_layers(clocks)

            for video_chan, draw_commands in enumerate(video_outs):
                serialized = msgpack.dumps(
                    (frame_number, frame_time, draw_commands),
                    use_single_float=True)
                socket.send_multipart((str(video_chan), serialized))

            if report:# and frame_number % 1 == 0:
                now = monotonic()
                log.debug("Framerate: {}".format(1 / (now - log_time)))
                log_time = now

    except Exception as err:
        # some exception we didn't catch
        _, _, tb = sys.exc_info()
        response.put((FATAL_ERROR, (err, traceback.format_tb(tb))))
        return
