import numpy as np
from .ui import UserInterface

# states for beam matrix UI
Idle = 'idle'
BeamSave = 'beam_save'
LookSave = 'look_save'
Delete = 'delete'
LookEdit = 'look_edit'

# beam button states
ButtonEmpty = 'button_empty'
ButtonBeam = 'button_beam'
ButtonLook = 'button_look'


class BeamMatrixUI (UserInterface):
    """Encapsulate the user interface to a beam matrix.

    The BeamMatrixUI depends on a MixerUI to retrieve and set the currently
    selected beam.
    """

    def __init__(self, beam_matrix, mixer_ui):
        super(BeamMatrixUI, self).__init__()
        self.beam_matrix = beam_matrix
        self.mixer_ui = mixer_ui

        self._state = None
        self.initialize()

    def initialize(self):
        self.state = Idle
        for row in self.beam_matrix.n_rows:
            for col in self.beam_matrix.n_columns:
                self.update_button(row, col, ButtonEmpty)

    @property
    def state(self):
        return self._state

    @state.setter
    def state(self, state):
        """When state is updated, send UI update commands."""
        if self._state is not state:
            self._state = state
            self.update_controllers('set_beam_matrix_state', state)

    def state_toggle(self, state):
        """Toggle state based on an input state command."""
        if self.state is state:
            self.state = Idle
        else:
            self.state = state

    def update_button(self, row, column, state):
        self.update_controllers('set_button_state', row, column, state)

    def grid_button_press(self, row, column):
        """Respond to a grid button press."""
        if self.state is Idle and self.beam_matrix.has_data[row][column]:
            # if idling, get a beam if there is one
            saved_beam = self.beam_matrix.get_element(row, channel)
            self.mixer_ui.replace_current_beam(saved_beam)
        elif self.state is BeamSave:
            # if we're saving a beam, dump it
            beam = self.mixer_ui.get_current_beam()
            self.beam_matrix.put_beam(row, column, beam)
            self.update_button(row, column, ButtonBeam)
            self.state = Idle
        elif self.state is LookSave:
            # dump mixer state into a saved look
            look = self.mixer_ui.get_copy_of_current_look()
            self.beam_matrix.put_look(row, column, look)
            self.update_button(row, column, ButtonLook)
            self.state = Idle
        elif self.state is Delete:
            # empty a button
            self.beam_matrix.clear_element(row, column)
            self.update_button(row, column, ButtonEmpty)
            self.state = Idle
        elif (self.state is LookEdit and
            self.beam_matrix.has_data[row][column] and
            self.beam_matrix.is_look[row][column]):
            # only do anything if there is actually a look in the slot
            look = self.beam_matrix.get_element(row, column)
            self.mixer_ui.set_look(look)
            self.state = Idle


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


    def put_beam(self, row, column, beam):
        """Put a copy of a beam into the minder."""
        self._beams[row][column] = beam.copy()

        self.is_look[row][column] = False
        self.has_data[row][column] = True

    def put_look(self, row, column, look):
        """Copy a look into the beam matrix."""
        self._beams[row][column] = look.copy()
        self.is_look[row][column] = True
        self.has_data[row][column] = True

    def clear_element(self, row, column):
        self._beams[row][column] = None
        self.is_look[row][column] = False
        self.has_data[row][column] = False

    def get_element(self, row, column):
        return self._beams[row][column].copy()

    def element_is_look(self, row, column):
        return self.is_look[row][column]

    def element_has_data(self, row, column):
        return self.has_data[row][column]

