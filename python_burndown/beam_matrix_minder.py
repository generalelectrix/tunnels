import pickle
from datetime import datetime
import logging as log
import numpy as np
from .model_interface import ModelInterface

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


class BeamMatrixMI (ModelInterface):
    """Encapsulate the interface to a beam matrix.

    The BeamMatrixMI depends on a MetaMI to retrieve and set the currently
    selected beam.  It is normally owned by the MetaMI.
    """

    def __init__(self, beam_matrix, meta_mi):
        super(BeamMatrixMI, self).__init__(model=beam_matrix)
        self.beam_matrix = beam_matrix
        self.meta_mi = meta_mi
        self.state = Idle

    def initialize(self):
        super(BeamMatrixMI, self).initialize()
        for row in range(self.beam_matrix.n_rows):
            for col in range(self.beam_matrix.n_columns):
                state = ButtonEmpty
                if self.beam_matrix.element_has_data(row, col):
                    if self.beam_matrix.element_is_look(row, col):
                        state = ButtonLook
                    else:
                        state = ButtonBeam

                self.update_button(row, col, state)

    def state_toggle(self, state):
        """Toggle state based on an input state command."""
        self.set_state(Idle if self.state == state else state)

    def set_state(self, state):
        self.state = state
        self.update_controllers('set_beam_matrix_state', self.state)

    def update_button(self, row, column, state):
        self.update_controllers('set_button_state', row, column, state)

    def grid_button_press(self, row, column):
        """Respond to a grid button press."""
        if self.state == Idle and self.beam_matrix.element_has_data(row, column):
            # if idling, get a beam if there is one
            saved_beam = self.beam_matrix.get_element(row, column)
            self.meta_mi.replace_current_beam(saved_beam)
        elif self.state == BeamSave:
            # if we're saving a beam, dump it
            beam = self.meta_mi.get_current_beam()
            self.beam_matrix.put_beam(row, column, beam)
            self.update_button(row, column, ButtonBeam)
            self.set_state(Idle)
        elif self.state == LookSave:
            # dump mixer state into a saved look
            look = self.meta_mi.get_copy_of_current_look()
            self.beam_matrix.put_look(row, column, look)
            self.update_button(row, column, ButtonLook)
            self.set_state(Idle)
        elif self.state == Delete:
            # empty a button
            self.beam_matrix.clear_element(row, column)
            self.update_button(row, column, ButtonEmpty)
            self.set_state(Idle)
        elif (self.state == LookEdit and
            self.beam_matrix.element_has_data(row, column) and
            self.beam_matrix.element_is_look(row, column)):
            # only do anything if there is actually a look in the slot
            look = self.beam_matrix.get_element(row, column)
            self.meta_mi.set_look(look)
            self.set_state(Idle)


