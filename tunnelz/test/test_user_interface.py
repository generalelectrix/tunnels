from nose.tools import assert_equal, assert_in
from tunnelz.ui import UserInterface, UiProperty, UiModelProperty

UIP0DEF = 'ui prop 0 default'
UIP1DEF = 'ui prop 1 default'

class TestUI (UserInterface):

    modelprop0 = UiModelProperty('modelprop0', 'callback0')
    modelprop1 = UiModelProperty('modelprop1', 'callback1', extra_arg='test_mp1exarg')

    uiprop0 = UiProperty(UIP0DEF, 'uicallback0')
    uiprop1 = UiProperty(UIP1DEF, 'uicallback1', extra_arg='test_uip1exarg')

    def __init__(self, model):
        super(TestUI, self).__init__(model)

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
    assert_in(('uicb1', UIP1DEF, 'test_uip1exarg'), cont.cb_results)
    assert_in(('uicb0', UIP0DEF), cont.cb_results)
    assert_in(('cb1', TP1DEF, 'test_mp1exarg'), cont.cb_results)
    assert_in(('cb0', TP0DEF), cont.cb_results)
    assert_equal(len(cont.cb_results), 4)

    cont.cb_results = []

    ui.modelprop0 = 'changed mp0'
    assert_equal(model.modelprop0, 'changed mp0')
    assert_equal(ui.modelprop0, 'changed mp0')
    assert_equal(cont.cb_results.pop(), ('cb0', 'changed mp0'))
    assert not cont.cb_results

    ui.modelprop1 = 'changed mp1'
    assert_equal(model.modelprop1, 'changed mp1')
    assert_equal(ui.modelprop1, 'changed mp1')
    assert_equal(cont.cb_results.pop(), ('cb1', 'changed mp1', 'test_mp1exarg'))
    assert not cont.cb_results

    ui.uiprop0 = 'changed uip0'
    assert_equal(ui.uiprop0, 'changed uip0')
    assert_equal(cont.cb_results.pop(), ('uicb0', 'changed uip0'))
    assert not cont.cb_results

    ui.uiprop1 = 'changed uip1'
    assert_equal(ui.uiprop1, 'changed uip1')
    assert_equal(cont.cb_results.pop(), ('uicb1', 'changed uip1', 'test_uip1exarg'))
    assert not cont.cb_results

    # now test multiple instances of connected to multiple models
    model1 = TestModel()
    ui1 = TestUI(model1)
    cont1 = TestController(ui1)

    ui1.initialize()
    assert_in(('uicb1', UIP1DEF, 'test_uip1exarg'), cont1.cb_results)
    assert_in(('uicb0', UIP0DEF), cont1.cb_results)
    assert_in(('cb1', TP1DEF, 'test_mp1exarg'), cont1.cb_results)
    assert_in(('cb0', TP0DEF), cont1.cb_results)
    assert_equal(len(cont1.cb_results), 4)

    cont1.cb_results = []

    # check to make sure our original UI and model is untouched
    assert_equal(model.modelprop0, 'changed mp0')
    assert_equal(ui.modelprop0, 'changed mp0')
    assert_equal(model.modelprop1, 'changed mp1')
    assert_equal(ui.modelprop1, 'changed mp1')
    assert_equal(ui.uiprop0, 'changed uip0')
    assert_equal(ui.uiprop1, 'changed uip1')