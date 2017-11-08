import logging as log
import traceback
from .animation import WaveformType, AnimationTarget, AnimationMI
from .beam_matrix_minder import BeamMatrixMinder
from .meta_mi import MetaMI
from .midi import MidiInput, MidiOutput
from .midi_controllers import (
    BeamMatrixMidiController,
    MetaControlMidiController,
    MixerMidiController,
    TunnelMidiController,
    AnimationMidiController,)
from .mixer import Mixer, MixerMI
from Queue import Empty
from .render_server import RenderServer
from . import sntp_service
from monotonic import monotonic
from .tunnel import Tunnel, TunnelMI
from .shapes import Line

import json

# how many beams you like?
N_BEAMS = 8
# how many virtual video channels should we send?
N_VIDEO_CHANNELS = 8

class Show (object):
    """Encapsulate the show runtime environment."""
    def __init__(self, config_file="show.cfg", load_path=None, save_path=None):
        with open(config_file, 'r') as cfg:
            self.config = config = json.load(cfg)

        if config["log_level"] == "debug":
            log.basicConfig(level=log.DEBUG)
        else:
            log.basicConfig(level=log.INFO)

        self.midi_in = midi_in = MidiInput()
        self.midi_out = midi_out = MidiOutput()

        self.use_midi = config['use_midi']

        self.setup_models(load_path, save_path)

        starting_beam = self.mixer.layers[0].beam

        # instantiate MIs
        self.mixer_mi = mixer_mi = MixerMI(self.mixer)
        self.tunnel_mi = tunnel_mi = TunnelMI(starting_beam)
        self.animator_mi = animator_mi = AnimationMI(starting_beam.get_animation(0))

        # top-level mi
        self.meta_mi = MetaMI(mixer_mi, tunnel_mi, animator_mi, self.beam_matrix)

        # setup all control surfaces
        self.setup_controllers()

        # initialize the MIs
        self.mixer_mi.initialize()
        self.tunnel_mi.initialize()
        self.animator_mi.initialize()
        self.meta_mi.initialize()

        # done!

    def setup_models(self, load_path, save_path):
        """Instantiate all of the model objects."""
        self.mixer = Mixer(n_layers=N_BEAMS, n_video_channels=N_VIDEO_CHANNELS)

        # if we're not using midi, set up test tunnels
        if self.config.get('stress_test', False):
            self.setup_stress_test()
        elif self.config.get('rotation_test', False):
            self.setup_rotation_test()
        elif self.config.get('aliasing_test', False):
            self.setup_aliasing_test()

        # beam matrix minder
        self.beam_matrix = beam_matrix = BeamMatrixMinder(
            load_path=load_path, save_path=save_path)

        # save a copy of the default tunnel for sanity. Don't erase it!
        beam_matrix.put_beam(4, 7, Tunnel())

    def setup_stress_test(self):
        """Set up all mixer tunnels to do everything at once."""
        for i, layer in enumerate(self.mixer.layers):

            # maximally brutal test fixture
            layer.level = 1.0

            tunnel = layer.beam

            tunnel.col_width = 0.25
            tunnel.col_spread = 1.0
            tunnel.col_sat = 0.25

            tunnel.marquee_speed = -1.0 + (2.0 * float(i) / float(N_BEAMS))

            tunnel.blacking = 0

            tunnel.radius = (0.1*i) % 1.0

            for i, anim in enumerate(tunnel.anims):
                anim.type = WaveformType.VALUES[i]
                anim.speed = float(i)/len(tunnel.anims) # various speeds
                anim.weight = 0.5 # finite weight
                anim.target = AnimationTarget.Thickness # hit thickness to do vector math
                anim.n_periods = 3 # more than zero periods for vector math

    def setup_rotation_test(self):
        """Set up one tunnel to test line feature."""
        layer = self.mixer.layers[0]
        layer.level = 1.0
        tunnel = layer.beam
        tunnel.display_as = Line

        tunnel.segs = 40

        tunnel.rot_speed = 0.0
        tunnel.marquee_speed = 0.2

        # xMove = tunnel.anims[0]
        # xMove.speed = 0.2
        # xMove.weight = 0.5
        # xMove.target = AnimationTarget.Size

    def setup_aliasing_test(self):
        """Set up one tunnel to test render smoothness."""
        layer = self.mixer.layers[0]
        layer.level = 1.0
        tunnel = layer.beam

        tunnel.rot_speed = 0.0
        tunnel.marquee_speed = 0.05

    def setup_controllers(self):
        self.setup_midi()

    def setup_midi(self):
        if self.use_midi:
            midi_in = self.midi_in
            midi_out = self.midi_out

            midi_ports = self.config['midi_ports']
            for port in midi_ports:
                midi_in.open_port(port)
                midi_out.open_port(port)

            self.metacontrol_midi_controller = MetaControlMidiController(
                self.meta_mi, midi_in, midi_out)

            self.bm_midi_controller = BeamMatrixMidiController(
                self.meta_mi.beam_matrix_mi, midi_in, midi_out)

            self.mixer_midi_controller = MixerMidiController(
                self.mixer_mi, midi_in, midi_out)

            self.tunnel_midi_controller = TunnelMidiController(
                self.tunnel_mi, midi_in, midi_out)

            self.animation_midi_controller = AnimationMidiController(
                self.animator_mi, midi_in, midi_out)

    def run(self, update_interval=20, n_frames=None, control_timeout=0.001):
        """Run the show loop.

        Args:
            update_interval (int): number of milliseconds between beam state updates
            n_frames (None or int): if None, run forever.  if finite number, only
                run for this many state updates.
        """

        report_framerate = self.config["report_framerate"]

        update_number = 0

        # start sntp time synchronization service
        # FIXME no clean quit mechanism!
        log.info("Starting SNTP time service.")
        sntp_service.run_service()
        log.info("SNTP time service started.")

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
                # process a control event if one is pending
                try:
                    self.midi_in.receive(timeout=control_timeout)
                except Empty:
                    # fine if we didn't get a control event
                    pass
                except Exception as e:
                    # trap any exception here and log an error to avoid crashing
                    # the whole controller
                    log.error(
                        "An error occurred while processing a midi control "
                        "event:\n{}\n{}".format(e, traceback.format_exc()))

                # compute updates until we're current
                now = time_millis()
                time_since_last_update = now - last_update

                while time_since_last_update > update_interval:
                    # update the state of the beams
                    for layer in self.mixer.layers:
                        layer.beam.update_state(update_interval)

                    last_update += update_interval
                    now = time_millis()
                    time_since_last_update = now - last_update
                    update_number += 1


                # pass the mixer to the render process if it is ready to draw
                # another frame and it hasn't drawn this frame yet
                if update_number > last_rendered_frame:
                    rendered = render_server.pass_frame_if_ready(
                        update_number, last_update, self.mixer)
                    if rendered:
                        last_rendered_frame = update_number

        finally:
            render_server.stop()
            log.info("Shut down render server.")


