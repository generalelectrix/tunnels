import numpy as np
from .ui import UserInterface, UiProperty

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

    The BeamMatrixUI depends on a MetaUI to retrieve and set the currently
    selected beam.  It is normally owned by the MetaUI.
    """
    state = UiProperty(Idle, 'set_beam_matrix_state')

    def __init__(self, beam_matrix, meta_ui):
        super(BeamMatrixUI, self).__init__(model=beam_matrix)
        self.beam_matrix = beam_matrix
        self.meta_ui = meta_ui

    def initialize(self):
        super(BeamMatrixUI, self).initialize()
        for row in xrange(self.beam_matrix.n_rows):
            for col in xrange(self.beam_matrix.n_columns):
                self.update_button(row, col, ButtonEmpty)

    def state_toggle(self, state):
        """Toggle state based on an input state command."""
        if self.state == state:
            self.state = Idle
        else:
            self.state = state

    def update_button(self, row, column, state):
        self.update_controllers('set_button_state', row, column, state)

    def grid_button_press(self, row, column):
        """Respond to a grid button press."""
        if self.state == Idle and self.beam_matrix.element_has_data(row, column):
            # if idling, get a beam if there is one
            saved_beam = self.beam_matrix.get_element(row, column)
            self.meta_ui.replace_current_beam(saved_beam)
        elif self.state == BeamSave:
            # if we're saving a beam, dump it
            beam = self.meta_ui.get_current_beam()
            self.beam_matrix.put_beam(row, column, beam)
            self.update_button(row, column, ButtonBeam)
            self.state = Idle
        elif self.state == LookSave:
            # dump mixer state into a saved look
            look = self.meta_ui.get_copy_of_current_look()
            self.beam_matrix.put_look(row, column, look)
            self.update_button(row, column, ButtonLook)
            self.state = Idle
        elif self.state == Delete:
            # empty a button
            self.beam_matrix.clear_element(row, column)
            self.update_button(row, column, ButtonEmpty)
            self.state = Idle
        elif (self.state == LookEdit and
            self.beam_matrix.element_has_data(row, column) and
            self.beam_matrix.element_is_look(row, column)):
            # only do anything if there is actually a look in the slot
            look = self.beam_matrix.get_element(row, column)
            self.meta_ui.set_look(look)
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
        self._is_look = np.zeros((self.n_rows, self.n_columns), bool)

        self._beams = [[None for _ in xrange(self.n_columns)] for _ in xrange(self.n_rows)]


    def put_beam(self, row, column, beam):
        """Put a copy of a beam into the minder."""
        self._beams[row][column] = beam.copy()

        self._is_look[row][column] = False

    def put_look(self, row, column, look):
        """Copy a look into the beam matrix."""
        self._beams[row][column] = look.copy()
        self._is_look[row][column] = True

    def clear_element(self, row, column):
        self._beams[row][column] = None
        self._is_look[row][column] = False

    def get_element(self, row, column):
        return self._beams[row][column].copy()

    def element_is_look(self, row, column):
        return self._is_look[row][column]

    def element_has_data(self, row, column):
        return self._beams[row][column] is not None

