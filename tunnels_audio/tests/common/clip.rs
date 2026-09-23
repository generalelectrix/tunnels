//! Test audio: interleaved 16-bit PCM as a serde type, stored with postcard.

use serde::{Deserialize, Serialize};

/// Interleaved 16-bit PCM.
#[derive(Debug, Serialize, Deserialize)]
pub struct Clip {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples, `frames * channels` long.
    pub samples: Vec<i16>,
}

impl Clip {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// The clip as stereo frames in [-1, 1]. Panics unless it has two channels.
    pub fn stereo_frames(&self) -> Vec<[f32; 2]> {
        assert_eq!(self.channels, 2, "clip is not stereo");
        self.samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[l, r]| [l as f32 / 32768.0, r as f32 / 32768.0])
            .collect()
    }
}

pub fn encode(clip: &Clip) -> Vec<u8> {
    postcard::to_allocvec(clip).expect("serialize clip")
}

pub fn decode(bytes: &[u8]) -> Result<Clip, postcard::Error> {
    postcard::from_bytes(bytes)
}
