use zmq::{Context, Socket};
use rmp_serde;

include!(concat!(env!("OUT_DIR"), "/serde_types.rs"));

/// Receive a single arc from a blocking call to socket.
fn receive_single_arc(socket: &Socket, ctx: &Context) -> ParsedArc {
    rmp_serde::from_str(socket.recv_string().unwrap()).unwrap()
}

/// Initialize a 0mq receiver and return a closure that will poll the receive
/// queue in a non-blocking fashion, returning a collection of snapshots if any
/// were received.  Panics if anything goes wrong.
fn init_receiver(addr: &str) -> Box<Fn() -> ParsedArc> {
    let mut ctx = zmq::Context::new();

    let mut socket = ctx.socket(zmq::SUB).unwrap();
    socket.connect(addr).unwrap();
    move || -> ParsedArc {
        receive_single_arc(socket, ctx)
    }
}