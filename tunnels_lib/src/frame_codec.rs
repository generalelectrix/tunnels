//! Wire format for rendered frames.
//!
//! A frame crosses the network as a compressed sequence of layers, each holding
//! one record per shape. The records are structurally the in-memory shapes, but
//! every field is carried in the narrowest representation that does not cost
//! anything visible: a value the type system bounds to the unit interval
//! becomes fixed point at the resolution the display quantizes to, and every
//! other field is `f32`, which resolves a fraction of a pixel at any on-screen
//! radius.
//!
//! The encoded frame is LZ4-compressed. A field that happens to be uniform
//! across a layer costs almost nothing after compression without the encoder
//! having to detect it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Layer, PathShape, RenderMode, ShapeGeometry, Snapshot};

/// 256 levels over the unit interval, for a value that ends up in an 8-bit
/// channel. A value outside the interval saturates rather than wrapping.
fn to_unit_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn from_unit_u8(v: u8) -> f64 {
    f64::from(v) / 255.0
}

/// 65536 levels over the unit interval, for a value that traverses several
/// 8-bit ramps and would band at 256. A value outside the interval saturates.
fn to_unit_u16(v: f64) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn from_unit_u16(v: u16) -> f64 {
    f64::from(v) / 65535.0
}

/// A single shape as it travels to a render client.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct WireShape {
    level: u8,
    thickness: f32,
    hue: u16,
    sat: u8,
    val: u8,
    x: f32,
    y: f32,
    extent_x: f32,
    extent_y: f32,
    start: f32,
    rot_angle: f32,
    spin_angle: f32,
}

impl From<&ShapeGeometry> for WireShape {
    fn from(s: &ShapeGeometry) -> Self {
        Self {
            level: to_unit_u8(s.level),
            thickness: s.thickness as f32,
            hue: to_unit_u16(s.hue),
            sat: to_unit_u8(s.sat),
            val: to_unit_u8(s.val),
            x: s.x as f32,
            y: s.y as f32,
            extent_x: s.extent_x as f32,
            extent_y: s.extent_y as f32,
            start: s.start as f32,
            rot_angle: s.rot_angle as f32,
            spin_angle: s.spin_angle as f32,
        }
    }
}

impl From<&WireShape> for ShapeGeometry {
    fn from(s: &WireShape) -> Self {
        Self {
            level: from_unit_u8(s.level),
            thickness: f64::from(s.thickness),
            hue: from_unit_u16(s.hue),
            sat: from_unit_u8(s.sat),
            val: from_unit_u8(s.val),
            x: f64::from(s.x),
            y: f64::from(s.y),
            extent_x: f64::from(s.extent_x),
            extent_y: f64::from(s.extent_y),
            start: f64::from(s.start),
            rot_angle: f64::from(s.rot_angle),
            spin_angle: f64::from(s.spin_angle),
        }
    }
}

/// A run of shapes drawn the same way, as it travels to a render client.
///
/// `f32` holds a full turn exactly, so a layer that closes into a circle still
/// tests as one after a round trip; for any narrower span the residual error is
/// a small fraction of a pixel at any on-screen radius.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct WireLayer {
    render_mode: RenderMode,
    path_shape: PathShape,
    span: f32,
    shapes: Vec<WireShape>,
}

/// A complete single-frame video snapshot, as it travels to a render client.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct WireSnapshot {
    frame_number: u64,
    layers: Vec<WireLayer>,
}

impl From<&Snapshot> for WireSnapshot {
    fn from(s: &Snapshot) -> Self {
        Self {
            frame_number: s.frame_number,
            layers: s
                .layers
                .iter()
                .map(|l| WireLayer {
                    render_mode: l.render_mode,
                    path_shape: l.path_shape,
                    span: l.span as f32,
                    shapes: l.shapes.iter().map(WireShape::from).collect(),
                })
                .collect(),
        }
    }
}

impl From<WireSnapshot> for Snapshot {
    fn from(s: WireSnapshot) -> Self {
        Self {
            frame_number: s.frame_number,
            layers: s
                .layers
                .into_iter()
                .map(|l| {
                    Arc::new(Layer::new(
                        l.render_mode,
                        l.path_shape,
                        f64::from(l.span),
                        l.shapes.iter().map(ShapeGeometry::from).collect(),
                    ))
                })
                .collect(),
        }
    }
}

/// A reason a frame cannot be put on the wire.
#[derive(Debug)]
pub struct EncodeError(String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame serialization failed: {}", self.0)
    }
}

impl std::error::Error for EncodeError {}

/// A reason an encoded frame cannot be read.
#[derive(Debug)]
pub enum DecodeError {
    /// The compressed envelope could not be expanded.
    Decompression(String),
    /// The expanded bytes are not a frame.
    Deserialization(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decompression(e) => write!(f, "frame decompression failed: {e}"),
            Self::Deserialization(e) => write!(f, "frame deserialization failed: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Serialize and compress a snapshot into the provided buffer.
pub fn encode(snapshot: &Snapshot, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    let plain =
        rmp_serde::to_vec(&WireSnapshot::from(snapshot)).map_err(|e| EncodeError(e.to_string()))?;
    *out = lz4_flex::compress_prepend_size(&plain);
    Ok(())
}

/// Decompress and reconstruct a snapshot from its encoded form.
pub fn decode(compressed: &[u8]) -> Result<Snapshot, DecodeError> {
    let plain = lz4_flex::decompress_size_prepended(compressed)
        .map_err(|e| DecodeError::Decompression(e.to_string()))?;
    let wire: WireSnapshot =
        rmp_serde::from_slice(&plain).map_err(|e| DecodeError::Deserialization(e.to_string()))?;
    Ok(wire.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(seed: f64) -> ShapeGeometry {
        ShapeGeometry {
            level: 0.5,
            thickness: 0.25 + seed,
            hue: seed.fract(),
            sat: 0.75,
            val: 1.0,
            x: -0.5 + seed,
            y: 0.25,
            extent_x: 1.0 + seed,
            extent_y: 0.5,
            start: seed.fract(),
            rot_angle: 0.125,
            spin_angle: 0.0,
        }
    }

    fn snapshot_of(layers: Vec<(RenderMode, PathShape, f64, Vec<ShapeGeometry>)>) -> Snapshot {
        Snapshot {
            frame_number: 7,
            layers: layers
                .into_iter()
                .map(|(r, p, span, s)| Arc::new(Layer::new(r, p, span, s)))
                .collect(),
        }
    }

    /// Round-trip fidelity across mixed modes and degenerate layer sizes.
    ///
    /// Colour survives to within its fixed-point step; everything else is `f32`
    /// and survives to `f32` precision.
    #[test]
    fn round_trip() {
        let uniform: Vec<ShapeGeometry> = (0..64).map(|i| shape(f64::from(i) / 64.0)).collect();
        let snapshot = snapshot_of(vec![
            (RenderMode::Arc, PathShape::Ellipse, 0.1, uniform.clone()),
            (RenderMode::Saucer, PathShape::Line, 0.25, uniform),
            (RenderMode::Dot, PathShape::Ellipse, 1.0, vec![]),
            (RenderMode::Arc, PathShape::Line, 0.5, vec![shape(0.3)]),
        ]);

        let mut buf = Vec::new();
        encode(&snapshot, &mut buf).expect("a snapshot must encode");
        let decoded = decode(&buf).expect("round trip should decode");

        assert_eq!(decoded.frame_number, snapshot.frame_number);
        assert_eq!(decoded.layers.len(), snapshot.layers.len());

        for (orig_layer, got_layer) in snapshot.layers.iter().zip(decoded.layers.iter()) {
            assert_eq!(orig_layer.n_shapes(), got_layer.n_shapes());
            assert_eq!(orig_layer.render_mode, got_layer.render_mode);
            assert_eq!(orig_layer.path_shape, got_layer.path_shape);
            assert_eq!(
                orig_layer.span as f32, got_layer.span as f32,
                "a layer's segment span must survive to f32 precision"
            );
            for (orig, got) in orig_layer.shapes.iter().zip(got_layer.shapes.iter()) {
                let unit_u8 = 1.0 / 255.0;
                assert!((orig.level - got.level).abs() <= unit_u8);
                assert!((orig.sat - got.sat).abs() <= unit_u8);
                assert!((orig.val - got.val).abs() <= unit_u8);
                assert!((orig.hue - got.hue).abs() <= 1.0 / 65535.0);
                for (a, b) in [
                    (orig.thickness, got.thickness),
                    (orig.x, got.x),
                    (orig.y, got.y),
                    (orig.extent_x, got.extent_x),
                    (orig.extent_y, got.extent_y),
                    (orig.start, got.start),
                    (orig.rot_angle, got.rot_angle),
                    (orig.spin_angle, got.spin_angle),
                ] {
                    assert!(
                        (a - b).abs() <= a.abs() * f64::from(f32::EPSILON) + 1e-9,
                        "an f32 field drifted beyond its representation: {a} vs {b}"
                    );
                }
            }
        }
    }

    /// A layer spanning exactly one turn still reads as one turn after a round
    /// trip.
    ///
    /// The rendering path selects a closed circle by testing the span against
    /// one full turn, so a span that came back even slightly short would
    /// silently change what is drawn.
    #[test]
    fn full_turn_span_survives_exactly() {
        let snapshot = snapshot_of(vec![(
            RenderMode::Arc,
            PathShape::Ellipse,
            1.0,
            vec![shape(0.0)],
        )]);
        let mut buf = Vec::new();
        encode(&snapshot, &mut buf).expect("a snapshot must encode");
        let got = decode(&buf).expect("round trip should decode");
        assert_eq!(got.layers[0].span, 1.0);
    }

    /// A value outside the unit interval saturates rather than wrapping, and a
    /// field with no declared range is carried as it stands.
    #[test]
    fn out_of_range_colour_saturates() {
        let mut s = shape(0.0);
        s.sat = 4.0;
        s.hue = -1.0;
        s.thickness = 5000.0;
        s.x = -900.0;
        let snapshot = snapshot_of(vec![(RenderMode::Arc, PathShape::Ellipse, 0.1, vec![s])]);

        let mut buf = Vec::new();
        encode(&snapshot, &mut buf).expect("a snapshot must encode");
        let got = &decode(&buf).expect("round trip should decode").layers[0].shapes[0];

        assert_eq!(got.sat, 1.0, "saturates at the top of the unit interval");
        assert_eq!(got.hue, 0.0, "saturates at the bottom of the unit interval");
        assert_eq!(got.thickness, 5000.0, "an unbounded field is carried as-is");
        assert_eq!(got.x, -900.0);
    }

    /// Corrupt bytes are reported rather than read as a frame, and never panic.
    #[test]
    fn rejects_unreadable_frames() {
        let snapshot = snapshot_of(vec![(
            RenderMode::Arc,
            PathShape::Ellipse,
            0.1,
            vec![shape(0.1); 16],
        )]);
        let mut buf = Vec::new();
        encode(&snapshot, &mut buf).expect("a snapshot must encode");

        assert!(
            matches!(decode(&buf[..4]), Err(DecodeError::Decompression(_))),
            "a truncated envelope must be refused"
        );

        let plain = lz4_flex::decompress_size_prepended(&buf).expect("its own output decompresses");
        let short = lz4_flex::compress_prepend_size(&plain[..plain.len() / 2]);
        assert!(
            matches!(decode(&short), Err(DecodeError::Deserialization(_))),
            "a truncated frame body must be refused"
        );

        for i in 0..buf.len() {
            let mut garbage = buf.clone();
            garbage[i] ^= 0xFF;
            let _ = decode(&garbage);
        }
    }
}
