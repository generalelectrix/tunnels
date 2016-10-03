// #![feature(rustc_macro)]

use zmq;
use zmq::{Context, Socket};
use rmp_serde::Deserializer;
use serde::Deserialize;
use std::io::Cursor;

// only needed for serde on stable
include!(concat!(env!("OUT_DIR"), "/serde_types.rs"));

// #[derive(Serialize, Deserialize, Debug)]
// struct ParsedArc {
//     level: i32,
//     thickness: f32,
//     hue: f32,
//     sat: f32,
//     val: i32,
//     x: f32,
//     y: f32,
//     rad_x: f32,
//     rad_y: f32,
//     start: f32,
//     stop: f32,
//     rot_angle: f32
// }

/// Receive a single arc from a blocking call to socket.
fn receive_single_arc(socket: &mut Socket) -> ParsedArc {
    let buf = socket.recv_bytes(0).unwrap();
    let cur = Cursor::new(&buf[..]);
    let mut de = Deserializer::new(cur);
    let result: ParsedArc = Deserialize::deserialize(&mut de).unwrap();
    result
}

struct Receiver {
    ctx: Context,
    socket: Socket
}

impl Receiver {
    fn new (addr: &str) -> Self {
        let mut ctx = Context::new();

        let mut socket = ctx.socket(zmq::SUB).unwrap();
        socket.connect(addr).unwrap();

        Receiver {ctx: ctx, socket: socket}
    }

    fn receive(&mut self) -> ParsedArc {
        receive_single_arc(&mut self.socket)
    }
}


#[test]
fn test_parse_arc() {
    let buf = [156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 0, 0, 0, 0, 202, 60, 2, 8, 33, 202, 0, 0, 0, 0];
    let cur = Cursor::new(&buf[..]);
    let mut de = Deserializer::new(cur);
    let result: ParsedArc = Deserialize::deserialize(&mut de).unwrap();
    println!("{:?}", result);
    assert!(true);
}
