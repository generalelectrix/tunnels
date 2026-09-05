//! Columnar wire format for rendered frames.
//!
//! A frame is encoded layer by layer, and within a layer each `Shape` field
//! becomes its own contiguous column. Columns are self-describing: each carries
//! an identifier, a width and an explicit byte offset, so a decoder can step
//! over a column it does not recognize and supply a default for one it expects
//! but does not find.
//!
//! Two properties are deliberate. **Every field is written for every shape**, so
//! a frame's encoded size depends only on how many shapes it holds — a look
//! costs what its shape count says it costs, with no step when a parameter stops
//! being constant. And **fixed point appears only where a value is bounded by
//! its own type and the display quantizes it anyway**: the colour components.
//! Geometry and angles stay `f32`, so no declared range can clip a value and a
//! slow rotation cannot stair-step.
//!
//! The frame is LZ4-compressed. Column layout puts like values next to each
//! other, so a field that happens to be uniform across a layer collapses to
//! almost nothing without the encoder having to detect it.

use std::sync::Arc;

use crate::{Layer, PathShape, RenderMode, ShapeGeometry, Snapshot};

/// Container framing version. It changes only when the structural layout of a
/// frame changes; adding, removing or reordering columns does not affect it.
pub const MAJOR_VERSION: u8 = 1;

/// The schema of a layer built from arc-segment shapes.
pub const SHAPE_TYPE_ARC_SEGMENT: u8 = 0;

const FRAME_HEADER_LEN: usize = 7;
/// Layer length, shape count, schema, render mode, path shape, column count,
/// two bytes of padding, then the layer's segment span as `f32`.
///
/// `f32` holds a full turn exactly, so a layer that closes into a circle still
/// tests as one after a round trip; for any narrower span the residual error is
/// a small fraction of a pixel at any on-screen radius.
const LAYER_HEADER_LEN: usize = 16;
const DESCRIPTOR_LEN: usize = 4;

/// How many bytes a field occupies per shape, and how its value is represented.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Width {
    /// 256 levels over the unit interval, for a value bounded by
    /// `UnipolarFloat` that ends up in an 8-bit channel.
    U8,
    /// 65536 levels over the unit interval, for a value bounded by `Phase`
    /// where 8 bits would band.
    U16,
    /// Full `f32`, for a value with no intrinsic bound or whose error is
    /// amplified by radius.
    F32,
}

impl Width {
    const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::F32 => 4,
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::U8 => 0,
            Self::U16 => 1,
            Self::F32 => 2,
        }
    }

    const fn from_code(c: u8) -> Self {
        match c {
            0 => Self::U8,
            1 => Self::U16,
            _ => Self::F32,
        }
    }
}

/// The identity, representation and fallback of one encodable field.
#[derive(Copy, Clone, Debug)]
pub struct FieldSpec {
    pub id: u8,
    pub name: &'static str,
    pub width: Width,
    /// Value taken when a layer does not carry this field.
    pub default: f64,
}

impl FieldSpec {
    /// Represent a value in this field's encoding.
    ///
    /// A fixed-point field is bounded to the unit interval by the type its value
    /// comes from; the clamp is a guard rather than a range choice, and it
    /// reports whether it had to act.
    pub fn encode(&self, v: f64) -> (u32, bool) {
        match self.width {
            Width::U8 => {
                let c = v.clamp(0.0, 1.0);
                ((c * 255.0).round() as u32, c != v)
            }
            Width::U16 => {
                let c = v.clamp(0.0, 1.0);
                ((c * 65535.0).round() as u32, c != v)
            }
            Width::F32 => ((v as f32).to_bits(), false),
        }
    }

    /// Recover the value an encoded representation stands for.
    pub fn decode(&self, raw: u32) -> f64 {
        match self.width {
            Width::U8 => f64::from(raw) / 255.0,
            Width::U16 => f64::from(raw) / 65535.0,
            Width::F32 => f64::from(f32::from_bits(raw)),
        }
    }
}

/// Field identifiers, stable across format revisions.
pub mod field {
    pub const LEVEL: u8 = 0;
    pub const THICKNESS: u8 = 1;
    pub const HUE: u8 = 2;
    pub const SAT: u8 = 3;
    pub const VAL: u8 = 4;
    pub const X: u8 = 5;
    pub const Y: u8 = 6;
    pub const EXTENT_X: u8 = 7;
    pub const EXTENT_Y: u8 = 8;
    pub const START: u8 = 9;
    pub const ROT_ANGLE: u8 = 11;
    pub const SPIN_ANGLE: u8 = 12;
}

/// The numeric fields of an arc-segment shape, in encoding order.
///
/// Columns are ordered widest first so that each one lands already aligned
/// for its own width, leaving a layer's size a plain function of its shape
/// count.
///
pub const NUMERIC_FIELDS: [FieldSpec; 12] = [
    FieldSpec {
        id: field::THICKNESS,
        name: "thickness",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::X,
        name: "x",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::Y,
        name: "y",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::EXTENT_X,
        name: "extent_x",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::EXTENT_Y,
        name: "extent_y",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::START,
        name: "start",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::ROT_ANGLE,
        name: "rot_angle",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::SPIN_ANGLE,
        name: "spin_angle",
        width: Width::F32,
        default: 0.0,
    },
    FieldSpec {
        id: field::HUE,
        name: "hue",
        width: Width::U16,
        default: 0.0,
    },
    FieldSpec {
        id: field::LEVEL,
        name: "level",
        width: Width::U8,
        default: 1.0,
    },
    FieldSpec {
        id: field::SAT,
        name: "sat",
        width: Width::U8,
        default: 0.0,
    },
    FieldSpec {
        id: field::VAL,
        name: "val",
        width: Width::U8,
        default: 1.0,
    },
];

/// Bytes each shape occupies across every column.
pub const BYTES_PER_SHAPE: usize = {
    let mut total = 0;
    let mut i = 0;
    while i < NUMERIC_FIELDS.len() {
        total += NUMERIC_FIELDS[i].width.bytes();
        i += 1;
    }
    total
};

fn field_value(shape: &ShapeGeometry, id: u8) -> f64 {
    match id {
        field::LEVEL => shape.level,
        field::THICKNESS => shape.thickness,
        field::HUE => shape.hue,
        field::SAT => shape.sat,
        field::VAL => shape.val,
        field::X => shape.x,
        field::Y => shape.y,
        field::EXTENT_X => shape.extent_x,
        field::EXTENT_Y => shape.extent_y,
        field::START => shape.start,
        field::ROT_ANGLE => shape.rot_angle,
        field::SPIN_ANGLE => shape.spin_angle,
        _ => 0.0,
    }
}

fn set_field_value(shape: &mut ShapeGeometry, id: u8, v: f64) {
    match id {
        field::LEVEL => shape.level = v,
        field::THICKNESS => shape.thickness = v,
        field::HUE => shape.hue = v,
        field::SAT => shape.sat = v,
        field::VAL => shape.val = v,
        field::X => shape.x = v,
        field::Y => shape.y = v,
        field::EXTENT_X => shape.extent_x = v,
        field::EXTENT_Y => shape.extent_y = v,
        field::START => shape.start = v,
        field::ROT_ANGLE => shape.rot_angle = v,
        field::SPIN_ANGLE => shape.spin_angle = v,
        _ => {}
    }
}

fn render_mode_code(m: RenderMode) -> u8 {
    match m {
        RenderMode::Arc => 0,
        RenderMode::Dot => 1,
        RenderMode::Saucer => 2,
    }
}

fn render_mode_from_code(c: u8) -> RenderMode {
    match c {
        1 => RenderMode::Dot,
        2 => RenderMode::Saucer,
        _ => RenderMode::Arc,
    }
}

fn path_shape_code(p: PathShape) -> u8 {
    match p {
        PathShape::Ellipse => 0,
        PathShape::Line => 1,
    }
}

fn path_shape_from_code(c: u8) -> PathShape {
    match c {
        1 => PathShape::Line,
        _ => PathShape::Ellipse,
    }
}

/// Diagnostics gathered while encoding a frame.
#[derive(Default, Clone, Debug)]
pub struct EncodeStats {
    /// How many values of each fixed-point field had to be clamped to the unit
    /// interval, indexed by position in `NUMERIC_FIELDS`. A non-zero entry means
    /// a value arrived outside the range its type is supposed to guarantee.
    pub clamped: [u32; 12],
    pub total_layers: u32,
    pub total_shapes: u32,
    /// Size before compression, a pure function of the layer and shape counts.
    pub uncompressed_bytes: usize,
}

/// A reason an encoded frame cannot be read.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer is shorter than the structure it claims to hold.
    Truncated { need: usize, have: usize },
    /// The frame declares a container framing version this format does not define.
    UnsupportedVersion { found: u8, supported: u8 },
    /// A column points outside the layer that declares it.
    ColumnOutOfBounds { field_id: u8, offset: usize },
    /// The compressed envelope could not be expanded.
    Decompression(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { need, have } => {
                write!(f, "frame truncated: need {need} bytes, have {have}")
            }
            Self::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "frame major version {found}, this decoder implements {supported}"
                )
            }
            Self::ColumnOutOfBounds { field_id, offset } => {
                write!(
                    f,
                    "column for field {field_id} at offset {offset} escapes its layer"
                )
            }
            Self::Decompression(e) => write!(f, "frame decompression failed: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Bytes a frame holds before compression, given how many shapes each layer has.
///
/// The result depends on nothing but those counts, which is what makes a look's
/// cost on the wire predictable from its shape budget alone.
pub fn uncompressed_size(shapes_per_layer: impl IntoIterator<Item = usize>) -> usize {
    let per_layer_fixed = LAYER_HEADER_LEN + NUMERIC_FIELDS.len() * DESCRIPTOR_LEN;
    shapes_per_layer
        .into_iter()
        .fold(align_up(FRAME_HEADER_LEN, 4), |total, n| {
            total + align_up(per_layer_fixed + n * BYTES_PER_SHAPE, 4)
        })
}

const fn align_up(v: usize, alignment: usize) -> usize {
    v.div_ceil(alignment) * alignment
}

fn pad_to(buf: &mut Vec<u8>, alignment: usize) {
    while !buf.len().is_multiple_of(alignment) {
        buf.push(0);
    }
}

/// Serialize and compress a snapshot, recording diagnostics about its contents.
///
/// `plain` is scratch space for the uncompressed form; reusing it across frames
/// keeps the encoder allocation-free in steady state.
pub fn encode_with_stats(
    snapshot: &Snapshot,
    plain: &mut Vec<u8>,
    out: &mut Vec<u8>,
    stats: &mut EncodeStats,
) {
    plain.clear();
    plain.push(MAJOR_VERSION);
    plain.extend_from_slice(&(snapshot.frame_number as u32).to_le_bytes());
    plain.extend_from_slice(&(snapshot.layers.len() as u16).to_le_bytes());

    for layer in &snapshot.layers {
        pad_to(plain, 4);
        encode_layer(layer, plain, stats);
        stats.total_layers += 1;
        stats.total_shapes += layer.n_shapes() as u32;
    }
    stats.uncompressed_bytes = plain.len();

    *out = lz4_flex::compress_prepend_size(plain);
}

/// Serialize and compress a snapshot into the provided buffer.
pub fn encode(snapshot: &Snapshot, out: &mut Vec<u8>) {
    let mut plain = Vec::new();
    let mut stats = EncodeStats::default();
    encode_with_stats(snapshot, &mut plain, out, &mut stats);
}

fn encode_layer(layer: &Layer, buf: &mut Vec<u8>, stats: &mut EncodeStats) {
    let layer_start = buf.len();
    let shapes = &layer.shapes;
    let n_shapes = shapes.len();
    let n_columns = NUMERIC_FIELDS.len();

    buf.extend_from_slice(&0u32.to_le_bytes()); // layer_len, patched below
    buf.extend_from_slice(&(n_shapes as u16).to_le_bytes());
    buf.push(SHAPE_TYPE_ARC_SEGMENT);
    buf.push(render_mode_code(layer.render_mode));
    buf.push(path_shape_code(layer.path_shape));
    buf.push(n_columns as u8);
    buf.extend_from_slice(&[0u8; 2]); // reserved, keeps the table 4-aligned
    buf.extend_from_slice(&(layer.span as f32).to_le_bytes());

    let descriptors = buf.len();
    buf.resize(descriptors + n_columns * DESCRIPTOR_LEN, 0);

    for (i, spec) in NUMERIC_FIELDS.iter().enumerate() {
        pad_to(buf, spec.width.bytes());
        let offset = buf.len() - layer_start;
        for shape in shapes {
            let (raw, clamped) = spec.encode(field_value(shape, spec.id));
            if clamped {
                stats.clamped[i] += 1;
            }
            match spec.width {
                Width::U8 => buf.push(raw as u8),
                Width::U16 => buf.extend_from_slice(&(raw as u16).to_le_bytes()),
                Width::F32 => buf.extend_from_slice(&raw.to_le_bytes()),
            }
        }
        let d = descriptors + i * DESCRIPTOR_LEN;
        buf[d] = spec.id;
        buf[d + 1] = spec.width.code();
        buf[d + 2..d + 4].copy_from_slice(&(offset as u16).to_le_bytes());
    }

    pad_to(buf, 4);
    let layer_len = (buf.len() - layer_start) as u32;
    buf[layer_start..layer_start + 4].copy_from_slice(&layer_len.to_le_bytes());
}

fn read_u16(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn read_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Decompress and reconstruct a snapshot from its encoded form.
///
/// A column whose field is not recognized is skipped, a field the buffer does
/// not carry takes the default from its specification, and a layer whose schema
/// is not implemented is dropped rather than failing the frame.
pub fn decode(compressed: &[u8]) -> Result<Snapshot, DecodeError> {
    let buf = lz4_flex::decompress_size_prepended(compressed)
        .map_err(|e| DecodeError::Decompression(e.to_string()))?;
    decode_plain(&buf)
}

fn decode_plain(buf: &[u8]) -> Result<Snapshot, DecodeError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(DecodeError::Truncated {
            need: FRAME_HEADER_LEN,
            have: buf.len(),
        });
    }
    if buf[0] != MAJOR_VERSION {
        return Err(DecodeError::UnsupportedVersion {
            found: buf[0],
            supported: MAJOR_VERSION,
        });
    }
    let frame_number = u64::from(read_u32(buf, 1));
    let n_layers = usize::from(read_u16(buf, 5));

    let mut layers = Vec::with_capacity(n_layers);
    let mut cursor = FRAME_HEADER_LEN;
    for _ in 0..n_layers {
        cursor += (4 - cursor % 4) % 4;
        if cursor + LAYER_HEADER_LEN > buf.len() {
            return Err(DecodeError::Truncated {
                need: cursor + LAYER_HEADER_LEN,
                have: buf.len(),
            });
        }
        let layer_len = read_u32(buf, cursor) as usize;
        if layer_len < LAYER_HEADER_LEN || cursor + layer_len > buf.len() {
            return Err(DecodeError::Truncated {
                need: cursor + layer_len,
                have: buf.len(),
            });
        }
        let layer = &buf[cursor..cursor + layer_len];
        cursor += layer_len;

        if layer[6] != SHAPE_TYPE_ARC_SEGMENT {
            continue;
        }
        layers.push(Arc::new(decode_layer(layer)?));
    }

    Ok(Snapshot {
        frame_number,
        layers,
    })
}

fn decode_layer(layer: &[u8]) -> Result<Layer, DecodeError> {
    let n_shapes = usize::from(read_u16(layer, 4));
    let render_mode = render_mode_from_code(layer[7]);
    let path_shape = path_shape_from_code(layer[8]);
    let n_columns = usize::from(layer[9]);
    let span = f64::from(f32::from_bits(read_u32(layer, 12)));

    let mut shapes = vec![
        ShapeGeometry {
            level: 0.0,
            thickness: 0.0,
            hue: 0.0,
            sat: 0.0,
            val: 0.0,
            x: 0.0,
            y: 0.0,
            extent_x: 0.0,
            extent_y: 0.0,
            start: 0.0,
            rot_angle: 0.0,
            spin_angle: 0.0,
        };
        n_shapes
    ];
    for spec in &NUMERIC_FIELDS {
        for shape in &mut shapes {
            set_field_value(shape, spec.id, spec.default);
        }
    }

    let table = LAYER_HEADER_LEN;
    if table + n_columns * DESCRIPTOR_LEN > layer.len() {
        return Err(DecodeError::Truncated {
            need: table + n_columns * DESCRIPTOR_LEN,
            have: layer.len(),
        });
    }

    for i in 0..n_columns {
        let d = table + i * DESCRIPTOR_LEN;
        let field_id = layer[d];
        let width = Width::from_code(layer[d + 1]);
        let offset = usize::from(read_u16(layer, d + 2));
        if offset + n_shapes * width.bytes() > layer.len() {
            return Err(DecodeError::ColumnOutOfBounds { field_id, offset });
        }

        let read = |k: usize| -> u32 {
            let at = offset + k * width.bytes();
            match width {
                Width::U8 => u32::from(layer[at]),
                Width::U16 => u32::from(read_u16(layer, at)),
                Width::F32 => read_u32(layer, at),
            }
        };

        let Some(spec) = NUMERIC_FIELDS.iter().find(|s| s.id == field_id) else {
            continue;
        };
        for (k, shape) in shapes.iter_mut().enumerate() {
            set_field_value(shape, field_id, spec.decode(read(k)));
        }
    }

    Ok(Layer::new(render_mode, path_shape, span, shapes))
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

    /// Build a snapshot whose layers all draw the same way.
    fn snapshot_of(layers: Vec<Vec<ShapeGeometry>>) -> Snapshot {
        snapshot_of_modes(
            layers
                .into_iter()
                .map(|s| (RenderMode::Arc, PathShape::Ellipse, 0.1, s))
                .collect(),
        )
    }

    fn snapshot_of_modes(
        layers: Vec<(RenderMode, PathShape, f64, Vec<ShapeGeometry>)>,
    ) -> Snapshot {
        Snapshot {
            frame_number: 7,
            layers: layers
                .into_iter()
                .map(|(r, p, span, s)| Arc::new(Layer::new(r, p, span, s)))
                .collect(),
        }
    }

    fn plain_of(snapshot: &Snapshot) -> Vec<u8> {
        let mut buf = Vec::new();
        encode(snapshot, &mut buf);
        lz4_flex::decompress_size_prepended(&buf).unwrap()
    }

    /// Round-trip fidelity across mixed modes and degenerate layer sizes.
    ///
    /// Colour survives to within its 8-bit step; everything else is `f32` and
    /// survives to `f32` precision.
    #[test]
    fn round_trip() {
        let uniform: Vec<ShapeGeometry> = (0..64).map(|i| shape(f64::from(i) / 64.0)).collect();
        let snapshot = snapshot_of_modes(vec![
            (RenderMode::Arc, PathShape::Ellipse, 0.1, uniform.clone()),
            (RenderMode::Saucer, PathShape::Line, 0.25, uniform),
            (RenderMode::Dot, PathShape::Ellipse, 1.0, vec![]),
            (RenderMode::Arc, PathShape::Line, 0.5, vec![shape(0.3)]),
        ]);

        let mut buf = Vec::new();
        encode(&snapshot, &mut buf);
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
                for spec in &NUMERIC_FIELDS {
                    let a = field_value(orig, spec.id);
                    let b = field_value(got, spec.id);
                    let tolerance = match spec.width {
                        Width::U8 => 1.0 / 255.0,
                        Width::U16 => 1.0 / 65535.0,
                        Width::F32 => a.abs() * f64::from(f32::EPSILON) + 1e-9,
                    };
                    assert!(
                        (a - b).abs() <= tolerance,
                        "{} drifted beyond its representation: {a} vs {b}",
                        spec.name,
                    );
                }
            }
        }
    }

    /// A frame costs what its shape count says it costs, whatever the content.
    ///
    /// This is the property that makes the network budget predictable, so it is
    /// asserted rather than left to hold by accident.
    #[test]
    fn uncompressed_size_depends_only_on_shape_count() {
        let uniform = vec![shape(0.4); 100];
        let varied: Vec<ShapeGeometry> = (0..100).map(|i| shape(f64::from(i) / 100.0)).collect();

        let flat = plain_of(&snapshot_of(vec![uniform])).len();
        let busy = plain_of(&snapshot_of(vec![varied])).len();
        assert_eq!(flat, busy, "content changed the uncompressed size");
        assert_eq!(
            flat,
            uncompressed_size([100]),
            "size is not predicted by shape count"
        );
    }

    /// A value outside the unit interval saturates and is reported rather than
    /// wrapping or panicking. Only the colour fields have a range to leave.
    #[test]
    fn out_of_range_colour_is_clamped_and_counted() {
        let mut s = shape(0.0);
        s.sat = 4.0;
        s.thickness = 5000.0;
        s.x = -900.0;
        let snapshot = snapshot_of(vec![vec![s]]);

        let mut plain = Vec::new();
        let mut buf = Vec::new();
        let mut stats = EncodeStats::default();
        encode_with_stats(&snapshot, &mut plain, &mut buf, &mut stats);

        let sat_idx = NUMERIC_FIELDS
            .iter()
            .position(|f| f.id == field::SAT)
            .unwrap();
        let thickness_idx = NUMERIC_FIELDS
            .iter()
            .position(|f| f.id == field::THICKNESS)
            .unwrap();
        assert_eq!(stats.clamped[sat_idx], 1);
        assert_eq!(
            stats.clamped[thickness_idx], 0,
            "an f32 field has no range to leave"
        );

        let got = &decode(&buf).unwrap().layers[0].shapes[0];
        assert_eq!(got.sat, 1.0, "saturates at the top of the unit interval");
        assert_eq!(got.thickness, 5000.0, "an unbounded field is carried as-is");
        assert_eq!(got.x, -900.0);
    }

    /// A layer spanning exactly one turn still reads as one turn after a round
    /// trip, at every starting angle.
    ///
    /// The rendering path selects a closed circle by testing the span against
    /// one full turn, so a span that came back even slightly short would
    /// silently change what is drawn.
    #[test]
    fn full_turn_span_survives_exactly() {
        for i in 0..2000 {
            let start = f64::from(i) / 2000.0;
            let mut s = shape(0.0);
            s.start = start;
            let snapshot =
                snapshot_of_modes(vec![(RenderMode::Arc, PathShape::Ellipse, 1.0, vec![s])]);

            let mut buf = Vec::new();
            encode(&snapshot, &mut buf);
            let got = &decode(&buf).unwrap().layers[0];
            assert_eq!(got.span, 1.0, "full turn came back as {}", got.span);
        }
    }

    /// A frame with different container framing is refused by version rather
    /// than misread, a truncated buffer is reported rather than read past its
    /// end, and corruption never panics.
    #[test]
    fn rejects_unreadable_frames() {
        let snapshot = snapshot_of(vec![vec![shape(0.1); 16]]);
        let mut buf = Vec::new();
        encode(&snapshot, &mut buf);

        let mut plain = lz4_flex::decompress_size_prepended(&buf).unwrap();
        plain[0] = MAJOR_VERSION.wrapping_add(1);
        assert_eq!(
            decode_plain(&plain).expect_err("a future major version must be refused"),
            DecodeError::UnsupportedVersion {
                found: MAJOR_VERSION.wrapping_add(1),
                supported: MAJOR_VERSION
            }
        );

        plain[0] = MAJOR_VERSION;
        plain.truncate(plain.len() - 8);
        assert!(
            matches!(decode_plain(&plain), Err(DecodeError::Truncated { .. })),
            "a short buffer must be refused"
        );

        let mut garbage = buf.clone();
        let n = garbage.len();
        garbage[n / 2] ^= 0xFF;
        let _ = decode(&garbage);
    }

    /// An unrecognized field is stepped over and its neighbours still decode; a
    /// field the buffer omits takes its documented default.
    #[test]
    fn unknown_and_missing_fields() {
        let shapes: Vec<ShapeGeometry> = (0..8).map(|i| shape(f64::from(i) / 8.0)).collect();
        let snapshot = snapshot_of(vec![shapes.clone()]);
        let mut plain = plain_of(&snapshot);

        let layer_start = FRAME_HEADER_LEN + (4 - FRAME_HEADER_LEN % 4) % 4;
        let table = layer_start + LAYER_HEADER_LEN;
        let n_columns = usize::from(plain[layer_start + 9]);
        let hue_descriptor = (0..n_columns)
            .map(|i| table + i * DESCRIPTOR_LEN)
            .find(|&d| plain[d] == field::HUE)
            .expect("hue is encoded");
        plain[hue_descriptor] = 200;

        let decoded = decode_plain(&plain).expect("an unknown column must not fail the frame");
        let hue_spec = NUMERIC_FIELDS.iter().find(|s| s.id == field::HUE).unwrap();
        for (orig, got) in shapes.iter().zip(decoded.layers[0].shapes.iter()) {
            assert_eq!(
                got.hue, hue_spec.default,
                "missing hue falls back to its default"
            );
            assert!(
                (orig.x - got.x).abs() <= 1e-6,
                "neighbouring columns still decode: {} vs {}",
                orig.x,
                got.x
            );
        }
    }

    /// A layer whose schema this decoder does not implement is skipped whole,
    /// and the layers around it still decode.
    #[test]
    fn unknown_shape_type_skips_only_that_layer() {
        let a: Vec<ShapeGeometry> = (0..8).map(|i| shape(f64::from(i) / 8.0)).collect();
        let b: Vec<ShapeGeometry> = (0..5).map(|i| shape(f64::from(i) / 5.0)).collect();
        let snapshot = snapshot_of(vec![a, b.clone()]);
        let mut plain = plain_of(&snapshot);

        let layer_start = FRAME_HEADER_LEN + (4 - FRAME_HEADER_LEN % 4) % 4;
        plain[layer_start + 6] = 77; // a schema from the future

        let decoded = decode_plain(&plain).expect("an unknown schema must not fail the frame");
        assert_eq!(
            decoded.layers.len(),
            1,
            "the unknown layer is dropped, not the frame"
        );
        assert_eq!(
            decoded.layers[0].n_shapes(),
            b.len(),
            "the following layer still decodes"
        );
    }
}
