pub(crate) mod animation;
pub(crate) mod animation_target;
pub mod audio;
pub(crate) mod clock;
mod device;
pub(crate) mod master_ui;
pub(crate) mod mixer;
pub(crate) mod tunnel;

use log::debug;
use midi_harness::DeviceChange;
use std::sync::mpsc::Sender;

use crate::{
    control::ControlEvent,
    master_ui::EmitStateChange,
    midi::{DeviceSpec, Event, Manager, Mapping, MidiDeviceInit},
    show::{ControlMessage, StateChange},
};
use anyhow::Result;

use tunnels_lib::number::{BipolarFloat, UnipolarFloat};

use self::animation::update_animation_control;
use self::animation_target::update_animation_target_control;
use self::audio::update_audio_control;
use self::clock::update_clock_control;
use self::master_ui::update_master_ui_control;
use self::mixer::update_mixer_control;
use self::tunnel::update_tunnel_control;

pub use self::mixer::PAGE_SIZE as MIXER_CHANNELS_PER_PAGE;
pub use crate::midi_controls::device::{init_apc_20, Device, MidiDevice, MidiHandler};

pub struct Dispatcher {
    midi_manager: Manager,
}

impl Dispatcher {
    /// Instantiate the master midi control dispatcher.
    /// Initialize midi inputs/outputs.
    pub fn new(midi_devices: Vec<MidiDeviceInit>, send: Sender<ControlEvent>) -> Result<Self> {
        let mut midi_manager = Manager::new(send);
        for init in midi_devices {
            match init {
                MidiDeviceInit::Connected(spec) => midi_manager.add_device(spec)?,
                MidiDeviceInit::Slot { name, device } => midi_manager.add_slot(name, device)?,
            }
        }

        Ok(Self { midi_manager })
    }

    /// Map the provided midi event to a show control message.
    /// Return None if the event does not map to a known control.
    pub fn map_event_to_show_control(
        &self,
        device: Device,
        event: Event,
    ) -> Option<ControlMessage> {
        match device.interpret(&event) {
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

    /// Handle a device appearing or disappearing.
    ///
    /// Return true if we should trigger a UI refresh due to a device reconnecting.
    pub fn handle_device_change(&mut self, change: DeviceChange) -> Result<bool> {
        self.midi_manager.handle_device_change(change)
    }

    pub fn add_midi_device(&mut self, spec: DeviceSpec<Device>) -> Result<()> {
        self.midi_manager.add_device(spec)
    }

    pub fn clear_midi_device(&mut self, slot_name: &str) -> Result<()> {
        self.midi_manager.clear_device(slot_name)
    }

    pub fn connect_midi_port(
        &mut self,
        slot_name: &str,
        device_id: midi_harness::DeviceId,
        kind: midi_harness::DeviceKind,
    ) -> Result<()> {
        self.midi_manager.connect_port(slot_name, device_id, kind)
    }

    pub fn midi_slot_statuses(&self) -> Vec<midi_harness::SlotStatus> {
        self.midi_manager.slot_statuses()
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

