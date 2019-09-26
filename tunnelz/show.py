import logging as log
import traceback
from queue import Queue, Empty
from .animation import WaveformType, AnimationTarget, AnimationMI
from .beam_matrix_minder import BeamMatrixMinder
from .clock import ControllableClock
from .devices import initialize_device
from .meta_mi import MetaMI
from .midi import MidiInput, MidiOutput, list_ports
from .midi_controllers import (
    BeamMatrixMidiController,
    MetaControlMidiController,
    MixerMidiController,
    TunnelMidiController,
    AnimationMidiController,
    ClockMidiController,
)
from .mixer import Mixer, MixerMI
from .render_server import RenderServer
from . import timesync
from time import monotonic
from .tunnel import Tunnel, TunnelMI

import yaml

# how many virtual video channels should we send?
N_VIDEO_CHANNELS = 8

# how many globally-available clocks?
N_CLOCKS = 8

# default configuration parameters
DEFAULT_CONFIG = dict(
    use_midi=False,
    midi_ports=[],
    report_framerate=False,
    log_level="debug",
    stress_test=False,
    rotation_test=False,
    aliasing_test=False,
    multi_channel_test=False,
)


class Show (object):
    """Top-level entity that owns all show resources including the event loop."""
    test_mode = False

    @classmethod
    def from_config(cls, config_file_path="show.yaml"):
        """Create a fresh show using the provided config file path."""
        with open(config_file_path, 'r') as cfg:
            config = yaml.load(cfg)

        return cls(config)

    @classmethod
    def from_prompt(cls):
        """Create a new show by prompting for input at the command line."""
        config = prompt_for_config()
        return cls(config)

    def __init__(self, config, load_path=None, save_path=None):
        """Create a Tunnel show.

        Args:
            config: dict containing configuration data
            load_path (optional): path to saved show file to load
            save_path (optional): path to use to save show file state
        """
        self.config = config

        if config["log_level"] == "debug":
            log.basicConfig(level=log.DEBUG)
        else:
            log.basicConfig(level=log.INFO)

        # keep a queue of requests from control input handlers to be serviced.
        self.control_requests = Queue()

        self.channel_count = config.get('channel_count', 16)

        self._setup_models(load_path, save_path)

        starting_beam = self.mixer.layers[0].beam

        # instantiate MIs
        self.mixer_mi = mixer_mi = MixerMI(self.mixer)
        self.tunnel_mi = tunnel_mi = TunnelMI(starting_beam)
        self.animator_mi = animator_mi = AnimationMI(starting_beam.get_animation(0))

        # top-level mi
        self.meta_mi = MetaMI(mixer_mi, tunnel_mi, animator_mi, self.beam_matrix)

        # setup all control surfaces
        if config['use_midi']:
            self._setup_midi()

        # initialize the MIs
        self.mixer_mi.initialize()
        self.tunnel_mi.initialize()
        self.animator_mi.initialize()
        self.meta_mi.initialize()
        for clock in self.clocks:
            clock.initialize()

        # potentially set up some test patterns if they were requested
        self._setup_test_pattern()

        # show is ready to run

    def run(self, update_interval=16, n_frames=None):
        """Run the show loop.

        Args:
            update_interval (int): number of milliseconds between beam state updates
            n_frames (None or int): if None, run forever.  if finite number, only
                run for this many state updates.
        """
        report_framerate = self.config.get("report_framerate", False)

        update_number = 0

        # start time synchronization service
        # FIXME no clean quit mechanism!
        log.info("Starting time synchronization service...")
        timesync.run_service()
        log.info("Time synchronization service started.")

        # start up the render server
        render_server = RenderServer(report=report_framerate)

        log.info("Starting render server...")
        render_server.start()
        log.info("Render server started.")

        time_millis = lambda: int(monotonic()*1000)

        last_update = time_millis()

        last_rendered_frame = -1

        try:
            while n_frames is None or update_number < n_frames:
                try:
                    # compute updates until we're current
                    now = time_millis()
                    time_since_last_update = now - last_update

                    while time_since_last_update > update_interval:
                        self._update_state(update_interval)

                        last_update += update_interval
                        now = time_millis()
                        time_since_last_update = now - last_update
                        update_number += 1

                    # pass the mixer to the render process if it is ready to draw
                    # another frame and it hasn't drawn this frame yet
                    if update_number > last_rendered_frame:
                        rendered = render_server.pass_frame_if_ready(
                            update_number, last_update, self.mixer, self.clocks)
                        if rendered:
                            last_rendered_frame = update_number

                    # process a control event for a fraction of the time between now
                    # and when we will need to update state again
                    now = time_millis()

                    time_to_next_update = last_update + update_interval - now

                    if time_to_next_update > 0:
                        # timeout arg is a float in seconds
                        # only use, say, 80% of the time we have to prioritize
                        # timely state updates
                        timeout = 0.8 * time_to_next_update / 1000.
                        self._service_control_event(timeout)
                except Exception:
                    if self.test_mode:
                        raise
                    # trap any exception here and log an error to avoid crashing
                    # the whole controller
                    log.exception("Exception in main show loop.")

        finally:
            render_server.stop()
            log.info("Shut down render server.")

    def _setup_models(self, load_path, save_path):
        """Instantiate all of the model objects."""
        self.mixer = Mixer(
            n_layers=self.channel_count,
            n_video_channels=N_VIDEO_CHANNELS,
            test_mode=self.test_mode)

        self.clocks = [ControllableClock() for _ in range(N_CLOCKS)]

        # beam matrix minder
        # FIXME: hardcoded page count
        self.beam_matrix = beam_matrix = BeamMatrixMinder(
            n_pages=2,
            load_path=load_path,
            save_path=save_path)

        # save a copy of the default tunnel for sanity. Don't erase it!
        beam_matrix.put_beam(4, 7, Tunnel())

    def _setup_midi(self):
        """Configure each requested midi port."""
        self.midi_inputs, self.midi_outputs = [], []

        midi_ports = self.config['midi_ports']
        # FIXME: #17 if a midi device on the bus anywhere has a different number of
        # inputs and outputs we need to account for that.
        for port in midi_ports:
            midi_in = MidiInput(port, self.control_requests)
            midi_out = MidiOutput(port)
            self.midi_inputs.append(midi_in)
            self.midi_outputs.append(midi_out)

            # perform device-specific init
            initialize_device(midi_out)

            # now attach all of the relevant controllers
            def create_controller(cls, mi, **kwargs):
                controller = cls(mi, midi_out, **kwargs)
                midi_in.register_controller(controller)

            # FIXME: shitty hack to use the APC20 as a wing.
            if midi_in.name == "Akai APC20":
                page = 1
            else:
                page = 0

            # FIXME: this is a terrible way to decide which controllers to
            # hook up to which control surfaces.
            if midi_in.name == "ReMOTE SL Port 1" or "Network Session" in midi_in.name:
                for i, clock in enumerate(self.clocks):
                    create_controller(ClockMidiController, clock, channel=i)

            if "Akai APC" in midi_in.name or "Network Session" in midi_in.name:
                create_controller(MetaControlMidiController, self.meta_mi, page=page)
                create_controller(BeamMatrixMidiController, self.meta_mi.beam_matrix_mi, page=page)
                create_controller(MixerMidiController, self.mixer_mi, page=page)
                create_controller(TunnelMidiController, self.tunnel_mi)
                create_controller(AnimationMidiController, self.animator_mi)

    def _service_control_event(self, timeout):
        """Service a single control event if one is pending."""
        try:
            control_request_ref = self.control_requests.get(True, timeout)
        except Empty:
            # no request pending
            return

        # control sources come in as weak references, only continue if the
        # reference is still live
        control_request = control_request_ref()
        if control_request is None:
            log.debug("Got a dead control request reference.")
            return

        # service the request by calling its handle method
        control_request.handle_message()

    def _update_state(self, update_interval):
        """Perform discrete state update on every part of the show."""
        # update the state of the global clocks
        for clock in self.clocks:
            clock.update_state(update_interval)

        # update the state of the beams
        self.mixer.update_state(update_interval, self.clocks)

    def _setup_test_pattern(self):
        """Set up a test pattern if set in the config."""
        if self.config.get('stress_test', False):
            self.setup_stress_test()
        elif self.config.get('rotation_test', False):
            self.setup_rotation_test()
        elif self.config.get('aliasing_test', False):
            self.setup_aliasing_test()
        elif self.config.get('multi_channel_test', False):
            self.setup_multi_channel_test()

    def setup_stress_test(self):
        """Set up all mixer tunnels to do everything at once."""
        for i, layer in enumerate(self.mixer.layers):

            # maximally brutal test fixture
            layer.level = 1.0

            tunnel = layer.beam

            tunnel.col_width = 0.25
            tunnel.col_spread = 1.0
            tunnel.col_sat = 0.25

            tunnel.marquee_speed = -1.0 + (2.0 * float(i) / float(self.channel_count))

            tunnel.blacking = 0.0

            tunnel.radius = (0.1*i) % 1.0

            for i, anim in enumerate(tunnel.anims):
                anim.type = WaveformType.VALUES[i]
                # various animation speeds
                anim.internal_clock.rate = AnimationMI.max_clock_rate * float(i) / len(tunnel.anims)
                anim.weight = 0.5 # finite weight
                anim.target = AnimationTarget.Thickness # hit thickness to do vector math
                anim.n_periods = 3 # more than zero periods for vector math

    def setup_rotation_test(self):
        """Set up one tunnel to test basic rotation."""
        layer = self.mixer.layers[0]
        layer.level = 1.0
        tunnel = layer.beam

        tunnel.segs = 40
        tunnel.aspect_ratio = 0.5

        tunnel.rot_speed = 0.2
        tunnel.marquee_speed = 0.0

    def setup_aliasing_test(self):
        """Set up one tunnel to test render smoothness."""
        layer = self.mixer.layers[0]
        layer.level = 1.0
        tunnel = layer.beam

        tunnel.rot_speed = 0.0
        tunnel.marquee_speed = 1.0

    def setup_multi_channel_test(self):
        """Set up eight unique tunnels, one per video output."""
        for i, layer in enumerate(self.mixer.layers):

            layer.level = 1.0
            layer.video_outs = {i % N_VIDEO_CHANNELS}

            tunnel = layer.beam
            tunnel.col_sat = 1.0

            tunnel.marquee_speed = 0.1

            tunnel.col_center = (float(i) / N_VIDEO_CHANNELS) % 1.0


# --- prompt the user for input to configure a show interactively ---

def prompt_for_config():
    """Prompt the user for input to assemble a show configuration.

    Return the configuration as a dict.
    """
    config = {}
    use_midi = prompt_bool("Use midi?")
    if use_midi:
        midi_ports = prompt_for_midi()
    else:
        midi_ports = []

    config['midi_ports'] = midi_ports
    config['use_midi'] = use_midi
    config['log_level'] = "info"

    return config

def prompt_for_midi():
    """Prompt the user to select one or more midi ports."""
    ports = []
    while prompt_bool("Add a midi port?"):
        inputs, outputs = list_ports()
        print(inputs)
        print(outputs)
        port = prompt_int("Select a port:")
        ports.append(port)
    return ports

def prompt_int(msg):
    """Prompt the user and parse input as an int."""
    while True:
        user_input = input(msg)
        try:
            return int(user_input)
        except ValueError:
            print("Please enter an integer.")

def prompt_bool(msg):
    """Prompt the user to answer a yes or no question."""
    while True:
        user_input = input("{} y/n:".format(msg)).lower()
        if len(user_input) == 0:
            continue

        c = user_input[0]
        if c == 'y':
            return True
        elif c == 'n':
            return False

