mod animation;
mod clock;
mod master_ui;
mod mixer;
mod tunnel;

use std::{collections::HashMap, time::Duration};

use crate::{
    device::Device,
    master_ui::EmitStateChange,
    midi::{Event, Manager, Mapping},
    numbers::{BipolarFloat, UnipolarFloat},
    show::ControlMessage,
    show::StateChange,
};

use self::animation::{map_animation_controls, update_animation_control};
use self::clock::{map_clock_controls, update_clock_control};
use self::master_ui::{map_master_ui_controls, update_master_ui_control};
use self::mixer::{map_mixer_controls, update_mixer_control};
use self::tunnel::{map_tunnel_controls, update_tunnel_control};

type ControlMessageCreator = Box<dyn Fn(u8) -> ControlMessage>;

pub struct ControlMap(pub HashMap<(Device, Mapping), ControlMessageCreator>);

impl ControlMap {
    fn new() -> Self {
        Self(HashMap::new())
    }
    pub fn add(&mut self, device: Device, mapping: Mapping, creator: ControlMessageCreator) {
        if self.0.insert((device, mapping), creator).is_some() {
            panic!("duplicate control definition: {:?} {:?}", device, mapping);
        }
    }
}
pub struct Dispatcher {
    map: ControlMap,
    manager: Manager,
}

impl Dispatcher {
    /// Instantiate the master midi control dispatcher.
    pub fn new(manager: Manager) -> Self {
        let mut map = ControlMap::new();
        map_tunnel_controls(Device::AkaiApc40, &mut map);
        map_tunnel_controls(Device::TouchOsc, &mut map);

        map_animation_controls(Device::AkaiApc40, &mut map);
        map_animation_controls(Device::TouchOsc, &mut map);

        map_mixer_controls(Device::AkaiApc40, 0, &mut map);
        map_mixer_controls(Device::AkaiApc20, 1, &mut map);
        map_mixer_controls(Device::TouchOsc, 0, &mut map);
        map_mixer_controls(Device::TouchOsc, 1, &mut map);

        map_master_ui_controls(Device::AkaiApc40, 0, &mut map);
        map_master_ui_controls(Device::AkaiApc20, 1, &mut map);
        map_master_ui_controls(Device::TouchOsc, 0, &mut map);
        map_master_ui_controls(Device::TouchOsc, 1, &mut map);

        // TODO: map clock controls for new hardware
        map_clock_controls(Device::TouchOsc, &mut map);
        Self { map, manager }
    }

    pub fn receive(&self, timeout: Duration) -> Option<(Device, Event)> {
        self.manager.receive(timeout)
    }

    /// Map a midi source device and event into a tunnels control message.
    /// Return None if no mapping is registered.
    pub fn dispatch(&self, device: Device, event: Event) -> Option<ControlMessage> {
        self.map
            .0
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
            StateChange::Mixer(sc) => update_mixer_control(sc, &mut self.manager),
            StateChange::Clock(sc) => update_clock_control(sc, &mut self.manager),
            StateChange::MasterUI(sc) => update_master_ui_control(sc, &mut self.manager),
        }
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
/// Knows how to emit MIDI to activate just the selected one.
pub struct RadioButtons {
    mappings: Vec<Mapping>,
    off: u8,
    on: u8,
}

impl RadioButtons {
    /// Emit midi to ensure that only the selected mapping is selected.
    /// Performs no check that the selected mapping is actually present.
    pub fn select<S: FnMut(Event)>(&self, selected: Mapping, mut send: S) {
        for mapping in &self.mappings {
            let value = if *mapping == selected {
                self.on
            } else {
                self.off
            };
            send(Event {
                mapping: *mapping,
                value,
            });
        }
    }

    /// Emit midi to turn every mapping off.
    pub fn all_off<S: FnMut(Event)>(&self, mut send: S) {
        for mapping in &self.mappings {
            send(Event {
                mapping: *mapping,
                value: self.off,
            });
        }
    }
}
