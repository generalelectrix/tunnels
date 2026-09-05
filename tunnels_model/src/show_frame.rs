//! The complete state a render consumes for one frame.

use std::error::Error;
use std::fmt;

use lz4_flex::block::{DecompressError, uncompressed_size};
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
    pub frame_number: u64,
    pub mixer: Mixer,
    pub clocks: StaticClockBank,
    pub palette: ColorPalette,
    pub positions: PositionBank,
    pub audio_envelope: UnipolarFloat,
}

/// The largest payload a frame is allowed to expand into.
///
/// A frame is a few kilobytes. The ceiling is here so that a corrupted length
/// prefix asks for a rejected decode rather than a multi-gigabyte allocation.
const MAX_DECODED_LEN: usize = 8 * 1024 * 1024;

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

    /// Serialize this frame into wire bytes.
    pub fn encode(&self) -> Result<Vec<u8>, FrameCodecError> {
        let plain = postcard::to_allocvec(self).map_err(FrameCodecError::Serialize)?;
        Ok(lz4_flex::compress_prepend_size(&plain))
    }

    /// Recover a frame from the wire bytes `encode` produces.
    ///
    /// Every way the bytes can be wrong is an error, never a panic: a mangled
    /// or truncated payload costs the frame it arrived in and nothing more.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameCodecError> {
        let (decoded_len, compressed) =
            uncompressed_size(bytes).map_err(FrameCodecError::Decompress)?;
        if decoded_len > MAX_DECODED_LEN {
            return Err(FrameCodecError::Oversized {
                declared: decoded_len,
                limit: MAX_DECODED_LEN,
            });
        }
        let plain =
            lz4_flex::decompress(compressed, decoded_len).map_err(FrameCodecError::Decompress)?;
        postcard::from_bytes(&plain).map_err(FrameCodecError::Deserialize)
    }
}

/// Why a show frame could not be put on the wire, or recovered from it.
#[derive(Debug)]
pub enum FrameCodecError {
    /// The frame could not be serialized.
    Serialize(postcard::Error),
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
            Self::Oversized { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::Mixer;

    fn frame() -> ShowFrame {
        ShowFrame {
            frame_number: 7,
            mixer: Mixer::new(1),
            clocks: StaticClockBank::default(),
            palette: ColorPalette::default(),
            positions: PositionBank::default(),
            audio_envelope: UnipolarFloat::new(0.5),
        }
    }

    #[test]
    fn round_trip_preserves_the_frame() {
        let decoded = ShowFrame::decode(&frame().encode().unwrap()).unwrap();
        assert_eq!(decoded.frame_number, 7);
        assert_eq!(decoded.audio_envelope, UnipolarFloat::new(0.5));
        assert_eq!(decoded.mixer.channel_count(), 8);
    }

    #[test]
    fn malformed_bytes_are_rejected_without_panicking() {
        let wire = frame().encode().unwrap();

        // Too short to hold even the length prefix.
        assert!(matches!(
            ShowFrame::decode(&wire[..2]),
            Err(FrameCodecError::Decompress(_))
        ));

        // A truncated body cannot expand to the length the prefix promises.
        assert!(matches!(
            ShowFrame::decode(&wire[..wire.len() / 2]),
            Err(FrameCodecError::Decompress(_))
        ));

        // A corrupted length prefix is refused rather than allocated.
        let mut oversized = wire.clone();
        oversized[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            ShowFrame::decode(&oversized),
            Err(FrameCodecError::Oversized { .. })
        ));

        // Bytes that decompress cleanly but describe something else.
        let junk = lz4_flex::compress_prepend_size(&[0xff; 64]);
        assert!(matches!(
            ShowFrame::decode(&junk),
            Err(FrameCodecError::Deserialize(_))
        ));
    }
}
