//! The test-clip container round-trips and rejects a truncated file.

mod common;

use common::clip::{Clip, decode, encode};

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
