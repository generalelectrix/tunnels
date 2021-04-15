use log::error;
use std::thread;
use std::{error::Error, time::Duration, time::Instant};

use rmp_serde::Serializer;
use serde::Serialize;
use tunnels_lib::RunFlag;
use zmq;
use zmq::Context;

pub fn run(
    ctx: &mut Context,
    port: u16,
    start: Instant,
    run: RunFlag,
) -> Result<thread::JoinHandle<()>, Box<dyn Error>> {
    let socket = ctx.socket(zmq::REP)?;
    let addr = format!("tcp://*:{}", port);
    socket.bind(&addr)?;
    // time out once per second
    socket.set_rcvtimeo(1000)?;

    // start up the service in a new thread
    let jh = thread::Builder::new()
        .name("timesync".to_string())
        .spawn(move || {
            let mut resp_buf = Vec::new();
            loop {
                if !run.should_run() {
                    return;
                }

                match socket.recv_bytes(0) {
                    Err(zmq::Error::EAGAIN) => (),
                    Err(e) => {
                        error!("Timesync receieve error: {}.", e);
                    }
                    Ok(_) => {
                        let now = start.elapsed().as_secs_f64();
                        if let Err(e) = now.serialize(&mut Serializer::new(&mut resp_buf)) {
                            error!("Timesync serialization error: {}.", e);
                        }
                        if let Err(e) = socket.send(&resp_buf, 0) {
                            error!("Timesync send error: {}.", e);
                        }
                    }
                }
                resp_buf.clear();
            }
        })?;
    Ok(jh)
}
