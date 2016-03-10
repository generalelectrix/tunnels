from nose.tools import assert_equal, assert_in
from tunnelz.ui import UserInterface

UIP0DEF = 'ui prop 0 default'
UIP1DEF = 'ui prop 1 default'

class TestUI (UserInterface):

    def __init__(self, model):
        super(TestUI, self).__init__(model)
        self.modelprop0 = self.ui_model_property('modelprop0', 'callback0')
        self.modelprop1 = self.ui_model_property('modelprop1', 'callback1', extra_arg='test_kw0')

        self.uiprop0 = self.ui_property(UIP0DEF, 'uicallback0')
        self.uiprop1 = self.ui_property(UIP1DEF, 'uicallback1', extra_arg='test_kw1')

TP0DEF = 'test prop 0 default'
TP1DEF = 'test prop 1 default'

class TestModel (object):

    def __init__(self):
        self.modelprop0 = TP0DEF
        self.modelprop1 = TP1DEF


class TestController (object):

    def __init__(self, ui):
        self.ui = ui
        ui.controllers.add(self)
        self.cb_results = []

    def callback0(self, val):
        self.cb_results.append(('cb0', val))

    def callback1(self, val, extra_arg):
        self.cb_results.append(('cb1', val, extra_arg))

    def uicallback0(self, val):
        self.cb_results.append(('uicb0', val))

    def uicallback1(self, val, extra_arg):
        self.cb_results.append(('uicb1', val, extra_arg))


def test_basic_properties():
    model = TestModel()
    ui = TestUI(model)
    cont = TestController(ui)

    ui.initialize()
    assert_in(('uicb1', UIP1DEF, 'test_kw1'), cont.cb_results)
    assert_in(('uicb0', UIP0DEF), cont.cb_results)
    assert_in(('cb1', TP1DEF, 'test_kw0'), cont.cb_results)
    assert_in(('cb0', TP0DEF), cont.cb_results)

    cont.cb_results = []

    ui.modelprop0 = 'changed mp0'
    print ui.modelprop0
    assert_equal(model.modelprop0, 'changed mp0')
    assert_equal(cont.cb_results.pop(), ('cb0', 'changed mp0'))
    assert not cont.cb_results
