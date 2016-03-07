from .LED_control import (
    set_beam_save_LED,
    set_look_save_LED,
    set_delete_LED,
    set_look_edit_LED,
    set_clip_launch_LED,
)
import numpy as np

# states for beam matrix UI
Idle, WaitForBeamSave, WaitForLookSave, WaitForDelete, WaitForLookEdit = xrange(5)

class BeamMatrixUI (object):
    """Encapsulate the user interface to a beam matrix.

    The BeamMatrixUI depends on a MixerUI to retrieve and set the currently
    selected beam.
    """

    def __init__(self, beam_matrix, mixer_ui):
        self.beam_matrix = beam_matrix
        self.mixer_ui = mixer_ui
        self.controllers = set()

        self._state = Idle

    @property
    def state(self):
        return self._state

    @state.setter
    def state(self, state):
        """When state is updated, send UI update commands."""
        self._state = state
        for controller in self.controllers:
            controller.set_beam_matrix_state(state)

    def state_toggle(self, state):
        """Toggle state based on an input state command."""
        if self.state == state:
            self.state = Idle
        else:
            self.state = state

    def grid_button_press(self, row, column):
        """Respond to a grid button press."""
        if self.state == Idle and self.beam_matrix.has_data[row][column]:
            # if idling, get a beam if there is one
            saved_beam = self.beam_matrix.get_element(row, channel)
            self.mixer_ui.replace_current_beam(saved_beam)



class BeamMatrixMinder (object):
    """Dealing with the matrix of APC40 buttons used to store beams.

    The contents of the matrix are immutable; putting and getting beams will
    automatically perform deep copying to ensure that beams can only be swapped
    out.
    """
    n_rows = 5 # using only clip launch
    n_columns = 8 # ignoring master track

    def __init__(self):
        self.is_look = np.zeros((self.n_rows, self.n_columns), bool)
        self.has_data = np.array(self.is_look)

        self._beams = [[None for _ in xrange(self.n_columns)] for _ in xrange(self.n_rows)]

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
        self._beams[row][column] = beam.copy()

        self.is_look[row][column] = False
        self.has_data[row][column] = True

        self.update_LED(row, column)

    def put_look(self, row, column, look):
        """Copy a look into the beam matrix."""
        self._beams[row][column] = look.copy()
        self.is_look[row][column] = True
        self.has_data[row][column] = True

        self.update_LED(row, column)

    def clear_element(self, row, column):
        self._beams[row][column] = None
        self.is_look[row][column] = False
        self.has_data[row][column] = False

        self.update_LED(row, column)

    def get_element(self, row, column):
        return self._beams[row][column].copy()

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

