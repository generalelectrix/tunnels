mod animation;
mod animation_target;
mod audio;
mod clock;
mod device;
mod master_ui;
mod mixer;
mod tunnel;

use log::debug;
use std::{collections::HashMap, sync::mpsc::Sender};

use crate::{
    control::ControlEvent,
    master_ui::EmitStateChange,
    midi::{DeviceSpec, Event, Manager, Mapping},
    show::ControlMessage,
    show::StateChange,
};
use anyhow::Result;

use tunnels_lib::number::{BipolarFloat, UnipolarFloat};

use self::animation::{map_animation_controls, update_animation_control};
use self::animation_target::{map_animation_target_controls, update_animation_target_control};
use self::audio::{map_audio_controls, update_audio_control};
use self::clock::{map_clock_controls, update_clock_control};
use self::master_ui::{map_master_ui_controls, update_master_ui_control};
use self::mixer::{map_mixer_controls, update_mixer_control};
use self::tunnel::{map_tunnel_controls, update_tunnel_control};

pub use self::mixer::PAGE_SIZE as MIXER_CHANNELS_PER_PAGE;
pub use crate::midi_controls::device::{init_apc_20, Device, MidiDevice};

type ControlMessageCreator = Box<dyn Fn(u8) -> ControlMessage>;

pub struct ControlMap(pub HashMap<(Device, Mapping), ControlMessageCreator>);

impl ControlMap {
    // Initialize a new instance of the control map.
    fn new() -> Self {
        let mut map = Self(HashMap::new());
        map_tunnel_controls(Device::AkaiApc40, &mut map);
        map_tunnel_controls(Device::TouchOsc, &mut map);

        map_animation_controls(Device::AkaiApc40, &mut map);
        map_animation_controls(Device::TouchOsc, &mut map);

        map_animation_target_controls(Device::TouchOsc, &mut map);

        map_mixer_controls(Device::AkaiApc40, 0, &mut map);
        map_mixer_controls(Device::AkaiApc20, 1, &mut map);
        map_mixer_controls(Device::TouchOsc, 0, &mut map);
        // FIXME: need to split out the video controls from the mixer controls
        // map_mixer_controls(Device::TouchOsc, 1, &mut map);

        map_master_ui_controls(Device::AkaiApc40, 0, &mut map);
        map_master_ui_controls(Device::AkaiApc20, 1, &mut map);
        map_master_ui_controls(Device::TouchOsc, 0, &mut map);
        // FIXME: need to split out the pagewise controls from the non-pagewise controls
        // map_master_ui_controls(Device::TouchOsc, 1, &mut map);

        map_clock_controls(Device::BehringerCmdMM1, &mut map);
        map_clock_controls(Device::TouchOsc, &mut map);

        map_audio_controls(Device::TouchOsc, &mut map);
        map
    }

    pub fn add(&mut self, device: Device, mapping: Mapping, creator: ControlMessageCreator) {
        if self.0.insert((device, mapping), creator).is_some() {
            panic!("duplicate control definition: {:?} {:?}", device, mapping);
        }
    }

    /// Map a midi source device and event into a tunnels control message.
    /// Return None if no mapping is registered.
    pub fn dispatch(&self, device: Device, event: Event) -> Option<ControlMessage> {
        self.0.get(&(device, event.mapping)).map(|c| c(event.value))
    }

    #[allow(unused)]
    // Produce a report describing all controls bound to all devices.
    pub fn report(&self) -> String {
        let mut controls: HashMap<Device, Vec<Mapping>> = HashMap::new();
        for ((device, mapping), _) in self.0.iter() {
            match controls.get_mut(device) {
                Some(mappings) => {
                    mappings.push(*mapping);
                }
                None => {
                    controls.insert(*device, vec![*mapping]);
                }
            }
        }

        let mut report = Vec::new();

        // Sort the mappings and produce the report.
        for (device, mappings) in controls.iter_mut() {
            mappings.sort();
            report.push(format!("{}", device));
            for mapping in mappings {
                report.push(format!("{}", mapping))
            }
        }
        report.join("\n")
    }
}

pub struct Dispatcher {
    midi_map: ControlMap,
    midi_manager: Manager<Device>,
}

impl Dispatcher {
    /// Instantiate the master midi control dispatcher.
    /// Create the midi control map and initialize midi inputs/outputs.
    pub fn new(midi_devices: Vec<DeviceSpec<Device>>, send: Sender<ControlEvent>) -> Result<Self> {
        let midi_map = ControlMap::new();

        let mut midi_manager = Manager::default();
        for device_spec in midi_devices.into_iter() {
            midi_manager.add_device(device_spec, send.clone())?;
        }

        Ok(Self {
            midi_map,
            midi_manager,
        })
    }

    /// Map the provided midi event to a show control message.
    /// Return None if the event does not map to a known control.
    pub fn map_event_to_show_control(
        &self,
        device: Device,
        event: Event,
    ) -> Option<ControlMessage> {
        match self.midi_map.dispatch(device, event) {
            Some(cm) => Some(cm),
            None => {
                debug!(
                    "Unknown midi command from device {} with mapping {}.",
                    device, event.mapping
                );
                None
            }
        }
    }
}

impl EmitStateChange for Dispatcher {
    /// Map application state changes into UI update midi messages.
    fn emit(&mut self, sc: StateChange) {
        match sc {
            StateChange::Tunnel(sc) => update_tunnel_control(sc, &mut self.midi_manager),
            StateChange::Animation(sc) => update_animation_control(sc, &mut self.midi_manager),
            StateChange::AnimationTarget(sc) => {
                update_animation_target_control(sc, &mut self.midi_manager)
            }
            StateChange::Mixer(sc) => update_mixer_control(sc, &mut self.midi_manager),
            StateChange::Clock(sc) => update_clock_control(sc, &mut self.midi_manager),
            StateChange::ColorPalette(_) => {
                // TODO: emit color data to interfaces if we build a color palette monitor
            }
            StateChange::MasterUI(sc) => update_master_ui_control(sc, &mut self.midi_manager),
            StateChange::Audio(sc) => update_audio_control(sc, &mut self.midi_manager),
        }
    }
}

pub fn bipolar_from_midi(val: u8) -> BipolarFloat {
    let denom = if val > 64 { 63. } else { 64. };
    BipolarFloat::new((val as f64 - 64.) / denom)
}

pub fn bipolar_to_midi(val: BipolarFloat) -> u8 {
    u16::min((((val.val() + 1.0) / 2.0) * 128.) as u16, 127) as u8
}

pub fn unipolar_from_midi(val: u8) -> UnipolarFloat {
    UnipolarFloat::new(val as f64 / 127.)
}

pub fn unipolar_to_midi(val: UnipolarFloat) -> u8 {
    (val.val() * 127.) as u8
}

/// Scale input with a quadratic curve.
/// This provides more resolution for smaller values.
fn quadratic_knob_input(v: BipolarFloat) -> BipolarFloat {
    let mut scaled = f64::powi(v.val(), 2);
    if v < 0. {
        scaled *= -1.
    }
    BipolarFloat::new(scaled)
}

/// Scale a linear value back into a quadratic knob mapping.
/// This provides more resolution for smaller values.
fn quadratic_knob_output(v: BipolarFloat) -> BipolarFloat {
    let mut scaled = v.val().abs().sqrt();
    if v < 0. {
        scaled *= -1.
    }
    BipolarFloat::new(scaled)
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

#[test]
fn test_quadratic_knob_roundtrip() {
    fn roundtrip(v: f64) {
        let v = BipolarFloat::new(v);
        assert_eq!(v, quadratic_knob_output(quadratic_knob_input(v)));
    }
    roundtrip(0.0);
    roundtrip(0.1);
    roundtrip(-0.1);
    roundtrip(1.0);
    roundtrip(-1.0);
}
