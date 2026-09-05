//! Measure what a rendered frame costs on the wire, and what the wire format
//! costs it in fidelity.
//!
//! Run in release; debug timings for this kind of code are meaningless.

use std::time::{Duration, Instant};

use rmp_serde::Serializer;
use serde::Serialize;
use tunnels::tunnel::fixture;
use tunnels_lib::frame_codec::{self, EncodeStats, NUMERIC_FIELDS};
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
/// Displacement a full turn of angular error produces at that radius.
const TURN_TO_PX: f64 = std::f64::consts::TAU * REFERENCE_RADIUS;

/// One shape field, and how an error in it becomes something an audience sees.
struct Field {
    name: &'static str,
    get: fn(&ShapeGeometry) -> f64,
    /// Multiplier turning an error in the field's own units into `unit`.
    scale: f64,
    unit: &'static str,
}

const FIELDS: [Field; 12] = [
    Field {
        name: "level",
        get: |s| s.level,
        scale: 255.0,
        unit: "levels",
    },
    Field {
        name: "thickness",
        get: |s| s.thickness,
        scale: CRITICAL_SIZE * THICKNESS_SCALE / 2.0,
        unit: "px",
    },
    Field {
        // A unit of hue traverses six colour sectors, each a full 8-bit ramp.
        name: "hue",
        get: |s| s.hue,
        scale: 255.0 * 6.0,
        unit: "levels",
    },
    Field {
        name: "sat",
        get: |s| s.sat,
        scale: 255.0,
        unit: "levels",
    },
    Field {
        name: "val",
        get: |s| s.val,
        scale: 255.0,
        unit: "levels",
    },
    Field {
        name: "x",
        get: |s| s.x,
        scale: X_RES,
        unit: "px",
    },
    Field {
        name: "y",
        get: |s| s.y,
        scale: Y_RES,
        unit: "px",
    },
    Field {
        name: "extent_x",
        get: |s| s.extent_x,
        scale: CRITICAL_SIZE,
        unit: "px",
    },
    Field {
        name: "extent_y",
        get: |s| s.extent_y,
        scale: CRITICAL_SIZE,
        unit: "px",
    },
    Field {
        name: "start",
        get: |s| s.start,
        scale: TURN_TO_PX,
        unit: "px",
    },
    Field {
        name: "rot_angle",
        get: |s| s.rot_angle,
        scale: TURN_TO_PX,
        unit: "px",
    },
    Field {
        name: "spin_angle",
        get: |s| s.spin_angle,
        scale: TURN_TO_PX,
        unit: "px",
    },
];

fn msgpack(s: &Snapshot) -> Vec<u8> {
    let mut buf = Vec::new();
    s.serialize(&mut Serializer::new(&mut buf)).unwrap();
    buf
}

fn msgpack_decode(buf: &[u8]) -> Snapshot {
    rmp_serde::from_slice(buf).unwrap()
}

fn msgpack_lz4(s: &Snapshot) -> Vec<u8> {
    lz4_flex::compress_prepend_size(&msgpack(s))
}

fn msgpack_lz4_decode(buf: &[u8]) -> Snapshot {
    msgpack_decode(&lz4_flex::decompress_size_prepended(buf).unwrap())
}

/// Encode a snapshot exactly as the render service puts it on the wire.
fn wire_encode(s: &Snapshot) -> Vec<u8> {
    let mut out = Vec::new();
    frame_codec::encode(s, &mut out);
    out
}

/// Recover a snapshot from the wire exactly as a render client does.
fn wire_decode(buf: &[u8]) -> Snapshot {
    frame_codec::decode(buf).unwrap()
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

/// The largest error any field suffered, expressed in what it changes on screen.
fn fidelity_report(original: &Snapshot, decoded: &Snapshot) -> (Vec<String>, f64) {
    let mut worst_px: f64 = 0.0;
    let mut lines = Vec::new();
    for field in &FIELDS {
        let mut max: f64 = 0.0;
        for (ol, gl) in original.layers.iter().zip(decoded.layers.iter()) {
            for (a, b) in ol.shapes.iter().zip(gl.shapes.iter()) {
                max = max.max(((field.get)(a) - (field.get)(b)).abs());
            }
        }
        let scaled = max * field.scale;
        if field.unit == "px" {
            worst_px = worst_px.max(scaled);
        }
        lines.push(format!("{} {scaled:.5} {}", field.name, field.unit));
    }
    let span_error = original
        .layers
        .iter()
        .zip(decoded.layers.iter())
        .fold(0.0_f64, |acc, (o, g)| acc.max((o.span - g.span).abs()));
    lines.push(format!("span {:.5} px", span_error * TURN_TO_PX));
    worst_px = worst_px.max(span_error * TURN_TO_PX);
    (lines, worst_px)
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

    println!("# Frame wire format measurement report\n");
    println!(
        "{FRAME_RATE} Hz, {CLIENTS} clients, release build. Error expressed against a \
         {X_RES:.0}x{Y_RES:.0} screen. Client frame budget is {FRAME_BUDGET_US:.0} µs.\n"
    );

    for (name, snap) in &fixtures {
        let n_shapes = snap.n_shapes();
        println!("\n## {name}\n");
        println!("{} layer(s), {} shapes.\n", snap.n_layers(), n_shapes);

        let plain = msgpack(snap);
        let compressed = msgpack_lz4(snap);
        let wire = wire_encode(snap);
        let baseline = plain.len();

        let rows = [
            Row {
                name: "msgpack f64, uncompressed",
                bytes: plain.len(),
                encode_us: time_ns(|| msgpack(snap)) / 1000.0,
                decode_us: time_ns(|| msgpack_decode(&plain)) / 1000.0,
            },
            Row {
                name: "msgpack f64 + lz4",
                bytes: compressed.len(),
                encode_us: time_ns(|| msgpack_lz4(snap)) / 1000.0,
                decode_us: time_ns(|| msgpack_lz4_decode(&compressed)) / 1000.0,
            },
            Row {
                name: "columnar + lz4 (ON THE WIRE)",
                bytes: wire.len(),
                encode_us: time_ns(|| wire_encode(snap)) / 1000.0,
                decode_us: time_ns(|| wire_decode(&wire)) / 1000.0,
            },
        ];

        println!("| encoding | bytes | B/shape | vs f64 | enc µs | dec µs | Mbit/s/client | × 6 |");
        println!("|---|---:|---:|---:|---:|---:|---:|---:|");
        for r in &rows {
            let per_client = r.bytes as f64 * 8.0 * FRAME_RATE / 1e6;
            println!(
                "| {} | {} | {:.1} | {:.2}× | {:.1} | {:.1} | {:.1} | {:.0} |",
                r.name,
                r.bytes,
                r.bytes as f64 / n_shapes as f64,
                baseline as f64 / r.bytes as f64,
                r.encode_us,
                r.decode_us,
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
            println!("Range audit: OUT OF RANGE -- {}", clamps.join(", "));
        }

        let (lines, worst_px) = fidelity_report(snap, &wire_decode(&wire));
        println!("\nWorst error per field: {}", lines.join(", "));
        println!("Worst geometric error: {worst_px:.5} px.");
    }
}
