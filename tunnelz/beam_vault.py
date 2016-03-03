
class BeamVault (object):
    """Storing and retrieving deep copies of beams.

    Beams stored in a vault are only referenced by the Vault itself.
    """

    def __init__(self, beams):
        """Initialize with some set of beams."""
        self.beams = tuple(beam.copy() for beam in beams)

    def __iter__(self):
        return iter(self.beams)

    def __len__(self):
        return len(self.beams)

    def copy(self):
        return BeamVault(self)

    def retrieve_copy(self, index):
        return self.beams[index].copy()