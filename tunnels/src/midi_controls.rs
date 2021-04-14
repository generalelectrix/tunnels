mod animation;
mod tunnel;

use std::{collections::HashMap, time::Duration};

use crate::{
    device::Device,
    midi::{Event, EventType, Manager, Mapping},
    numbers::{BipolarFloat, UnipolarFloat},
    show::ControlMessage,
    show::StateChange,
    ui::EmitStateChange,
};

use self::animation::{map_animation_controls, update_animation_control};
use self::tunnel::{map_tunnel_controls, update_tunnel_control};

type ControlMessageCreator = fn(u8) -> ControlMessage;

type ControlMap = HashMap<(Device, Mapping), ControlMessageCreator>;
pub struct Dispatcher {
    map: ControlMap,
    manager: Manager,
}

impl Dispatcher {
    /// Instantiate the master midi control dispatcher.
    pub fn new(manager: Manager) -> Self {
        let mut map = HashMap::new();
        map_tunnel_controls(Device::AkaiApc40, &mut map);
        map_tunnel_controls(Device::TouchOsc, &mut map);
        Self { map, manager }
    }

    pub fn receive(&self, timeout: Duration) -> Option<(Device, Event)> {
        self.manager.receive(timeout)
    }

    /// Map a midi source device and event into a tunnels control message.
    /// Return None if no mapping is registered.
    pub fn dispatch(&self, device: Device, event: Event) -> Option<ControlMessage> {
        self.map
            .get(&(device, event.mapping))
            .map(|c| c(event.value))
    }
}

impl EmitStateChange for Dispatcher {
    /// Map application state changes into UI update midi messages.
    fn emit(&mut self, sc: StateChange) {
        match sc {
            StateChange::Tunnel(sc) => update_tunnel_control(sc, &mut self.manager),
            StateChange::Animation(sc) => update_animation_control(sc, &mut self.manager),
        }
    }
}

fn add_control(
    map: &mut ControlMap,
    device: Device,
    mapping: Mapping,
    creator: ControlMessageCreator,
) {
    if map.insert((device, mapping), creator).is_some() {
        panic!("duplicate control definition: {:?} {:?}", device, mapping);
    }
}

fn bipolar_from_midi(val: u8) -> BipolarFloat {
    let denom = if val > 64 { 63. } else { 64. };
    BipolarFloat((val - 64) as f64 / denom)
}

fn bipolar_to_midi(val: BipolarFloat) -> u8 {
    u16::min((((val.0 + 1.0) / 2.0) * 128.) as u16, 127) as u8
}

fn unipolar_from_midi(val: u8) -> UnipolarFloat {
    UnipolarFloat(val as f64 / 127.)
}

fn unipolar_to_midi(val: UnipolarFloat) -> u8 {
    (val.0 * 127.) as u8
}

/// Defines a collection of button mappings, only one of which can be active.
/// Knows how to emit MIDI to active just the selected one.
/// Assumes that value 0 turns a indicator off and 1 turns it on.
pub struct RadioButtons {
    mappings: Vec<Mapping>,
}

impl RadioButtons {
    /// Emit midi to ensure that only the selected mapping is selected.
    /// Performs no check that the selected mapping is actually present.
    pub fn select<S: FnMut(Event)>(&self, selected: Mapping, mut send: S) {
        for mapping in &self.mappings {
            let value = (*mapping == selected) as u8;
            send(Event {
                mapping: *mapping,
                value,
            });
        }
    }
}
