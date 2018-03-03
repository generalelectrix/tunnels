"""Top-level integration and smoke tests."""
import os
from nose.tools import assert_equal
from tunnelz import Show, DEFAULT_CONFIG
from tunnelz.show import N_VIDEO_CHANNELS

def layer_checksum(layer):
    return sum(val for draw_call in layer for val in draw_call)

class TestShow (object):

    test_save_file_path = "tunnel_test_save.test"

    def setUp(self):
        """Set the Show class to run in test mode."""
        Show.test_mode = True

    def test_stress_test(self):

        config = DEFAULT_CONFIG.copy()
        config['stress_test'] = True

        s = Show(config, save_path=self.test_save_file_path)

        def check_render(layer_checksums):
            """Some basic checks on the result of drawing layers."""
            # test rendering
            video_feeds = s.mixer.draw_layers(s.clocks)

            # rendering should be idempotent
            assert_equal(video_feeds, s.mixer.draw_layers(s.clocks))

            # should have the right number of video channels
            assert_equal(N_VIDEO_CHANNELS, len(video_feeds))

            # channel 0 should have some data in it
            ch0 = video_feeds[0]

            assert ch0
            # each beam should have some draw calls
            for layer in ch0:
                assert layer

            # checksum on the layers to catch generic unexpected changes
            # this may turn out to be platform-dependent as it is quite crude
            for i, layer in enumerate(ch0):
                assert_equal(layer_checksums[i], layer_checksum(layer))

        check_render(self.layer_checksums_fr0)

        # test single update step
        s._update_state(20)

        check_render(self.layer_checksums_fr1)

    def tearDown(self):
        try:
            os.remove(self.test_save_file_path)
        except OSError as err:
            print "Couldn't delete saved test file:", err

    # before update, every beam should be identical as we haven't let them
    # evolve at all yet
    layer_checksums_fr0 = [624.97270847568575] * 16

    # layer checksums for frame 1
    layer_checksums_fr1 = [
        625.78766366950549,
        626.6027261695067,
        625.30911366950704,
        625.90682616950699,
        626.39586366950641,
        626.77622616950839,
        627.04791366950667,
        627.21092616950716,
        625.26526366950759,
        625.31960116950847,
        625.48261366950692,
        625.75430116950668,
        626.13466366950649,
        626.6237011695091,
        627.22141366950882,
        625.92780116950814]
