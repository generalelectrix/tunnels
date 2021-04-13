mod animation;
mod beam;
mod beam_matrix_minder;
mod clock;
mod device;
mod look;
mod midi;
mod midi_controls;
mod mixer;
mod numbers;
mod show;
mod tunnel;
mod waveforms;

use std::error::Error;

use midi::list_ports;
use show::Show;

fn main() -> Result<(), Box<dyn Error>> {
    let (inputs, outputs) = list_ports()?;
    println!("Available input ports:\n{}\n", inputs);
    println!("Available output ports:\n{}\n", outputs);
    Ok(())
}
