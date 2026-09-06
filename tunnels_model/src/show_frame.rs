//! The complete state a render consumes for one frame.

use std::error::Error;
use std::fmt;

use lz4_flex::block::{
    CompressError, DecompressError, compress_into, decompress_into, get_maximum_output_size,
    uncompressed_size,
};
use serde::{Deserialize, Serialize};
use tunnels_lib::number::UnipolarFloat;

use crate::clock_bank::StaticClockBank;
use crate::mixer::Mixer;
use crate::palette::ColorPalette;
use crate::position_bank::PositionBank;
use crate::render_context::RenderContext;

/// The version of the model an encoded show frame describes.
///
/// **Bump this whenever the shape of `ShowFrame`, or of anything reachable
/// from it, changes.** The encoding is tagless — the schema is the Rust type
/// and travels nowhere — so bytes written against a different shape decode
/// into a structurally valid but wrong show instead of failing. A version
/// carried alongside them is the only thing that says the two ends agree.
///
/// What it catches: a machine running a binary from a different build of the
/// model than the one publishing to it, which otherwise renders silently wrong
/// geometry for a whole show.
///
/// What it does not catch: a change to the model that nobody bumped this for.
/// Two builds that disagree about the shape while agreeing about the number
/// decode each other's bytes exactly as badly as they would with no version at
/// all.
const WIRE_VERSION: u8 = 1;

/// The bytes an encoded show frame begins with, ahead of its version.
const FRAME_MAGIC: [u8; 3] = *b"TNL";

/// The size of the header an encoded show frame begins with.
const FRAME_HEADER_LEN: usize = FRAME_MAGIC.len() + 1;

/// Everything a render reads to draw one frame, and nothing else.
///
/// The beam model carries its own integrated per-frame state, so a frame is
/// self-contained: rendering it does not depend on any state update having run
/// first, and the same frame always expands into the same geometry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShowFrame {
    pub mixer: Mixer,
    pub clocks: StaticClockBank,
    pub palette: ColorPalette,
    pub positions: PositionBank,
    pub audio_envelope: UnipolarFloat,
}

/// One frame of show state, held by reference.
///
/// Field for field this is `ShowFrame`, and serde writes a reference exactly
/// as it writes the value behind it, so the two encode to identical bytes.
/// Only `clocks` differs in kind: a static clock bank is computed from the
/// live one rather than stored, so a reference to one has nothing to name.
///
/// A frame only serializes in this form. Decoding always yields the owned one.
#[derive(Debug, Serialize)]
pub struct ShowFrameRef<'a> {
    pub mixer: &'a Mixer,
    pub clocks: StaticClockBank,
    pub palette: &'a ColorPalette,
    pub positions: &'a PositionBank,
    pub audio_envelope: UnipolarFloat,
}

/// The largest payload a frame is allowed to expand into.
///
/// A frame is a few kilobytes. The ceiling is here so that a corrupted length
/// prefix asks for a rejected decode rather than a multi-gigabyte allocation.
const MAX_DECODED_LEN: usize = 8 * 1024 * 1024;

/// The scratch a show frame is serialized and compressed in.
///
/// The buffers are held rather than produced, so encoding a frame no larger
/// than the largest one encoded before it writes into memory that is already
/// there, rather than asking the allocator for more.
///
/// One allocation a frame is the floor regardless of the scratch: the LZ4
/// compressor builds a 4096-entry hash table on every call and takes none to
/// reuse, so an encode that allocates nothing at all would need a compressor
/// vendored to accept one.
#[derive(Debug, Default)]
pub struct FrameEncoder {
    /// The serialized frame, ahead of compression.
    plain: Vec<u8>,
    /// The header, and the compressed frame behind it.
    wire: Vec<u8>,
}

impl FrameEncoder {
    /// Serialize a frame into the bytes `ShowFrame::decode` reads, which stand
    /// until the next frame is encoded.
    ///
    /// The bytes carry a four byte header — magic, then the wire version —
    /// ahead of the compressed frame, so that bytes from a build holding a
    /// different model are recognized as such rather than decoded. The
    /// compressed frame opens with its uncompressed length, little-endian, as
    /// an LZ4 block does.
    pub fn encode(&mut self, frame: &ShowFrameRef) -> Result<&[u8], FrameCodecError> {
        self.plain.clear();
        postcard::to_io(frame, &mut self.plain).map_err(FrameCodecError::Serialize)?;
        let plain_len = u32::try_from(self.plain.len())
            .map_err(|_| FrameCodecError::TooLarge(self.plain.len()))?;

        self.wire.clear();
        self.wire.extend_from_slice(&FRAME_MAGIC);
        self.wire.push(WIRE_VERSION);
        self.wire.extend_from_slice(&plain_len.to_le_bytes());

        // Compression writes into the buffer directly, so it needs room for
        // the worst case the block format can produce before it starts.
        let block = self.wire.len();
        self.wire
            .resize(block + get_maximum_output_size(self.plain.len()), 0);
        let compressed = compress_into(&self.plain, &mut self.wire[block..])
            .map_err(FrameCodecError::Compress)?;
        self.wire.truncate(block + compressed);
        Ok(&self.wire)
    }
}

/// The scratch a show frame is decompressed in.
///
/// The buffer is held rather than produced, so decoding a frame no larger than
/// the largest one decoded before it expands into memory that is already
/// there, rather than asking the allocator for more.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    /// The decompressed frame, ahead of deserialization.
    plain: Vec<u8>,
}

impl FrameDecoder {
    /// Recover a frame from the wire bytes `FrameEncoder::encode` produces.
    ///
    /// Every way the bytes can be wrong is an error, never a panic: a mangled
    /// or truncated payload costs the frame it arrived in and nothing more.
    /// That includes bytes asserting more nesting than a look is allowed,
    /// which are refused before the nesting is followed rather than after;
    /// bytes carrying a wire version other than this build's, which are
    /// refused before they can become a plausible-looking wrong show; and
    /// bytes claiming an expansion past the limit, which are refused before
    /// anything is sized to them.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<ShowFrame, FrameCodecError> {
        let body = strip_header(bytes)?;
        let (decoded_len, compressed) =
            uncompressed_size(body).map_err(FrameCodecError::Decompress)?;
        if decoded_len > MAX_DECODED_LEN {
            return Err(FrameCodecError::Oversized {
                declared: decoded_len,
                limit: MAX_DECODED_LEN,
            });
        }

        // Only past the limit does the declared length, which came off the
        // wire, get to decide how much memory to hold.
        self.plain.resize(decoded_len, 0);
        let plain_len =
            decompress_into(compressed, &mut self.plain).map_err(FrameCodecError::Decompress)?;
        postcard::from_bytes(&self.plain[..plain_len]).map_err(FrameCodecError::Deserialize)
    }
}

impl ShowFrame {
    /// Borrow the sidecar state as the context a beam resolves against.
    pub fn render_context(&self) -> RenderContext<'_> {
        RenderContext {
            clocks: &self.clocks,
            palette: &self.palette,
            positions: &self.positions,
            audio_envelope: self.audio_envelope,
        }
    }

    /// Recover a frame from the wire bytes `FrameEncoder::encode` produces.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameCodecError> {
        FrameDecoder::default().decode(bytes)
    }
}

/// Check the header the wire bytes open with, yielding the frame after it.
fn strip_header(bytes: &[u8]) -> Result<&[u8], FrameCodecError> {
    let (header, body) = bytes
        .split_at_checked(FRAME_HEADER_LEN)
        .ok_or(FrameCodecError::NotAFrame)?;
    let (magic, version) = header.split_at(FRAME_MAGIC.len());
    if magic != FRAME_MAGIC {
        return Err(FrameCodecError::NotAFrame);
    }
    if version[0] != WIRE_VERSION {
        return Err(FrameCodecError::WireVersion {
            found: version[0],
            expected: WIRE_VERSION,
        });
    }
    Ok(body)
}

/// Why a show frame could not be put on the wire, or recovered from it.
#[derive(Debug)]
pub enum FrameCodecError {
    /// The frame could not be serialized.
    Serialize(postcard::Error),
    /// The frame is too large to name its own length on the wire.
    TooLarge(usize),
    /// The serialized frame could not be compressed.
    Compress(CompressError),
    /// The bytes do not open the way an encoded frame does.
    NotAFrame,
    /// The bytes describe a model of a different vintage than this one.
    WireVersion { found: u8, expected: u8 },
    /// The compressed bytes could not be expanded.
    Decompress(DecompressError),
    /// The bytes claim to expand to more than a frame is allowed to occupy.
    Oversized { declared: usize, limit: usize },
    /// The expanded bytes describe something other than a frame.
    Deserialize(postcard::Error),
}

impl fmt::Display for FrameCodecError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Serialize(e) => write!(f, "could not serialize a show frame: {e}"),
            Self::TooLarge(len) => write!(
                f,
                "a show frame serializing to {len} bytes is longer than a wire length can describe"
            ),
            Self::Compress(e) => write!(f, "could not compress a show frame: {e}"),
            Self::NotAFrame => write!(f, "these bytes are not an encoded show frame"),
            Self::WireVersion { found, expected } => write!(
                f,
                "a show frame at wire version {found} cannot be read by a build speaking wire version {expected}: one end is running a stale binary"
            ),
            Self::Decompress(e) => write!(f, "could not decompress a show frame: {e}"),
            Self::Oversized { declared, limit } => write!(
                f,
                "a show frame claiming to decompress to {declared} bytes exceeds the {limit} byte limit"
            ),
            Self::Deserialize(e) => write!(f, "could not deserialize a show frame: {e}"),
        }
    }
}

impl Error for FrameCodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialize(e) | Self::Deserialize(e) => Some(e),
            Self::Decompress(e) => Some(e),
            Self::Compress(e) => Some(e),
            Self::TooLarge(_)
            | Self::Oversized { .. }
            | Self::NotAFrame
            | Self::WireVersion { .. } => None,
        }
    }
}

/// Show frames sized to exercise the model, for holding a render to its output.
#[cfg(any(test, feature = "fixtures"))]
pub mod fixture {
    use arrayvec::ArrayVec;
    use tunnels_lib::color::Hsv;
    use tunnels_lib::number::Phase;

    use crate::beam::Beam;
    use crate::clock::StaticClock;
    use crate::clock_bank::{ClockIdx, MAX_CLOCKS};
    use crate::look::Look;
    use crate::mixer::{ChannelIdx, VideoChannel};
    use crate::palette::{
        ColorPaletteIdx, ControlMessage as PaletteControlMessage,
        EmitStateChange as EmitPaletteStateChange, StateChange as PaletteStateChange,
    };
    use crate::position_bank::{Position, PositionIdx};
    use crate::tunnel::Tunnel;
    use crate::tunnel::fixture::{bind_to_frame_state, configure_max_variation};
    use std::time::Duration;

    use super::*;

    /// The segment count a tunnel draws at full stress.
    const STRESS_SEGMENTS: u8 = 126;

    /// How far a fixture is advanced off its initial state, so that no smoother
    /// and no integrated angle is sampled at the value it only takes at rest.
    const ADVANCE: Duration = Duration::from_micros(25_300);

    struct NoopEmitter;

    impl EmitPaletteStateChange for NoopEmitter {
        fn emit_palette_state_change(&mut self, _: PaletteStateChange) {}
    }

    /// A show frame under a name, so that a failure says which frame broke.
    pub struct NamedFrame {
        pub name: &'static str,
        pub frame: ShowFrame,
    }

    /// Every show frame a render is held to.
    pub fn all() -> Vec<NamedFrame> {
        vec![
            NamedFrame {
                name: "default beams",
                frame: default_frame(),
            },
            NamedFrame {
                name: "max variation",
                frame: max_variation_frame(),
            },
            NamedFrame {
                name: "nested looks",
                frame: nested_look_frame(),
            },
        ]
    }

    /// A frame of default beams at full level, on default show state.
    ///
    /// Every channel routes to video channel zero, which is where a mixer
    /// leaves it, so seven of the eight video channels draw nothing.
    pub fn default_frame() -> ShowFrame {
        let mut mixer = Mixer::new(1);
        for channel in mixer.channels() {
            channel.level = UnipolarFloat::ONE;
        }
        mixer.update_state(ADVANCE, UnipolarFloat::ZERO);
        ShowFrame {
            mixer,
            clocks: StaticClockBank::default(),
            palette: ColorPalette::default(),
            positions: PositionBank::default(),
            audio_envelope: UnipolarFloat::ZERO,
        }
    }

    /// A frame at the worst case a mixer can produce: eight tunnels at full
    /// segment count, each spending every animation slot on a distinct
    /// spatially-varying target and reading its hue, its centre and its
    /// animation timing out of the frame's own banks.
    ///
    /// Every video channel draws at least one tunnel, and two channels fan out
    /// to a second video channel apiece.
    pub fn max_variation_frame() -> ShowFrame {
        let mut mixer = Mixer::new(1);
        let n_channels = mixer.channel_count();
        for (i, channel) in mixer.channels().enumerate() {
            channel.level = UnipolarFloat::new(0.25 + 0.75 * (i as f64 / n_channels as f64));
            channel.bump = i == 3;
            channel.mask = i == 5;
            channel.video_outs.clear();
            channel.video_outs.insert(VideoChannel(i));
            if i % 4 == 0 {
                channel.video_outs.insert(VideoChannel((i + 1) % 8));
            }
            if let Beam::Tunnel(tunnel) = &mut channel.beam {
                stress_tunnel(tunnel, i, n_channels);
            }
        }
        mixer.update_state(ADVANCE, audio_envelope());

        ShowFrame {
            mixer,
            clocks: clocks(),
            palette: palette(),
            positions: positions(),
            audio_envelope: audio_envelope(),
        }
    }

    /// A frame whose channels hold looks that themselves hold looks.
    ///
    /// Looks nest without bound and carry their subchannels' routing and
    /// masking with them, which makes them the deepest structure the model can
    /// put on the wire.
    pub fn nested_look_frame() -> ShowFrame {
        let inner = stress_look(0);
        let mut middle = stress_mixer(1);
        *middle.beam(ChannelIdx(0)) = Beam::Look(inner);
        let middle = middle.as_look();

        let mut mixer = stress_mixer(2);
        *mixer.beam(ChannelIdx(2)) = Beam::Look(middle.clone());
        *mixer.beam(ChannelIdx(6)) = Beam::Look(middle);
        mixer.update_state(ADVANCE, audio_envelope());

        ShowFrame {
            mixer,
            clocks: clocks(),
            palette: palette(),
            positions: positions(),
            audio_envelope: audio_envelope(),
        }
    }

    /// The audio level the fixtures that read the envelope are scaled by.
    fn audio_envelope() -> UnipolarFloat {
        UnipolarFloat::new(0.7)
    }

    /// Configure one tunnel of a stressed channel, spread by its position in
    /// the mixer and bound to the frame's banks.
    fn stress_tunnel(tunnel: &mut Tunnel, index: usize, of: usize) {
        configure_max_variation(tunnel, index, of, STRESS_SEGMENTS);
        bind_to_frame_state(
            tunnel,
            ColorPaletteIdx(index % PALETTE_SIZE),
            PositionIdx(index % POSITION_COUNT),
            ClockIdx(index % MAX_CLOCKS),
        );
    }

    /// A mixer of stressed tunnels, spread by `generation` so that mixers at
    /// different depths of a look draw differently.
    fn stress_mixer(generation: usize) -> Mixer {
        let mut mixer = Mixer::new(1);
        let n_channels = mixer.channel_count();
        for (i, channel) in mixer.channels().enumerate() {
            channel.level = UnipolarFloat::ONE;
            channel.mask = i == generation;
            channel.video_outs.clear();
            channel.video_outs.insert(VideoChannel(i));
            if let Beam::Tunnel(tunnel) = &mut channel.beam {
                stress_tunnel(tunnel, i + generation, n_channels + generation);
            }
        }
        mixer
    }

    /// A look of stressed tunnels.
    fn stress_look(generation: usize) -> Look {
        stress_mixer(generation).as_look()
    }

    /// The number of colors in the fixture palette.
    const PALETTE_SIZE: usize = 5;

    /// The number of positions in the fixture position bank.
    const POSITION_COUNT: usize = 3;

    /// A palette of distinct hues, so that a tunnel selecting one of them
    /// draws differently from a tunnel selecting another.
    fn palette() -> ColorPalette {
        let colors = (0..PALETTE_SIZE)
            .map(|i| Hsv::from_hue(i as f64 / PALETTE_SIZE as f64))
            .collect();
        let mut palette = ColorPalette::default();
        palette.control(
            PaletteControlMessage::Set(PaletteStateChange::Contents(colors)),
            &mut NoopEmitter,
        );
        palette
    }

    /// A position bank of distinct offsets.
    fn positions() -> PositionBank {
        let mut bank = PositionBank::default();
        bank.control(
            (0..POSITION_COUNT)
                .map(|i| Position {
                    x: -0.5 + i as f64 / POSITION_COUNT as f64,
                    y: 0.25 - i as f64 / POSITION_COUNT as f64,
                })
                .collect(),
        );
        bank
    }

    /// A full bank of clocks, each at its own phase, tick count and submaster.
    fn clocks() -> StaticClockBank {
        let mut bank = ArrayVec::new();
        for i in 0..MAX_CLOCKS {
            let frac = i as f64 / MAX_CLOCKS as f64;
            let _ = bank.try_push(StaticClock {
                phase: Phase::new(frac),
                ticks: i as i64,
                submaster_level: UnipolarFloat::new(0.25 + 0.75 * frac),
                use_audio_size: i % 3 == 0,
            });
        }
        StaticClockBank(bank)
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::NamedFrame;
    use super::*;
    use crate::beam::Beam;
    use crate::layer::{LayerCollection, ShapeGeometry};
    use crate::look::{Look, MAX_NESTING_DEPTH};
    use crate::mixer::{Channel, ChannelIdx, Mixer, VideoChannel};
    use crate::tunnel::Tunnel;
    use std::collections::BTreeSet;
    use std::fmt;

    /// The wire bytes of a frame, written by an encoder the caller holds.
    fn encoded(encoder: &mut FrameEncoder, frame: &ShowFrame) -> Vec<u8> {
        encoder.encode(&borrow(frame)).unwrap().to_vec()
    }

    fn frame() -> ShowFrame {
        ShowFrame {
            mixer: Mixer::new(1),
            clocks: StaticClockBank::default(),
            palette: ColorPalette::default(),
            positions: PositionBank::default(),
            audio_envelope: UnipolarFloat::new(0.5),
        }
    }

    /// Every field of a shape, under the name a failure should report.
    fn shape_fields(shape: &ShapeGeometry) -> [(&'static str, f64); 12] {
        [
            ("level", shape.level),
            ("thickness", shape.thickness),
            ("hue", shape.hue),
            ("sat", shape.sat),
            ("val", shape.val),
            ("x", shape.x),
            ("y", shape.y),
            ("extent_x", shape.extent_x),
            ("extent_y", shape.extent_y),
            ("start", shape.start),
            ("rot_angle", shape.rot_angle),
            ("spin_angle", shape.spin_angle),
        ]
    }

    /// Panic unless two renders of a video channel agree bit for bit.
    ///
    /// The render is deterministic and the payload lossless, so every float is
    /// compared as its raw bits: a tolerance here would hide real drift.
    fn assert_identical(label: &str, expected: &LayerCollection, actual: &LayerCollection) {
        assert_eq!(expected.len(), actual.len(), "{label}: layer count");
        for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
            assert_eq!(
                e.render_mode, a.render_mode,
                "{label}: layer {i} render mode"
            );
            assert_eq!(e.path_shape, a.path_shape, "{label}: layer {i} path shape");
            assert_eq!(
                e.span.to_bits(),
                a.span.to_bits(),
                "{label}: layer {i} span"
            );
            assert_eq!(
                e.shapes.len(),
                a.shapes.len(),
                "{label}: layer {i} shape count"
            );
            for (j, (expected_shape, actual_shape)) in e.shapes.iter().zip(&a.shapes).enumerate() {
                for ((name, ev), (_, av)) in shape_fields(expected_shape)
                    .iter()
                    .zip(shape_fields(actual_shape))
                {
                    assert_eq!(
                        ev.to_bits(),
                        av.to_bits(),
                        "{label}: layer {i} shape {j} {name}: {ev} != {av}"
                    );
                }
            }
        }
    }

    /// Panic unless two values print identically, naming the first line on
    /// which they diverge.
    ///
    /// The printed form of a frame is far too large to read whole, and the one
    /// line that moved is the answer.
    fn assert_prints_identically<T: fmt::Debug>(label: &str, expected: &T, actual: &T) {
        let expected = format!("{expected:#?}");
        let actual = format!("{actual:#?}");
        if expected == actual {
            return;
        }
        for (line, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
            assert_eq!(e, a, "{label}: line {line}");
        }
        panic!(
            "{label}: prints {} lines, expected {}",
            actual.lines().count(),
            expected.lines().count()
        );
    }

    /// A frame is the same frame after the wire, and draws the same shapes.
    ///
    /// The two halves catch different failures. The render says that what the
    /// audience sees is unchanged; the printed model says that every field
    /// survived, including those no render reads today, which the render alone
    /// can never speak for.
    ///
    /// A frame is written by reference and read back owned, so the round trip
    /// also holds the two forms of a frame to a single shape.
    #[test]
    fn a_round_tripped_frame_is_unchanged() {
        let mut encoder = FrameEncoder::default();
        for NamedFrame { name, frame } in fixture::all() {
            let wire = encoded(&mut encoder, &frame);
            println!("{name}: {} bytes on the wire", wire.len());

            let decoded = ShowFrame::decode(&wire).unwrap();
            assert_prints_identically(name, &frame, &decoded);

            for channel in 0..Mixer::N_VIDEO_CHANNELS {
                let video_channel = VideoChannel(channel);
                let expected = frame
                    .mixer
                    .render_video_channel(video_channel, frame.render_context());
                let actual = decoded
                    .mixer
                    .render_video_channel(video_channel, decoded.render_context());
                assert_identical(
                    &format!("{name}, video channel {channel}"),
                    &expected,
                    &actual,
                );
            }
        }
    }

    /// The same frame, named by reference rather than owned.
    fn borrow(frame: &ShowFrame) -> ShowFrameRef<'_> {
        ShowFrameRef {
            mixer: &frame.mixer,
            clocks: frame.clocks.clone(),
            palette: &frame.palette,
            positions: &frame.positions,
            audio_envelope: frame.audio_envelope,
        }
    }

    /// The same model always encodes to the same bytes.
    ///
    /// Nothing in a frame may iterate in an order the process picked at
    /// random, or two runs put different bytes on the wire for the same show.
    #[test]
    fn an_encoding_depends_only_on_the_model() {
        let mut encoder = FrameEncoder::default();
        for (first, second) in fixture::all().iter().zip(fixture::all()) {
            let first_wire = encoded(&mut encoder, &first.frame);
            let second_wire = encoded(&mut encoder, &second.frame);
            assert!(
                first_wire == second_wire,
                "{}: two builds of one frame encoded differently, {} bytes against {}",
                first.name,
                first_wire.len(),
                second_wire.len()
            );
        }
    }

    #[test]
    fn round_trip_preserves_the_frame() {
        let mut encoder = FrameEncoder::default();
        let decoded = ShowFrame::decode(&encoded(&mut encoder, &frame())).unwrap();
        assert_eq!(decoded.audio_envelope, UnipolarFloat::new(0.5));
        assert_eq!(decoded.mixer.channel_count(), 8);
    }

    /// A beam wrapping a tunnel in `depth` levels of nested look.
    ///
    /// Built by growing outwards rather than by recursing, so that producing
    /// the tree costs no more stack than producing one level of it.
    fn nested_beam(depth: usize) -> Beam {
        let mut beam = Beam::Tunnel(Tunnel::default());
        for _ in 0..depth {
            beam = Beam::Look(Look::from_channels(vec![Channel {
                beam,
                level: UnipolarFloat::ONE,
                bump: false,
                mask: false,
                video_outs: BTreeSet::from([VideoChannel(0)]),
            }]));
        }
        beam
    }

    /// A look nests without bound, so bytes claiming unbounded nesting have to
    /// be refused rather than followed.
    #[test]
    fn nesting_past_the_limit_is_rejected_without_overflowing() {
        let mut encoder = FrameEncoder::default();
        let mut deepest_allowed = frame();
        *deepest_allowed.mixer.beam(ChannelIdx(0)) = nested_beam(MAX_NESTING_DEPTH);
        assert!(
            ShowFrame::decode(&encoded(&mut encoder, &deepest_allowed)).is_ok(),
            "a look nested to the limit is still a look"
        );

        // One level past the limit, and no deeper: the tree is built and
        // dropped here too, and dropping it is as recursive as decoding it.
        let mut too_deep = frame();
        *too_deep.mixer.beam(ChannelIdx(0)) = nested_beam(MAX_NESTING_DEPTH + 1);
        let err = ShowFrame::decode(&encoded(&mut encoder, &too_deep))
            .expect_err("nesting past the limit must not decode");
        assert!(
            matches!(err, FrameCodecError::Deserialize(_)),
            "expected a deserialization failure, got {err}"
        );
    }

    /// Wire bytes carrying a body that is not the one `encode` produced.
    fn wire_with_body(body: &[u8]) -> Vec<u8> {
        let mut wire = FRAME_MAGIC.to_vec();
        wire.push(WIRE_VERSION);
        wire.extend_from_slice(body);
        wire
    }

    #[test]
    fn malformed_bytes_are_rejected_without_panicking() {
        let mut encoder = FrameEncoder::default();
        let wire = encoded(&mut encoder, &frame());

        // Too short to hold even the header.
        assert!(matches!(
            ShowFrame::decode(&wire[..2]),
            Err(FrameCodecError::NotAFrame)
        ));

        // Header, and nothing to hold the length prefix.
        assert!(matches!(
            ShowFrame::decode(&wire[..FRAME_HEADER_LEN]),
            Err(FrameCodecError::Decompress(_))
        ));

        // A truncated body carries less than the frame it was written from,
        // and where the cut lands decides which end notices: the block may
        // fail to expand at all, or expand short and leave the frame
        // unfinished.
        assert!(
            matches!(
                ShowFrame::decode(&wire[..wire.len() / 2]),
                Err(FrameCodecError::Decompress(_) | FrameCodecError::Deserialize(_))
            ),
            "half of a frame decoded as a whole one"
        );

        // A corrupted length prefix is refused rather than allocated.
        let mut oversized = wire.clone();
        oversized[FRAME_HEADER_LEN..FRAME_HEADER_LEN + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            ShowFrame::decode(&oversized),
            Err(FrameCodecError::Oversized { .. })
        ));

        // Bytes that decompress cleanly but describe something else.
        let junk = wire_with_body(&lz4_flex::compress_prepend_size(&[0xff; 64]));
        assert!(matches!(
            ShowFrame::decode(&junk),
            Err(FrameCodecError::Deserialize(_))
        ));
    }

    /// Bytes from a build holding a different model are refused, not rendered.
    ///
    /// A tagless encoding makes a stale peer the dangerous case: its frame
    /// would otherwise decode into a structurally valid show that is not the
    /// one being played.
    #[test]
    fn a_frame_from_another_build_does_not_decode() {
        let mut encoder = FrameEncoder::default();
        let wire = encoded(&mut encoder, &frame());

        let stale_version = WIRE_VERSION.wrapping_add(1);
        let mut stale = wire.clone();
        stale[FRAME_MAGIC.len()] = stale_version;
        match ShowFrame::decode(&stale).expect_err("a stale frame must not decode") {
            FrameCodecError::WireVersion { found, expected } => {
                assert_eq!(found, stale_version);
                assert_eq!(expected, WIRE_VERSION);
            }
            other => panic!("a frame at another wire version failed as {other}"),
        }

        let mut foreign = wire.clone();
        foreign[0] = !foreign[0];
        match ShowFrame::decode(&foreign).expect_err("foreign bytes must not decode") {
            FrameCodecError::NotAFrame => {}
            other => panic!("bytes that are not a frame failed as {other}"),
        }
    }

    /// A reused decoder recovers the same frame a fresh one does.
    ///
    /// A frame expands into the buffer the frame before it expanded into, so a
    /// frame shorter than its predecessor leaves some of those bytes behind.
    /// The decoder has to be held to what it decompressed rather than to what
    /// its buffer holds.
    #[test]
    fn a_reused_decoder_recovers_the_same_frame() {
        let frames = fixture::all();
        let mut encoder = FrameEncoder::default();
        let wire: Vec<Vec<u8>> = frames
            .iter()
            .map(|named| encoded(&mut encoder, &named.frame))
            .collect();
        let mut decoder = FrameDecoder::default();
        let in_order = frames.iter().zip(&wire);
        // Backwards as well as forwards, so that a frame is decompressed into
        // a buffer both smaller and larger than the one it needs.
        for (NamedFrame { name, frame }, wire) in in_order.clone().chain(in_order.rev()) {
            let decoded = decoder.decode(wire).unwrap();
            assert_prints_identically(name, frame, &decoded);
        }
    }

    /// The bytes an encoded frame is defined to be, stated without reference
    /// to the code that produces them: the magic, the wire version, the
    /// uncompressed length little-endian, then an LZ4 block.
    fn wire_format(frame: &ShowFrameRef) -> Vec<u8> {
        let plain = postcard::to_allocvec(frame).unwrap();
        let mut wire = Vec::new();
        wire.extend_from_slice(b"TNL");
        wire.push(1);
        wire.extend_from_slice(&lz4_flex::compress_prepend_size(&plain));
        wire
    }

    /// A reused encoder writes the bytes the wire format defines, frame after
    /// frame.
    ///
    /// The bytes are a contract between applications rather than an internal
    /// detail: every render client reads them, and reads them from a binary
    /// built at another time. They are held against an independent statement
    /// of the format, so that a change to how they are produced fails here
    /// instead of quietly redefining what they are.
    #[test]
    fn an_encoded_frame_is_byte_for_byte_the_wire_format() {
        let frames = fixture::all();
        let mut encoder = FrameEncoder::default();
        // Twice over, so that an encoder writing into buffers it has already
        // filled is held to the same bytes as one writing into empty ones.
        for pass in 1..=2 {
            for NamedFrame { name, frame } in &frames {
                let expected = wire_format(&borrow(frame));
                let actual = encoder.encode(&borrow(frame)).unwrap();
                assert!(
                    actual == expected,
                    "{name}, pass {pass}: encoded {} bytes against the {} the wire format defines",
                    actual.len(),
                    expected.len()
                );
            }
        }
    }
}
