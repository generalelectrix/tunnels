//! Compare the frame codec against the msgpack wire format on size, speed and
//! representation error.
//!
//! Run in release; debug timings for this kind of code are meaningless.

use std::time::{Duration, Instant};

use rmp_serde::Serializer;
use serde::{Deserialize, Serialize};
use tunnels::tunnel::fixture;
use tunnels_lib::frame_codec::{self, EncodeStats, NUMERIC_FIELDS, Width};
use tunnels_lib::{ShapeGeometry, Snapshot};

const FRAME_RATE: f64 = 240.0;
const CLIENTS: f64 = 6.0;
/// A client's whole budget for one frame at the show's render rate.
const FRAME_BUDGET_US: f64 = 1e6 / FRAME_RATE;

const X_RES: f64 = 1920.0;
const Y_RES: f64 = 1080.0;
const CRITICAL_SIZE: f64 = 1080.0;
const THICKNESS_SCALE: f64 = 0.5;
/// Radius at which an angular error becomes a displacement in pixels.
const REFERENCE_RADIUS: f64 = CRITICAL_SIZE / 2.0;

/// How a field's error converts to something observable, and what that is called.
fn display_scale(field_id: u8) -> (f64, &'static str) {
    use frame_codec::field as f;
    let turn_to_px = std::f64::consts::TAU * REFERENCE_RADIUS;
    match field_id {
        f::LEVEL | f::SAT | f::VAL => (255.0, "levels"),
        // A unit of hue traverses six colour sectors, each a full 8-bit ramp.
        f::HUE => (255.0 * 6.0, "levels"),
        f::THICKNESS => (CRITICAL_SIZE * THICKNESS_SCALE / 2.0, "px"),
        f::X => (X_RES, "px"),
        f::Y => (Y_RES, "px"),
        f::EXTENT_X | f::EXTENT_Y => (CRITICAL_SIZE, "px"),
        _ => (turn_to_px, "px"),
    }
}

// --- the cheap alternative: f32 fields and integer enum discriminants ---

#[derive(Serialize, Deserialize)]
struct ShapeF32 {
    render_mode: u8,
    path_shape: u8,
    level: f32,
    thickness: f32,
    hue: f32,
    sat: f32,
    val: f32,
    x: f32,
    y: f32,
    extent_x: f32,
    extent_y: f32,
    start: f32,
    stop: f32,
    rot_angle: f32,
    spin_angle: f32,
}

#[derive(Serialize, Deserialize)]
struct SnapshotF32 {
    frame_number: u64,
    layers: Vec<Vec<ShapeF32>>,
}

impl From<&Snapshot> for SnapshotF32 {
    fn from(s: &Snapshot) -> Self {
        Self {
            frame_number: s.frame_number,
            layers: s
                .layers
                .iter()
                .map(|layer| {
                    let render_mode = layer.render_mode as u8;
                    let path_shape = layer.path_shape as u8;
                    let span = layer.span;
                    layer
                        .shapes
                        .iter()
                        .map(|sh| ShapeF32 {
                            render_mode,
                            path_shape,
                            level: sh.level as f32,
                            thickness: sh.thickness as f32,
                            hue: sh.hue as f32,
                            sat: sh.sat as f32,
                            val: sh.val as f32,
                            x: sh.x as f32,
                            y: sh.y as f32,
                            extent_x: sh.extent_x as f32,
                            extent_y: sh.extent_y as f32,
                            start: sh.start as f32,
                            stop: (sh.start + span) as f32,
                            rot_angle: sh.rot_angle as f32,
                            spin_angle: sh.spin_angle as f32,
                        })
                        .collect()
                })
                .collect(),
        }
    }
}

fn msgpack_f64(s: &Snapshot) -> Vec<u8> {
    let mut buf = Vec::new();
    s.serialize(&mut Serializer::new(&mut buf)).unwrap();
    buf
}

fn msgpack_f32(s: &SnapshotF32) -> Vec<u8> {
    let mut buf = Vec::new();
    s.serialize(&mut Serializer::new(&mut buf)).unwrap();
    buf
}

fn codec(s: &Snapshot) -> Vec<u8> {
    let mut buf = Vec::new();
    frame_codec::encode(s, &mut buf);
    buf
}

fn lz4(b: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(b)
}

fn zstd1(b: &[u8]) -> Vec<u8> {
    zstd::encode_all(b, 1).unwrap()
}

/// Median nanoseconds for one call, over enough repetitions to swamp noise.
fn time_ns<T>(mut f: impl FnMut() -> T) -> f64 {
    for _ in 0..20 {
        std::hint::black_box(f());
    }
    let mut samples = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline && samples.len() < 4000 {
        let batch = 8;
        let t0 = Instant::now();
        for _ in 0..batch {
            std::hint::black_box(f());
        }
        samples.push(t0.elapsed().as_secs_f64() * 1e9 / f64::from(batch));
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

struct Row {
    name: &'static str,
    bytes: usize,
    encode_us: f64,
    decode_us: f64,
}

fn get(s: &ShapeGeometry, id: u8) -> f64 {
    use frame_codec::field as f;
    match id {
        f::LEVEL => s.level,
        f::THICKNESS => s.thickness,
        f::HUE => s.hue,
        f::SAT => s.sat,
        f::VAL => s.val,
        f::X => s.x,
        f::Y => s.y,
        f::EXTENT_X => s.extent_x,
        f::EXTENT_Y => s.extent_y,
        f::START => s.start,
        f::ROT_ANGLE => s.rot_angle,
        f::SPIN_ANGLE => s.spin_angle,
        _ => 0.0,
    }
}

fn main() {
    let fixtures: Vec<(&str, Snapshot)> = vec![
        ("default tunnel", fixture::default_tunnel_snapshot()),
        ("stress tunnel", fixture::stress_tunnel_evolved_snapshot()),
        (
            "MAX VARIATION (8 x 128 segs)",
            fixture::max_variation_frame_snapshot(),
        ),
        ("marquee animated", fixture::marquee_animated_snapshot(3)),
        ("mixed-mode look", fixture::mixed_mode_look_snapshot()),
    ];

    println!("# Frame codec measurement report\n");
    println!(
        "{FRAME_RATE} Hz, {CLIENTS} clients, release build. Error expressed against a \
         {X_RES:.0}x{Y_RES:.0} screen. Client frame budget is {FRAME_BUDGET_US:.0} µs.\n"
    );

    for (name, snap) in &fixtures {
        let n_shapes = snap.n_shapes();
        println!("\n## {name}\n");
        println!("{} layer(s), {} shapes.\n", snap.n_layers(), n_shapes);

        let f32_snap = SnapshotF32::from(snap);
        let mp64 = msgpack_f64(snap);
        let mp32 = msgpack_f32(&f32_snap);
        let wire = codec(snap);
        let plain = lz4_flex::decompress_size_prepended(&wire).unwrap();

        let baseline = mp64.len();
        let mp64_lz4 = lz4(&mp64);
        let mp32_lz4 = lz4(&mp32);
        let mp32_zstd = zstd1(&mp32);

        let rows = [
            Row {
                name: "msgpack f64 (current)",
                bytes: mp64.len(),
                encode_us: time_ns(|| msgpack_f64(snap)) / 1000.0,
                decode_us: time_ns(|| rmp_serde::from_slice::<Snapshot>(&mp64).unwrap()) / 1000.0,
            },
            Row {
                name: "msgpack f64 + lz4",
                bytes: mp64_lz4.len(),
                encode_us: time_ns(|| lz4(&msgpack_f64(snap))) / 1000.0,
                decode_us: time_ns(|| {
                    let raw = lz4_flex::decompress_size_prepended(&mp64_lz4).unwrap();
                    rmp_serde::from_slice::<Snapshot>(&raw).unwrap()
                }) / 1000.0,
            },
            Row {
                name: "msgpack f32 + int enums + lz4",
                bytes: mp32_lz4.len(),
                encode_us: time_ns(|| lz4(&msgpack_f32(&SnapshotF32::from(snap)))) / 1000.0,
                decode_us: time_ns(|| {
                    let raw = lz4_flex::decompress_size_prepended(&mp32_lz4).unwrap();
                    rmp_serde::from_slice::<SnapshotF32>(&raw).unwrap()
                }) / 1000.0,
            },
            Row {
                name: "  same, zstd-1",
                bytes: mp32_zstd.len(),
                encode_us: time_ns(|| zstd1(&msgpack_f32(&SnapshotF32::from(snap)))) / 1000.0,
                decode_us: time_ns(|| {
                    let raw = zstd::decode_all(&mp32_zstd[..]).unwrap();
                    rmp_serde::from_slice::<SnapshotF32>(&raw).unwrap()
                }) / 1000.0,
            },
            Row {
                name: "CODEC, before its own lz4",
                bytes: plain.len(),
                encode_us: 0.0,
                decode_us: 0.0,
            },
            Row {
                name: "CODEC (flat columnar, on the wire)",
                bytes: wire.len(),
                encode_us: time_ns(|| codec(snap)) / 1000.0,
                decode_us: time_ns(|| frame_codec::decode(&wire).unwrap()) / 1000.0,
            },
        ];

        println!("| encoding | bytes | B/shape | vs f64 | enc µs | dec µs | Mbit/s/client | × 6 |");
        println!("|---|---:|---:|---:|---:|---:|---:|---:|");
        for r in &rows {
            let per_client = r.bytes as f64 * 8.0 * FRAME_RATE / 1e6;
            println!(
                "| {} | {} | {:.1} | {:.2}× | {} | {} | {:.1} | {:.0} |",
                r.name,
                r.bytes,
                r.bytes as f64 / n_shapes as f64,
                baseline as f64 / r.bytes as f64,
                if r.encode_us > 0.0 {
                    format!("{:.1}", r.encode_us)
                } else {
                    "—".into()
                },
                if r.decode_us > 0.0 {
                    format!("{:.1}", r.decode_us)
                } else {
                    "—".into()
                },
                per_client,
                per_client * CLIENTS,
            );
        }

        let mut stats = EncodeStats::default();
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        frame_codec::encode_with_stats(snap, &mut scratch, &mut out, &mut stats);
        let predicted = frame_codec::uncompressed_size(snap.shapes_per_layer());
        println!(
            "\nUncompressed {} B (predicted from shape counts: {predicted} B) = {:.1} B/shape, flat.",
            stats.uncompressed_bytes,
            stats.uncompressed_bytes as f64 / n_shapes as f64,
        );

        let clamps: Vec<String> = NUMERIC_FIELDS
            .iter()
            .enumerate()
            .filter(|(i, _)| stats.clamped[*i] > 0)
            .map(|(i, f)| format!("{}={}", f.name, stats.clamped[i]))
            .collect();
        if clamps.is_empty() {
            println!("Range audit: no value left the unit interval its type guarantees.");
        } else {
            println!("Range audit: OUT OF RANGE — {}", clamps.join(", "));
        }

        let decoded = frame_codec::decode(&wire).unwrap();
        let mut worst_px: f64 = 0.0;
        let mut lines = Vec::new();
        for spec in &NUMERIC_FIELDS {
            let (scale, unit) = display_scale(spec.id);
            let mut max: f64 = 0.0;
            for (ol, gl) in snap.layers.iter().zip(decoded.layers.iter()) {
                for (a, b) in ol.shapes.iter().zip(gl.shapes.iter()) {
                    max = max.max((get(a, spec.id) - get(b, spec.id)).abs());
                }
            }
            if unit == "px" {
                worst_px = worst_px.max(max * scale);
            }
            let representation = match spec.width {
                Width::U8 => "u8",
                Width::U16 => "u16",
                Width::F32 => "f32",
            };
            lines.push(format!(
                "{} ({representation}) {:.5} {unit}",
                spec.name,
                max * scale
            ));
        }
        println!("\nWorst error per field: {}", lines.join(", "));
        println!("Worst geometric error: {worst_px:.5} px.");
    }
}
