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
}

pub fn encode(clip: &Clip) -> Vec<u8> {
    postcard::to_allocvec(clip).expect("serialize clip")
}

pub fn decode(bytes: &[u8]) -> Result<Clip, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejects_truncation() {
        let clip = Clip {
            sample_rate: 48000,
            channels: 2,
            samples: (0..2000)
                .map(|i| ((i as f32 / 100.0).sin() * 30000.0) as i16)
                .chain([i16::MAX, i16::MIN])
                .collect(),
        };
        let bytes = encode(&clip);
        let back = decode(&bytes).expect("decodes");
        assert_eq!(back.sample_rate, 48000);
        assert_eq!(back.channels, 2);
        assert_eq!(back.samples, clip.samples);
        assert_eq!(
            decode(&bytes[..bytes.len() - 1]).unwrap_err(),
            postcard::Error::DeserializeUnexpectedEnd
        );
    }
}
