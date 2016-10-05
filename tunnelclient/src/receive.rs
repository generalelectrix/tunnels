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


pub struct Receiver {
    ctx: Context,
    socket: Socket
}

impl Receiver {
    pub fn new (addr: &str) -> Self {
        let mut ctx = Context::new();

        let mut socket = ctx.socket(zmq::SUB).unwrap();
        socket.connect(addr).unwrap();

        Receiver {ctx: ctx, socket: socket}
    }

    pub fn receive(&mut self) -> Snapshot {
        let buf = self.socket.recv_bytes(0).unwrap();
        let cur = Cursor::new(&buf[..]);
        let mut de = Deserializer::new(cur);
        Deserialize::deserialize(&mut de).unwrap()
    }
}


#[test]
fn test_parse_arc() {
    let buf = [156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 0, 0, 0, 0, 202, 60, 2, 8, 33, 202, 0, 0, 0, 0];
    let cur = Cursor::new(&buf[..]);
    let mut de = Deserializer::new(cur);
    let result: ArcSegment = Deserialize::deserialize(&mut de).unwrap();
    println!("{:?}", result);
    assert!(true);
}

#[test]
fn test_parse_msg() {
    let buf = [147, 0, 0, 146, 220, 0, 63, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 0, 0, 0, 0, 202, 60, 2, 8, 33, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 60, 130, 8, 33, 202, 60, 195, 12, 49, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 2, 8, 33, 202, 61, 34, 138, 41, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 67, 12, 49, 202, 61, 99, 142, 57, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 130, 8, 33, 202, 61, 146, 73, 37, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 162, 138, 41, 202, 61, 178, 203, 45, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 195, 12, 49, 202, 61, 211, 77, 53, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 227, 142, 57, 202, 61, 243, 207, 61, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 2, 8, 33, 202, 62, 10, 40, 163, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 18, 73, 37, 202, 62, 26, 105, 167, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 34, 138, 41, 202, 62, 42, 170, 171, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 50, 203, 45, 202, 62, 58, 235, 175, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 67, 12, 49, 202, 62, 75, 44, 179, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 83, 77, 53, 202, 62, 91, 109, 183, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 99, 142, 57, 202, 62, 107, 174, 187, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 115, 207, 61, 202, 62, 123, 239, 191, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 130, 8, 33, 202, 62, 134, 24, 98, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 138, 40, 163, 202, 62, 142, 56, 228, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 146, 73, 37, 202, 62, 150, 89, 102, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 154, 105, 167, 202, 62, 158, 121, 232, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 162, 138, 41, 202, 62, 166, 154, 106, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 170, 170, 171, 202, 62, 174, 186, 236, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 178, 203, 45, 202, 62, 182, 219, 110, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 186, 235, 175, 202, 62, 190, 251, 240, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 195, 12, 49, 202, 62, 199, 28, 114, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 203, 44, 179, 202, 62, 207, 60, 244, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 211, 77, 53, 202, 62, 215, 93, 118, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 219, 109, 183, 202, 62, 223, 125, 248, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 227, 142, 57, 202, 62, 231, 158, 122, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 235, 174, 187, 202, 62, 239, 190, 252, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 243, 207, 61, 202, 62, 247, 223, 126, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 251, 239, 191, 202, 63, 0, 0, 0, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 2, 8, 33, 202, 63, 4, 16, 65, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 6, 24, 98, 202, 63, 8, 32, 130, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 10, 40, 163, 202, 63, 12, 48, 195, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 14, 56, 228, 202, 63, 16, 65, 4, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 18, 73, 37, 202, 63, 20, 81, 69, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 22, 89, 102, 202, 63, 24, 97, 134, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 26, 105, 167, 202, 63, 28, 113, 199, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 30, 121, 232, 202, 63, 32, 130, 8, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 34, 138, 41, 202, 63, 36, 146, 73, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 38, 154, 106, 202, 63, 40, 162, 138, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 42, 170, 171, 202, 63, 44, 178, 203, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 46, 186, 236, 202, 63, 48, 195, 12, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 50, 203, 45, 202, 63, 52, 211, 77, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 54, 219, 110, 202, 63, 56, 227, 142, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 58, 235, 175, 202, 63, 60, 243, 207, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 62, 251, 240, 202, 63, 65, 4, 16, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 67, 12, 49, 202, 63, 69, 20, 81, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 71, 28, 114, 202, 63, 73, 36, 146, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 75, 44, 179, 202, 63, 77, 52, 211, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 79, 60, 244, 202, 63, 81, 69, 20, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 83, 77, 53, 202, 63, 85, 85, 85, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 87, 93, 118, 202, 63, 89, 101, 150, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 91, 109, 183, 202, 63, 93, 117, 215, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 95, 125, 248, 202, 63, 97, 134, 24, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 99, 142, 57, 202, 63, 101, 150, 89, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 103, 158, 122, 202, 63, 105, 166, 154, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 107, 174, 187, 202, 63, 109, 182, 219, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 111, 190, 252, 202, 63, 113, 199, 28, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 115, 207, 61, 202, 63, 117, 215, 93, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 119, 223, 126, 202, 63, 121, 231, 158, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 123, 239, 191, 202, 63, 125, 247, 223, 202, 0, 0, 0, 0, 220, 0, 63, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 0, 0, 0, 0, 202, 60, 2, 8, 33, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 60, 130, 8, 33, 202, 60, 195, 12, 49, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 2, 8, 33, 202, 61, 34, 138, 41, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 67, 12, 49, 202, 61, 99, 142, 57, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 130, 8, 33, 202, 61, 146, 73, 37, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 162, 138, 41, 202, 61, 178, 203, 45, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 195, 12, 49, 202, 61, 211, 77, 53, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 61, 227, 142, 57, 202, 61, 243, 207, 61, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 2, 8, 33, 202, 62, 10, 40, 163, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 18, 73, 37, 202, 62, 26, 105, 167, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 34, 138, 41, 202, 62, 42, 170, 171, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 50, 203, 45, 202, 62, 58, 235, 175, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 67, 12, 49, 202, 62, 75, 44, 179, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 83, 77, 53, 202, 62, 91, 109, 183, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 99, 142, 57, 202, 62, 107, 174, 187, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 115, 207, 61, 202, 62, 123, 239, 191, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 130, 8, 33, 202, 62, 134, 24, 98, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 138, 40, 163, 202, 62, 142, 56, 228, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 146, 73, 37, 202, 62, 150, 89, 102, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 154, 105, 167, 202, 62, 158, 121, 232, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 162, 138, 41, 202, 62, 166, 154, 106, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 170, 170, 171, 202, 62, 174, 186, 236, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 178, 203, 45, 202, 62, 182, 219, 110, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 186, 235, 175, 202, 62, 190, 251, 240, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 195, 12, 49, 202, 62, 199, 28, 114, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 203, 44, 179, 202, 62, 207, 60, 244, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 211, 77, 53, 202, 62, 215, 93, 118, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 219, 109, 183, 202, 62, 223, 125, 248, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 227, 142, 57, 202, 62, 231, 158, 122, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 235, 174, 187, 202, 62, 239, 190, 252, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 243, 207, 61, 202, 62, 247, 223, 126, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 62, 251, 239, 191, 202, 63, 0, 0, 0, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 2, 8, 33, 202, 63, 4, 16, 65, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 6, 24, 98, 202, 63, 8, 32, 130, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 10, 40, 163, 202, 63, 12, 48, 195, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 14, 56, 228, 202, 63, 16, 65, 4, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 18, 73, 37, 202, 63, 20, 81, 69, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 22, 89, 102, 202, 63, 24, 97, 134, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 26, 105, 167, 202, 63, 28, 113, 199, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 30, 121, 232, 202, 63, 32, 130, 8, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 34, 138, 41, 202, 63, 36, 146, 73, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 38, 154, 106, 202, 63, 40, 162, 138, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 42, 170, 171, 202, 63, 44, 178, 203, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 46, 186, 236, 202, 63, 48, 195, 12, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 50, 203, 45, 202, 63, 52, 211, 77, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 54, 219, 110, 202, 63, 56, 227, 142, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 58, 235, 175, 202, 63, 60, 243, 207, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 62, 251, 240, 202, 63, 65, 4, 16, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 67, 12, 49, 202, 63, 69, 20, 81, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 71, 28, 114, 202, 63, 73, 36, 146, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 75, 44, 179, 202, 63, 77, 52, 211, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 79, 60, 244, 202, 63, 81, 69, 20, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 83, 77, 53, 202, 63, 85, 85, 85, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 87, 93, 118, 202, 63, 89, 101, 150, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 91, 109, 183, 202, 63, 93, 117, 215, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 95, 125, 248, 202, 63, 97, 134, 24, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 99, 142, 57, 202, 63, 101, 150, 89, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 103, 158, 122, 202, 63, 105, 166, 154, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 107, 174, 187, 202, 63, 109, 182, 219, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 111, 190, 252, 202, 63, 113, 199, 28, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 115, 207, 61, 202, 63, 117, 215, 93, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 119, 223, 126, 202, 63, 121, 231, 158, 202, 0, 0, 0, 0, 156, 204, 255, 202, 62, 128, 0, 0, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 204, 255, 202, 0, 0, 0, 0, 202, 0, 0, 0, 0, 202, 62, 224, 0, 0, 202, 62, 224, 0, 0, 202, 63, 123, 239, 191, 202, 63, 125, 247, 223, 202, 0, 0, 0, 0];
    let cur = Cursor::new(&buf[..]);
    let mut de = Deserializer::new(cur);
    let result: Snapshot = Deserialize::deserialize(&mut de).unwrap();
    println!("{:?}", result);
    assert!(true);
}

#[test]
fn test_unpack_multiple() {
    let buf = [146, 1, 2];
    let cur = Cursor::new(&buf[..]);
    let mut de = Deserializer::new(cur);
    let x: (i32, i32) = Deserialize::deserialize(&mut de).unwrap();
    //let y: i32 = Deserialize::deserialize(&mut de).unwrap();
    println!("{:?}", x);
}
