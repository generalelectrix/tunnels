// deserialization structs

#[derive(Deserialize, Debug)]
pub struct ParsedArc {
    level: i64,
    thickness: f64,
    hue: f64,
    sat: f64,
    val: i64,
    x: f64,
    y: f64,
    rad_x: f64,
    rad_y: f64,
    start: f64,
    stop: f64,
    rot_angle: f64
}

#[derive(Deserialize, Debug)]
pub struct Snapshot {
    frame_number: i64,
    frame_time: i64, // ms
    draw_ops: Vec<Vec<ParsedArc>>
}