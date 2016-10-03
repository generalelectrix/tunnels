// deserialization structs

#[derive(Deserialize, Debug)]
struct ParsedArc {
    level: i32,
    thickness: f32,
    hue: f32,
    sat: f32,
    val: i32,
    x: f32,
    y: f32,
    rad_x: f32,
    rad_y: f32,
    start: f32,
    stop: f32,
    rot_angle: f32
}