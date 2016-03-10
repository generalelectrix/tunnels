from functools import wraps
from weakref import WeakSet

class UserInterface (object):
    """Base class for UIs.  Mostly responsible for implementing Observer.

    Maintains observing controllers using a weak reference set.
    """
    def __init__(self, model):
        """Initialize a user interface to an underlying model object."""
        self.model = model
        self.controllers = WeakSet()
        self._model_properties = set()
        self._properties = set()

    def model_alias(self):
        """Return a safe property alias to the model object.

        Use this for convenience in UIs where aliasing 'model' to some other
        name is convenient, while not having to remember to also reassign the
        alias.  For UIs that will only ever refer to one model, this is not
        necessary.
        """
        return UiModel()

    def swap_model(self, model):
        """Swap in a new model object and reinitialize controllers."""
        self.model = model
        self.initialize()

    def update_controllers(self, method, *args, **kwargs):
        """Call a named method on the controllers."""
        for controller in self.controllers:
            getattr(controller, method)(*args, **kwargs)

    def ui_model_property(self, attribute, callback_name, **kwargs):
        """Create a new UI-observed property into a model object attribute.

        The controllers will NOT be informed of the creation of this property.
        They should be initialized through the use of the initialize method.

        Changing the model that this UI refers to will also take effect in the
        UI properties.

        Args:
            attribute: the underlying model attribute to map this property to
            callback_name: the callback to call on the controllers
            **kwargs: any extra keyword arguments to pass to the callback
        """
        uiprop = UiModelProperty(attribute, callback_name, kwargs)
        self._model_properties.add(uiprop)
        return uiprop

    def ui_property(self, initial_value, callback_name, **kwargs):
        """Create a new UI-observed property.

        The controllers will NOT be informed of the creation of this property.
        They should be initialized through the use of the initialize method.

        Args:
            initial_value: the initial value to set this property to
            callback_name: the callback to call on the controllers
            **kwargs: any extra keyword arguments to pass to the callback
        """
        uiprop = UiProperty(initial_value, callback_name, kwargs)
        self._properties.add(uiprop)
        return uiprop

    def initialize(self):
        """Notify the controllers of the current value of every property.

        Inheriting classes should extend this method to include more complex
        controls besides ui properties.
        """
        for prop in self._model_properties:
            val = getattr(self.model, prop.attribute)
            for controller in self.controllers:
                getattr(controller, prop.callback_name)(val, **prop.kwargs)
        for prop in self._properties:
            val = prop.val
            for controller in self.controllers:
                getattr(controller, prop.callback_name)(val, **prop.kwargs)


class UiModel (object):
    """Convenience descriptor for safely aliasing the model object."""
    def __get__(self, obj, objtype):
        return obj.model

    def __set__(self, obj, new_model):
        obj.swap_model(new_model)


class UiProperty (object):
    """Descriptor for creating observed properties."""
    def __init__(self, initial_value, callback_name, kwargs):
        self.val = initial_value
        self.callback_name = callback_name
        self.kwargs = kwargs
    def __get__(self, obj, objtype):
        return self.val

    def __set__(self, obj, val):
        self.val = val
        for controller in obj.controllers:
            getattr(controller, self.callback_name)(val, **self.kwargs)

class UiModelProperty (object):
    """Descriptor for creating UI-observed properties of model attributes."""
    def __init__(self, attribute, callback_name=None, kwargs=None):
        self.attribute = attribute
        self.callback_name = callback_name if callback_name is not None else attribute
        self.kwargs = {} if kwargs is None else kwargs
    def __get__(self, obj, objtype):
        return getattr(obj.model, self.attribute)

    def __set__(self, obj, val):
        setattr(obj.model, self.attribute, val)
        for controller in obj.controllers:
            getattr(controller, self.callback_name)(val, **self.kwargs)


def ui_method(callback_name, result_filter_func=None, **decoargs):
    """Decorator to make a method act something like a ui property.

    Only use this decorator on methods in classes which subclass UserInterface.

    The wrapped method will be called.  The value it returns will be passed
    through an optional result_filter_func before being passed to the
    observing controllers by calling their callback_name method with the
    filtered result as the first argument, as well as any optional keyword
    arguments passed to this decorator.  The original return value will then be
    returned.
    """
    def ui_method_decorator(method):
        @wraps(method)
        def ui_method_wrapper(self, *args, **kwargs):
            filtered_result = result = method(self, *args, **kwargs)
            if result_filter_func is not None:
                filtered_result = result_filter_func(result)
            for controller in self.controllers:
                getattr(controller, callback_name)(filtered_result, **decoargs)
            return result
        return ui_method_wrapper
    return ui_method_decorator