use log::{self, info};
use serde::{Deserialize, Serialize};
use simple_error::bail;
use std::{
    error::Error,
    sync::mpsc::{channel, Receiver, Sender},
    time::{Duration, Instant},
};
use tunnels_lib::Timestamp;

use crate::{
    animation,
    beam_store::{self, BeamStore},
    clock,
    clock::ClockBank,
    device::Device,
    master_ui,
    master_ui::MasterUI,
    midi::{DeviceSpec, Manager},
    midi_controls::Dispatcher,
    mixer,
    mixer::Mixer,
    send::{start_render_service, Frame},
    timesync::TimesyncServer,
    tunnel,
};

#[derive(Copy, Clone, Debug)]
pub enum TestMode {
    Stress,
    Rotation,
    Aliasing,
    MultiChannel,
}

#[derive(Clone, Debug)]
pub struct Config {
    midi_devices: Vec<DeviceSpec>,
    test_mode: Option<TestMode>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            midi_devices: Vec::new(),
            test_mode: None,
        }
    }
}

pub struct Show {
    dispatcher: Dispatcher,
    ui: MasterUI,
    mixer: Mixer,
    clocks: ClockBank,
}

impl Show {
    // /// Create a new show by loading a config file.
    // fn from_config(path: String) -> Result<Self, Box<dyn Error>> {

    // }

    /// Create a new show from the provided config.
    pub fn new(config: Config) -> Result<Self, Box<dyn Error>> {
        // Determine if we need to configure a double-wide mixer for APC20 wing.
        let use_wing = config
            .midi_devices
            .iter()
            .any(|spec| spec.device == Device::AkaiApc20);

        let n_pages = if use_wing { 2 } else { 1 };

        // Initialize midi system.
        let mut midi_manager = Manager::new();
        for device_spec in config.midi_devices.into_iter() {
            midi_manager.add_device(device_spec)?;
        }

        let mut mixer = Mixer::new(n_pages);
        let mut dispatcher = Dispatcher::new(midi_manager);
        let ui = MasterUI::new(n_pages);

        // Emit initial UI state.
        ui.emit_state(&mut mixer, &mut dispatcher);

        Ok(Self {
            dispatcher,
            ui,
            mixer,
            clocks: ClockBank::new(),
        })
    }

    /// Run the show in the current thread.
    pub fn run(&mut self, update_interval: Duration) -> Result<(), Box<dyn Error>> {
        info!("Show is starting.");
        let mut frame_number = 0;
        let mut ctx = zmq::Context::new();
        let start = Instant::now();

        let _timesync = TimesyncServer::start(&mut ctx, start)?;
        let frame_sender = start_render_service(&mut ctx)?;

        let mut last_update = start;
        let mut timestamp = Timestamp(0);

        loop {
            if Instant::now() - last_update > update_interval {
                self.update_state(update_interval);
                last_update += update_interval;
                timestamp.step(update_interval);

                if let Err(_) = frame_sender.send(Frame {
                    number: frame_number,
                    timestamp: timestamp,
                    mixer: self.mixer.clone(),
                    clocks: self.clocks.clone(),
                }) {
                    bail!("Render server hung up.  Aborting show.");
                }
                frame_number += 1;
            }

            // Process a control event for a fraction of the time between now
            // and when we need to update state again.
            if let Some(time_to_next_update) =
                (last_update + update_interval).checked_duration_since(Instant::now())
            {
                // Use 80% of the time remaining to potentially process a
                // control event.
                let timeout = time_to_next_update.mul_f64(0.8);
                self.service_control_event(timeout);
            }
        }
    }

    fn update_state(&mut self, delta_t: Duration) {
        // Update the clocks first as other entities may depend on them for
        // time evolution.
        self.clocks.update_state(delta_t);
        self.mixer.update_state(delta_t, &self.clocks);
    }

    fn service_control_event(&mut self, timeout: Duration) {
        if let Some(msg) = self.dispatcher.receive(timeout) {
            if let Some(control_message) = self.dispatcher.dispatch(msg.0, msg.1) {
                self.ui.handle_control_message(
                    control_message,
                    &mut self.mixer,
                    &mut self.dispatcher,
                )
            }
        }
    }
}

pub enum ControlMessage {
    Tunnel(tunnel::ControlMessage),
    Animation(animation::ControlMessage),
    Mixer(mixer::ControlMessage),
    MasterUI(master_ui::ControlMessage),
}

pub enum StateChange {
    Tunnel(tunnel::StateChange),
    Animation(animation::StateChange),
    Mixer(mixer::StateChange),
    Clock(clock::StateChange),
    MasterUI(master_ui::StateChange),
    //BeamStore(beam_store::StateChange),
}
