from .beam_vault import BeamVault
from .LED_control import (
    set_beam_save_LED,
    set_look_save_LED,
    set_delete_LED,
    set_look_edit_LED,
    set_clip_launch_LED,
)
import numpy as np

class BeamMatrixMinder (object):
    """Dealing with the matrix of APC40 buttons used to store beams."""
    n_rows = 5 # using only clip launch
    n_columns = 8 # ignoring master track

    def __init__(self):
        self.is_look = np.zeros((self.n_rows, self.n_columns), bool)
        self.has_data = np.array(self.is_look)

        self.beams = [[None for _ in xrange(self.n_columns)] for _ in xrange(self.n_rows)]

        # update LED state
        for row in xrange(self.n_rows):
            for column in xrange(self.n_columns):
                self.update_LED(row, column)

        self.waiting_for_beam_save = False
        set_beam_save_LED(0)
        self.waiting_for_look_save = False
        set_look_save_LED(0)
        self.waiting_for_delete = False
        set_delete_LED(0)
        self.waiting_for_look_edit = False
        set_look_edit_LED(0)

    def put_beam(self, row, column, beam):
        """Put a copy of a beam into the minder."""
        self.beams[row][column] = BeamVault((beam,))

        self.is_look[row][column] = False
        self.has_data[row][column] = True

        self.update_LED(row, column)

    def put_look(self, row, column, the_look):
        """put a BeamVault into the beam matrix

        assume the BeamVault isn't referenced by anything else
        """
        self.beams[row][column] = the_look
        self.is_look[row][column] = True
        self.has_data[row][column] = True

        self.update_LED(row, column)

    def clear_element(self, row, column):
        self.beams[row][column] = None
        self.is_look[row][column] = False
        self.has_data[row][column] = False

        self.update_LED(row, column)

    def get_element(self, row, column):
        return self.beams[row][column]

    def element_is_look(self, row, column):
        return self.is_look[row][column]

    def element_has_data(self, row, column):
        return self.has_data[row][column]

    def update_LED(self, row, column):
        # if this element has data, turn it on
        if self.element_has_data(row, column):
            if self.element_is_look(row, column):
                # its a look, make it red
                set_clip_launch_LED(row, column, 1, 1)
            else:
                # otherwise, make it orange
                set_clip_launch_LED(row, column, 1, 2)

        # otherwise, turn it off
        else:
            set_clip_launch_LED(row, column, 0, 1)

