//! The complete state a render consumes for one frame.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use tunnels_lib::number::UnipolarFloat;

use crate::clock_bank::StaticClockBank;
use crate::mixer::Mixer;
use crate::palette::ColorPalette;
use crate::position_bank::PositionBank;
use crate::render_context::RenderContext;

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

/// The scratch a show frame is serialized in.
///
/// The buffer is held rather than produced, so encoding a frame no larger than
/// the largest one encoded before it writes into memory that is already there,
/// rather than asking the allocator for more.
#[derive(Debug, Default)]
pub struct FrameEncoder {
    /// The serialized frame.
    wire: Vec<u8>,
}

impl FrameEncoder {
    /// Serialize a frame into the bytes `ShowFrame::decode` reads, which stand
    /// until the next frame is encoded.
    ///
    /// The bytes are tagless: the schema is the Rust type, and it does not
    /// travel with them.
    pub fn encode(&mut self, frame: &ShowFrameRef) -> Result<&[u8], FrameCodecError> {
        self.wire.clear();
        postcard::to_io(frame, &mut self.wire).map_err(FrameCodecError::Serialize)?;
        Ok(&self.wire)
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
    ///
    /// Every way the bytes can be wrong is an error, never a panic: a mangled
    /// or truncated payload costs the frame it arrived in and nothing more.
    /// That includes bytes asserting more nesting than a look is allowed,
    /// which are refused before the nesting is followed rather than after.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameCodecError> {
        postcard::from_bytes(bytes).map_err(FrameCodecError::Deserialize)
    }
}

/// Why a show frame could not be put on the wire, or recovered from it.
#[derive(Debug)]
pub enum FrameCodecError {
    /// The frame could not be serialized.
    Serialize(postcard::Error),
    /// The bytes describe something other than a frame.
    Deserialize(postcard::Error),
}

impl fmt::Display for FrameCodecError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Serialize(e) => write!(f, "could not serialize a show frame: {e}"),
            Self::Deserialize(e) => write!(f, "could not deserialize a show frame: {e}"),
        }
    }
}

impl Error for FrameCodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialize(e) | Self::Deserialize(e) => Some(e),
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
    use crate::layer::FigureLibrary;
    use crate::look::Look;
    use crate::mixer::{ChannelIdx, VideoChannel};
    use crate::palette::{
        ColorPaletteIdx, ControlMessage as PaletteControlMessage,
        EmitStateChange as EmitPaletteStateChange, StateChange as PaletteStateChange,
    };
    use crate::position_bank::{Position, PositionIdx};
    use crate::tunnel::Tunnel;
    use crate::tunnel::fixture::{bind_to_frame_state, configure_figure, configure_max_variation};
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
            NamedFrame {
                name: "figures",
                frame: figure_frame(),
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

    /// A frame of filled figures, which is the other kind of layer a beam
    /// expands into, drawn from both libraries.
    ///
    /// A figure carries no per-shape geometry: what travels is which figure to
    /// draw, where to put it, the colour model to resolve it against, and the
    /// animations left unresolved because they vary across it. None of that is
    /// on the path a run of segments takes, so a suite of segment frames speaks
    /// for none of it.
    ///
    /// The two libraries name a figure differently — an index into a table the
    /// build fixes, or a family and the two numbers that place a figure inside
    /// it — so channels alternate between them. A frame carrying one naming
    /// leaves the other unexercised, and a name that arrived as another name
    /// would draw another figure.
    pub fn figure_frame() -> ShowFrame {
        let mut mixer = Mixer::new(1);
        let n_channels = mixer.channel_count();
        for (i, channel) in mixer.channels().enumerate() {
            channel.level = UnipolarFloat::new(0.25 + 0.75 * (i as f64 / n_channels as f64));
            channel.mask = i == 2;
            channel.video_outs.clear();
            channel.video_outs.insert(VideoChannel(i));
            if let Beam::Tunnel(tunnel) = &mut channel.beam {
                let library = if i.is_multiple_of(2) {
                    FigureLibrary::Baked
                } else {
                    FigureLibrary::Generated
                };
                configure_figure(tunnel, library, i, n_channels);
                bind_to_frame_state(
                    tunnel,
                    ColorPaletteIdx(i % PALETTE_SIZE),
                    PositionIdx(i % POSITION_COUNT),
                    ClockIdx(i % MAX_CLOCKS),
                );
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
    use crate::layer::{
        ColorField, FillLayer, Hsva, Layer, LayerCollection, Placement, ShapeGeometry,
    };
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

    /// Every float of a placement, under the name a failure should report.
    ///
    /// Destructured rather than read field by field, here and in the three
    /// below it, so that a field added to one of these types fails to compile
    /// instead of going quietly uncompared.
    fn placement_fields(p: Placement) -> [(&'static str, f64); 5] {
        let Placement {
            x,
            y,
            extent_x,
            extent_y,
            rot_angle,
        } = p;
        [
            ("x", x),
            ("y", y),
            ("extent_x", extent_x),
            ("extent_y", extent_y),
            ("rot_angle", rot_angle),
        ]
    }

    /// Every float of a resolved colour, under the name a failure should
    /// report.
    fn color_fields(c: Hsva) -> [(&'static str, f64); 4] {
        let Hsva {
            hue,
            sat,
            val,
            level,
        } = c;
        [("hue", hue), ("sat", sat), ("val", val), ("level", level)]
    }

    /// Every float of a shape, under the name a failure should report.
    fn shape_fields(shape: &ShapeGeometry) -> impl Iterator<Item = (&'static str, f64)> {
        let ShapeGeometry {
            color,
            placement,
            thickness,
            start,
            spin_angle,
        } = *shape;
        color_fields(color)
            .into_iter()
            .chain(placement_fields(placement))
            .chain([
                ("thickness", thickness),
                ("start", start),
                ("spin_angle", spin_angle),
            ])
    }

    /// Every float of a figure, under the name a failure should report.
    fn fill_fields(fill: &FillLayer) -> impl Iterator<Item = (&'static str, f64)> {
        let ColorField {
            phase: _,
            cycles,
            center,
            width,
            sat,
            val,
            level,
        } = fill.color;
        placement_fields(fill.placement)
            .into_iter()
            .chain([
                ("spin_speed", fill.spin_speed),
                ("thickness", fill.thickness),
            ])
            .chain([
                ("cycles", cycles),
                ("center", center),
                ("width", width),
                ("sat", sat),
                ("val", val),
                ("level", level),
            ])
    }

    /// How many layers of each kind a comparison walked.
    ///
    /// A beam expands into one kind or the other, and the two are compared by
    /// different code, so a suite of frames that reaches only one kind leaves
    /// the other's comparison standing unrun.
    #[derive(Default)]
    struct LayersCompared {
        segments: usize,
        fills: usize,
    }

    impl LayersCompared {
        fn add(&mut self, other: Self) {
            self.segments += other.segments;
            self.fills += other.fills;
        }
    }

    /// Panic unless two renders of a video channel agree bit for bit.
    ///
    /// The render is deterministic and the payload lossless, so every float is
    /// compared as its raw bits: a tolerance here would hide real drift.
    fn assert_identical(
        label: &str,
        expected: &LayerCollection,
        actual: &LayerCollection,
    ) -> LayersCompared {
        let mut compared = LayersCompared::default();
        assert_eq!(expected.len(), actual.len(), "{label}: layer count");
        for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
            match (e.as_ref(), a.as_ref()) {
                (Layer::Segments(e), Layer::Segments(a)) => {
                    compared.segments += 1;
                    assert_eq!(
                        e.render_mode, a.render_mode,
                        "{label}: layer {i} render mode"
                    );
                    assert_eq!(
                        e.segment_path, a.segment_path,
                        "{label}: layer {i} segment path"
                    );
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
                    for (j, (expected_shape, actual_shape)) in
                        e.shapes.iter().zip(&a.shapes).enumerate()
                    {
                        for ((name, ev), (_, av)) in
                            shape_fields(expected_shape).zip(shape_fields(actual_shape))
                        {
                            assert_eq!(
                                ev.to_bits(),
                                av.to_bits(),
                                "{label}: layer {i} shape {j} {name}: {ev} != {av}"
                            );
                        }
                    }
                }
                (Layer::Fill(e), Layer::Fill(a)) => {
                    compared.fills += 1;
                    assert_eq!(e.figure, a.figure, "{label}: layer {i} figure");
                    assert_eq!(e.draw_mode, a.draw_mode, "{label}: layer {i} draw mode");
                    assert_eq!(
                        e.color.phase, a.color.phase,
                        "{label}: layer {i} colour phase"
                    );
                    assert_eq!(
                        e.color_anims.len(),
                        a.color_anims.len(),
                        "{label}: layer {i} colour animation count"
                    );
                    assert_eq!(
                        e.warps.len(),
                        a.warps.len(),
                        "{label}: layer {i} warp count"
                    );
                    for ((name, ev), (_, av)) in fill_fields(e).zip(fill_fields(a)) {
                        assert_eq!(
                            ev.to_bits(),
                            av.to_bits(),
                            "{label}: layer {i} {name}: {ev} != {av}"
                        );
                    }
                }
                _ => panic!("{label}: layer {i} is a different kind of layer"),
            }
        }
        compared
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
        let mut compared = LayersCompared::default();
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
                compared.add(assert_identical(
                    &format!("{name}, video channel {channel}"),
                    &expected,
                    &actual,
                ));
            }
        }
        // A kind of layer no fixture produces is a kind of layer this test
        // says nothing about, however many frames it walks.
        assert!(compared.segments > 0, "no fixture drew a run of segments");
        assert!(compared.fills > 0, "no fixture drew a filled figure");
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

    #[test]
    fn malformed_bytes_are_rejected_without_panicking() {
        let mut encoder = FrameEncoder::default();
        let wire = encoded(&mut encoder, &frame());

        // Too short to describe a frame at all.
        assert!(matches!(
            ShowFrame::decode(&wire[..2]),
            Err(FrameCodecError::Deserialize(_))
        ));

        // A truncated frame carries less than the one it was written from, and
        // leaves the model it describes unfinished.
        assert!(
            matches!(
                ShowFrame::decode(&wire[..wire.len() / 2]),
                Err(FrameCodecError::Deserialize(_))
            ),
            "half of a frame decoded as a whole one"
        );

        // Bytes that are well-formed enough to read, and describe something
        // else.
        assert!(matches!(
            ShowFrame::decode(&[0xff; 64]),
            Err(FrameCodecError::Deserialize(_))
        ));
    }

    /// The frame in postcard and nothing else: the bytes an encoder has to
    /// produce, written the plainest way there is to write them.
    fn wire_format(frame: &ShowFrameRef) -> Vec<u8> {
        postcard::to_allocvec(frame).unwrap()
    }

    /// A reused encoder writes what a fresh one writes, frame after frame.
    ///
    /// This holds the encoder and not the schema. Both sides reach the same
    /// serde impl, so a field added to the model, or moved within it, moves
    /// both sides together and passes here — the name notwithstanding, no byte
    /// layout is pinned. What cannot move is the encoder: writing into a buffer
    /// holding a previous frame has to produce what writing into an empty one
    /// does.
    ///
    /// Nothing pins the layout across a change to the model, and nothing needs
    /// to: a client and the console it renders for are the same build, so the
    /// two ends cannot hold different ideas of the schema.
    #[test]
    fn an_encoded_frame_is_byte_for_byte_the_wire_format() {
        let frames = fixture::all();
        let mut encoder = FrameEncoder::default();
        // Twice over, so that an encoder writing into a buffer it has already
        // filled is held to the same bytes as one writing into an empty one.
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
