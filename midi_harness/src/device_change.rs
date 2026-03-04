//! Support for wiring up a callback to receive device connection/disconnection updates.
use anyhow::{Result, anyhow};
use std::collections::HashSet;
use std::sync::mpsc::sync_channel;

/// Initialize MIDI device notifications.
/// The provided callback will be called when devices appear or disappear.
///
/// This should be called at application init.
/// If it is called more than once, only the last registered callback will get
/// device appeared/disappeared updates.
///
/// NOTE: this currently only supports MacOS for dynamic device notifications.
/// Other OSes will fall back to an initial report of the currently-connected
/// devices.
pub fn install_midi_device_change_handler(handler: impl HandleDeviceChange) -> Result<()> {
    let (send, recv) = sync_channel(0);

    let mut connected_devices = ConnectedDevices::default();

    std::thread::spawn(move || {
        for _ in recv {
            connected_devices.update(&handler);
        }
    });

    #[cfg(target_os = "macos")]
    // Register for device update notifications
    coremidi_hotplug_notification::receive_device_updates(move || {
        let _ = send.send(());
    })
    .map_err(|err| anyhow!("failed to initialize MIDI harness: {err}"))?;

    #[cfg(not(target_os = "macos"))]
    {
        warn!("MIDI device hotplugging notification is not supported on this OS!");

        // Run one initial cycle of device discovery and report the results.
        // This is sufficient for static device discovery at init.
        // TODO: decide if we can support this for other platforms
        send.send(());
    }

    Ok(())
}

pub trait HandleDeviceChange: Send + 'static {
    /// Handle a device change notification.
    fn on_device_change(&self, change: Result<DeviceChange>);
}

#[derive(Default)]
struct ConnectedDevices {
    inputs: Devices,
    outputs: Devices,
}

impl ConnectedDevices {
    /// Refresh the currently-connected devices. Send messages for devices that
    /// have connected or disconnected, as well as errors.
    fn update(&mut self, handler: &impl HandleDeviceChange) {
        self.update_inputs(handler);
        self.update_outputs(handler);
    }

    fn update_inputs(&mut self, handler: &impl HandleDeviceChange) {
        let port = match midir::MidiInput::new("update_inputs") {
            Ok(port) => port,
            Err(err) => {
                handler.on_device_change(Err(anyhow!("failed to refresh MIDI inputs: {err}")));
                return;
            }
        };
        let ports: Vec<_> = port
            .ports()
            .into_iter()
            .filter_map(|p| {
                let name = port.port_name(&p).ok()?;
                Some((DeviceId(p.id()), name))
            })
            .collect();
        self.inputs = report_device_changes(&self.inputs, ports, DeviceKind::Input, handler);
    }

    fn update_outputs(&mut self, handler: &impl HandleDeviceChange) {
        let port = match midir::MidiOutput::new("update_outputs") {
            Ok(port) => port,
            Err(err) => {
                handler.on_device_change(Err(anyhow!("failed to refresh MIDI inputs: {err}")));
                return;
            }
        };
        let ports: Vec<_> = port
            .ports()
            .into_iter()
            .filter_map(|p| {
                let name = port.port_name(&p).ok()?;
                Some((DeviceId(p.id()), name))
            })
            .collect();
        self.outputs = report_device_changes(&self.outputs, ports, DeviceKind::Output, handler);
    }
}

fn report_device_changes(
    previous: &Devices,
    current: Vec<(DeviceId, String)>,
    kind: DeviceKind,
    handler: &impl HandleDeviceChange,
) -> Devices {
    let current_ids: Devices = current.iter().map(|(id, _)| id).cloned().collect();

    for disconnected in previous.difference(&current_ids) {
        handler.on_device_change(Ok(DeviceChange::Disconnected(disconnected.clone())));
    }
    for (id, name) in current {
        if previous.contains(&id) {
            continue;
        }
        handler.on_device_change(Ok(DeviceChange::Connected { id, name, kind }));
    }
    current_ids
}

type Devices = HashSet<DeviceId>;

/// An opaque ID for a connected MIDI device.
///
/// Produced by the underlying support library. The exact semantics of what
/// generates these is not clear.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]

pub struct DeviceId(pub(crate) String);

/// A device appeared or disappeared.
pub enum DeviceChange {
    Connected {
        id: DeviceId,
        name: String,
        kind: DeviceKind,
    },
    Disconnected(DeviceId),
}

/// Is this a MIDI input or output device?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    /// MIDI input
    Input,
    /// MIDI output
    Output,
}
