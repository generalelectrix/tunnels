use log::{self, info};
use simple_error::bail;
use std::{
    error::Error,
    time::{Duration, Instant},
};
use tunnels_lib::Timestamp;

use crate::{
    animation,
    clock_bank::{self, ClockBank},
    device::Device,
    master_ui,
    master_ui::MasterUI,
    midi::{DeviceSpec, Manager},
    midi_controls::Dispatcher,
    mixer,
    mixer::{Channel, Mixer},
    send::{start_render_service, Frame},
    timesync::TimesyncServer,
    tunnel,
};

pub struct Show {
    dispatcher: Dispatcher,
    ui: MasterUI,
    mixer: Mixer,
    clocks: ClockBank,
}

impl Show {
    /// Create a new show from the provided config.
    pub fn new(midi_devices: Vec<DeviceSpec>) -> Result<Self, Box<dyn Error>> {
        // Determine if we need to configure a double-wide mixer for APC20 wing.
        let use_wing = midi_devices
            .iter()
            .any(|spec| spec.device == Device::AkaiApc20);

        let n_pages = if use_wing { 2 } else { 1 };

        // Initialize midi system.
        let mut midi_manager = Manager::new();
        for device_spec in midi_devices.into_iter() {
            midi_manager.add_device(device_spec)?;
        }

        Ok(Self {
            dispatcher: Dispatcher::new(midi_manager),
            ui: MasterUI::new(n_pages),
            mixer: Mixer::new(n_pages),
            clocks: ClockBank::new(),
        })
    }

    /// Set up the show in a test mode, defined by the provided setup function.
    pub fn test_mode<T: Fn(usize, usize, &mut Channel)>(&mut self, setup: T) {
        let channel_count = self.mixer.channels().count();
        self.mixer
            .channels()
            .enumerate()
            .for_each(|(i, chan)| setup(channel_count, i, chan));
    }

    /// Run the show in the current thread.
    pub fn run(&mut self, update_interval: Duration) -> Result<(), Box<dyn Error>> {
        info!("Show is starting.");

        // Emit initial UI state.
        self.ui
            .emit_state(&mut self.mixer, &mut self.clocks, &mut self.dispatcher);

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
        self.clocks.update_state(delta_t, &mut self.dispatcher);
        self.mixer.update_state(delta_t);
    }

    fn service_control_event(&mut self, timeout: Duration) {
        if let Some(msg) = self.dispatcher.receive(timeout) {
            if let Some(control_message) = self.dispatcher.dispatch(msg.0, msg.1) {
                self.ui.handle_control_message(
                    control_message,
                    &mut self.mixer,
                    &mut self.clocks,
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
    Clock(clock_bank::ControlMessage),
    MasterUI(master_ui::ControlMessage),
}

pub enum StateChange {
    Tunnel(tunnel::StateChange),
    Animation(animation::StateChange),
    Mixer(mixer::StateChange),
    Clock(clock_bank::StateChange),
    MasterUI(master_ui::StateChange),
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_mode::stress;
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    fn calculate_hash<T: Hash>(t: &T) -> u64 {
        let mut s = DefaultHasher::new();
        t.hash(&mut s);
        s.finish()
    }

    /// Test show rendering against static test expectations.
    /// The purpose of this test is to catch accidental regressions in the
    /// tunnel state or rendering algorithm.
    #[test]
    fn test_render() -> Result<(), Box<dyn Error>> {
        let mut show = Show::new(Vec::new())?;

        show.test_mode(stress);

        // Before any evolution, all beams should have the same hash.
        check_render(&show, vec![10192734706909399927; 8]);

        // Evolve by one timestep.
        show.update_state(Duration::from_micros(16667));

        check_render(
            &show,
            vec![
                11099297128101933385,
                3353985019292787671,
                5185332194001566062,
                11932444950289299954,
                8376301734077447906,
                1310600794707049194,
                3051887567039304307,
                10270101680240701565,
            ],
        );
        Ok(())
    }

    /// Render the state of the show, hash the layers, and compare to expectation.
    fn check_render(show: &Show, beam_hashes: Vec<u64>) {
        let video_feeds = show.mixer.render(&show.clocks);

        // Should have the expected number of video channels.
        assert_eq!(Mixer::N_VIDEO_CHANNELS, video_feeds.len());

        // Channel 0 should contain data, but none of the others.
        assert!(video_feeds[0].len() > 0);
        for (i, chan) in video_feeds.iter().enumerate() {
            if i == 0 {
                assert!(chan.len() > 0);
            } else {
                assert_eq!(0, chan.len());
            }
        }

        // Hash each beam and compare to our expectations.
        assert_eq!(beam_hashes.len(), video_feeds[0].len());
        for (beam_hash, channel) in beam_hashes.iter().zip(video_feeds[0].iter()) {
            assert_eq!(*beam_hash, calculate_hash(channel));
        }
    }
}
