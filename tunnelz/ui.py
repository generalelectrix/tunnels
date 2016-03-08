class UserInterface (object):
    """Base class for UIs.  Mostly responsible for implementing Observer."""
    def __init__(self):
        self.controllers = set()

    def update_controllers(self, method, *args, **kwargs):
        """Call a named method on the controllers."""
        for controller in self.controllers:
            getattr(controller, method)(*args, **kwargs)