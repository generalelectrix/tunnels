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

        if config["log_level"] == "debug":
            log.basicConfig(level=log.DEBUG)
        else:
            log.basicConfig(level=log.INFO)


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

