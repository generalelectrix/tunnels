import numpy as np
from .model_interface import ModelInterface, MiProperty
import cPickle as pickle
import logging as log
from uuid import uuid1

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
        for row in xrange(self.beam_matrix.n_rows):
            for col in xrange(self.beam_matrix.n_columns):
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


class BeamMatrixMinder (object):
    """Dealing with the matrix of APC40 buttons used to store beams.

    The contents of the matrix are immutable; putting and getting beams will
    automatically perform deep copying to ensure that beams can only be swapped
    out.
    """

    n_rows = 5
    col_per_page = 8

    def __init__(self, n_pages, load_path=None, save_path=None):
        """Create a new minder, backed by a file path.

        The minder will keep the cached version on disk in sync.
        If load_path is provided, the state of this minder will be filled by loading from
        the saved file on disk.  If no load path is provided, nothing will be loaded,
        but a save file will be created using a uuid.  This file can be overridden
        by passing save_path.
        """
        self.n_columns = n_pages * self.col_per_page
        self._cache_path = (
                save_path if save_path is not None
                else "tunnelz_save_{}.tunnel".format(uuid1()))

        if load_path is None:
            self._is_look = np.zeros((self.n_rows, self.n_columns), bool)
            self._beams = [[None for _ in xrange(self.n_columns)] for _ in xrange(self.n_rows)]
        else:
            self._beams, self._is_look = self._load_from_disk(load_path)

        self._save_to_disk()

    def _save_to_disk(self):
        """Pickle the contents of this minder to a file on disk."""
        # since we periodically save to disk, don't want to crash the show if this fails
        try:
            with open(self._cache_path, 'w+') as f:
                pickle.dump((self._beams, self._is_look), f, pickle.HIGHEST_PROTOCOL)
        except Exception as err:
            log.error("An error occurred while saving beam matrix to disk: {}", err)

    def _load_from_disk(self, path):
        """Unpickle a saved file and return its contents."""
        # no exception handling here because this should only happen on startup
        # and we should bail completely if it fails.
        with open(path, 'r') as f:
            beams, is_look = pickle.load(f)
            return beams, is_look

    def put_beam(self, row, column, beam):
        """Put a copy of a beam into the minder."""
        self._beams[row][column] = beam.copy()

        self._is_look[row][column] = False

        self._save_to_disk()

    def put_look(self, row, column, look):
        """Copy a look into the beam matrix."""
        self._beams[row][column] = look.copy()
        self._is_look[row][column] = True

        self._save_to_disk()

    def clear_element(self, row, column):
        self._beams[row][column] = None
        self._is_look[row][column] = False

        self._save_to_disk()

    def get_element(self, row, column):
        return self._beams[row][column].copy()

    def element_is_look(self, row, column):
        return self._is_look[row][column]

    def element_has_data(self, row, column):
        return self._beams[row][column] is not None

