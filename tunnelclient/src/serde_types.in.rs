// deserialization structs


#[derive(Deserialize, Debug)]
pub struct ArcSegment {
    pub level: f64,
    pub thickness: f64,
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    pub x: f64,
    pub y: f64,
    pub rad_x: f64,
    pub rad_y: f64,
    pub start: f64,
    pub stop: f64,
    pub rot_angle: f64
}

#[derive(Deserialize, Debug)]
pub struct Snapshot {
    pub frame_number: i64,
    pub frame_time: i64, // ms
    pub draw_ops: Vec<Vec<ArcSegment>>
}