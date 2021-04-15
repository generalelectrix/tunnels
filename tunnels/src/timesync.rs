use std::time::Instant;

use zmq;
use zmq::{Context, Socket};

pub struct Server {
    socket: Socket,
    start: Instant,
}

impl Server {
    pub fn new(ctx: &mut Context, port: u16, start: Instant) -> Result<Self, Box<dyn Error>> {
        let socket = ctx.socket(zmq::REP)?;
        let addr = format!("tcp://*:{}", port);
        socket.bind(addr)?;
        Ok(Self { socket, start })
    }
}
