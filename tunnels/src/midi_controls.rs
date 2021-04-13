mod tunnel;

use std::collections::HashMap;

use crate::{
    device::Device,
    midi::{Event, EventType, Mapping},
    numbers::{BipolarFloat, UnipolarFloat},
    show::ControlMessage,
    show::StateChange,
};

use self::tunnel::{map_tunnel_controls, update_tunnel_control};

type ControlMessageCreator = fn(u8) -> ControlMessage;

type ControlMap = HashMap<(Device, Mapping), ControlMessageCreator>;
pub struct Dispatcher {
    map: ControlMap,
}

impl Dispatcher {
    /// Instantiate the master midi control dispatcher.
    pub fn new() -> Self {
        let mut map = HashMap::new();
        map_tunnel_controls(Device::AkaiApc40, &mut map);
        map_tunnel_controls(Device::TouchOsc, &mut map);
        Self { map }
    }

    /// Map a midi source device and event into a tunnels control message.
    /// Return None if no mapping is registered.
    pub fn dispatch(&self, device: Device, event: Event) -> Option<ControlMessage> {
        self.map
            .get(&(device, event.mapping))
            .map(|c| c(event.value))
    }

    /// Map application state changes into UI update midi messages.
    pub fn update<S>(&self, sc: StateChange, send_midi: S)
    where
        S: Fn(Device, Event),
    {
        match sc {
            StateChange::Tunnel(sc) => update_tunnel_control(sc, send_midi),
        }
    }
}

fn add_control(
    map: &mut ControlMap,
    device: Device,
    event_type: EventType,
    channel: u8,
    control: u8,
    creator: ControlMessageCreator,
) {
    let mapping = Mapping {
        event_type,
        channel,
        control,
    };
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
