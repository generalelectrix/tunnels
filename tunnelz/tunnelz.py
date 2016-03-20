import logging as log
from .animation import WaveformType, AnimationTarget, AnimationMI
from .beam_matrix_minder import BeamMatrixMinder
from itertools import count
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
import time
from .tunnel import Tunnel, TunnelMI

# how many beams you like?
N_BEAMS = 8

class Show (object):
    """Encapsulate the show runtime environment."""
    def __init__(self, use_midi=True):
        log.basicConfig(level=log.DEBUG)

        self.midi_in = midi_in = MidiInput()
        self.midi_out = midi_out = MidiOutput()

        self.use_midi = use_midi

        self.setup_models()

        starting_beam = self.mixer.layers[0].beam

        # instantiate MIs
        self.mixer_mi = mixer_mi = MixerMI(self.mixer)
        self.tunnel_mi = tunnel_mi = TunnelMI(starting_beam)
        self.animator_mi = animator_mi = AnimationMI(starting_beam.get_current_animation())

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

    def setup_models(self):
        """Instantiate all of the model objects."""
        self.mixer = mixer = Mixer(N_BEAMS)

        # if we're not using midi, set up test tunnels
        if not self.use_midi:
            for i, layer in enumerate(mixer.layers):

                # maximally brutal test fixture
                layer.level = 255

                tunnel = layer.beam

                tunnel.col_width = 0.25
                tunnel.col_spread = 1.0
                tunnel.col_sat = 0.25

                tunnel.rot_speedI = float(i / N_BEAMS)

                tunnel.blacking = 0

                for i, anim in enumerate(tunnel.anims):
                    anim.type = WaveformType.VALUES[i]
                    anim.speed = float(i)/len(tunnel.anims) # various speeds
                    anim.weight = 64 # finite weight
                    anim.target = AnimationTarget.Thickness # hit thickness to do vector math
                    anim.n_periods = 3 # more than zero periods for vector math

        # beam matrix minder
        self.beam_matrix = beam_matrix = BeamMatrixMinder()

        # save a copy of the default tunnel for sanity. Don't erase it!
        beam_matrix.put_beam(4, 7, Tunnel())

    def setup_controllers(self):
        self.setup_midi()

    def setup_midi(self):
        if self.use_midi:
            midi_in = self.midi_in
            midi_out = self.midi_out

            #FIXME-midi port configuration
            midi_in.open_port(2)
            midi_out.open_port(2)

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

    def run(self, framerate=30.0, n_frames=None, verbose=False):

        render_period = 1.0 / framerate
        last = time.time()
        friter = count() if n_frames is None else xrange(n_frames)
        render_dt = 0.0
        for framenumber in friter:
            self.process_control_events_until_render(render_period - render_dt, verbose=False)
            start_render = time.time()
            self.draw()
            end_render = time.time()
            render_dt = end_render - start_render
            if (framenumber + 1) % 30 == 0:
                log.info("{} fps".format(30 / (end_render - last)))
                last = end_render

    def process_control_events_until_render(self, time_left, verbose=False):
        start = time.time()
        events_processed = 0
        midi_in = self.midi_in
        while True:
            time_until_render = time_left - (time.time() - start)
            # if it is time to render, stop the command loop
            if time_until_render <= 0.0:
                break

            # process control events
            try:
                # time out slightly before render time to improve framerate stability
                midi_in.receive(timeout=time_until_render*0.95)
                events_processed += 1
            except Empty:
                # fine if we didn't get a control event
                pass

        if verbose:
            log.debug("{} events/sec".format(events_processed / time_left))

    # method called whenever processing draws a frame, basically the event loop
    def draw(self, socket=True, write=False, print_=False):

        # black out everything to remove leftover pixels
        # FIXME-RENDERING
        # background(0)

        dc_agg = self.mixer.draw_layers()
        if print_:
            print dc_agg
        if write:
            file = 'layer0.csv'
            dc_agg.write_to_file(file)
        if socket:
            dc_agg.write_to_socket()

