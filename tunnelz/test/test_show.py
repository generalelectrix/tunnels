"""Top-level integration and smoke tests."""
import os
from nose.tools import assert_equal
from tunnelz import Show, DEFAULT_CONFIG
from tunnelz.show import N_VIDEO_CHANNELS

class TestShow (object):

    test_save_file_path = "tunnel_test_save.test"

    def test_stress_test(self):

        config = DEFAULT_CONFIG.copy()
        config['stress_test'] = True

        s = Show(config, save_path=self.test_save_file_path)

        # test single update step
        s._update_state(20)

        # test rendering
        video_feeds = s.mixer.draw_layers(s.clocks)

        # should have the right number of video channels
        assert_equal(N_VIDEO_CHANNELS, len(video_feeds))

        # channel 0 should have some data in it
        ch0 = video_feeds[0]

        assert ch0
        # each beam should have some draw calls
        for layer in ch0:
            assert layer

    def tearDown(self):
        try:
            os.remove(self.test_save_file_path)
        except OSError as err:
            print "Couldn't delete saved test file:", err
