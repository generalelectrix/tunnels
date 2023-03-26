use log::{error, info};
use std::thread;
use std::{error::Error, time::Instant};

use rmp_serde::Serializer;
use serde::Serialize;
use tunnels_lib::{RunFlag, Timestamp};
use zmq;
use zmq::Context;

const PORT: u64 = 8989;
pub struct TimesyncServer {
    join_handle: Option<thread::JoinHandle<()>>,
    run: RunFlag,
}

impl TimesyncServer {
    /// Start the timesync server.
    /// The server will run until it is dropped.
    pub fn start(ctx: &Context, start: Instant) -> Result<Self, Box<dyn Error>> {
        let socket = ctx.socket(zmq::REP)?;
        let addr = format!("tcp://*:{}", PORT);
        socket.bind(&addr)?;
        // time out once per second
        socket.set_rcvtimeo(1000)?;
        let run = RunFlag::new();
        let run_local = run.clone();

        // start up the service in a new thread
        let mut resp_buf = Vec::new();
        let jh = thread::Builder::new()
            .name("timesync".to_string())
            .spawn(move || loop {
                if !run.should_run() {
                    return;
                }

                match socket.recv_bytes(0) {
                    Err(zmq::Error::EAGAIN) => (),
                    Err(e) => {
                        error!("Timesync receieve error: {}.", e);
                    }
                    Ok(_) => {
                        if let Err(e) =
                            Timestamp::since(start).serialize(&mut Serializer::new(&mut resp_buf))
                        {
                            error!("Timesync serialization error: {}.", e);
                        }
                        if let Err(e) = socket.send(&resp_buf, 0) {
                            error!("Timesync send error: {}.", e);
                        }
                        resp_buf.clear();
                    }
                }
            })?;
        info!("Timesync server started.");
        Ok(Self {
            join_handle: Some(jh),
            run: run_local,
        })
    }
}

impl Drop for TimesyncServer {
    fn drop(&mut self) {
        info!("Timesync server shutting down...");
        self.run.stop();
        self.join_handle.take().unwrap().join().unwrap();
        info!("Timesync server shut down.");
    }
}
