//! A tiny self-describing PCM container for test audio.
//!
//! Layout, all integers little-endian:
//!
//! ```text
//! "TCLP"  u8 version(=1)  u32 sample_rate  u16 channels  u32 frames
//! then frames × channels samples, each the first-order delta from the
//! previous sample of the same channel, zigzag-mapped and LEB128-encoded
//! ```
//!
//! Music deltas are small, so most samples take one or two bytes. The point
//! is a format with no dependencies that a test can read in a few lines, not
//! the compression ratio.

const MAGIC: &[u8; 4] = b"TCLP";
const VERSION: u8 = 1;

/// Decoded interleaved 16-bit PCM.
#[derive(Debug)]
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

fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

fn unzigzag(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(bytes: &[u8], pos: &mut usize) -> Result<u32, String> {
    let mut v: u32 = 0;
    for shift in (0..35).step_by(7) {
        let b = *bytes.get(*pos).ok_or("truncated varint")?;
        *pos += 1;
        v |= u32::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err("varint too long".into())
}

pub fn encode(clip: &Clip) -> Vec<u8> {
    let channels = clip.channels as usize;
    let mut out = Vec::with_capacity(15 + clip.samples.len() * 2);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&clip.sample_rate.to_le_bytes());
    out.extend_from_slice(&clip.channels.to_le_bytes());
    out.extend_from_slice(&(clip.frames() as u32).to_le_bytes());
    let mut prev = vec![0i32; channels];
    for frame in clip.samples.chunks_exact(channels) {
        for (ch, &s) in frame.iter().enumerate() {
            let delta = i32::from(s) - prev[ch];
            prev[ch] = i32::from(s);
            put_varint(&mut out, zigzag(delta));
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<Clip, String> {
    if bytes.len() < 15 || &bytes[..4] != MAGIC {
        return Err("not a TCLP clip".into());
    }
    if bytes[4] != VERSION {
        return Err(format!("unsupported clip version {}", bytes[4]));
    }
    let sample_rate = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
    let channels = u16::from_le_bytes([bytes[9], bytes[10]]);
    let frames = u32::from_le_bytes([bytes[11], bytes[12], bytes[13], bytes[14]]) as usize;
    if channels == 0 {
        return Err("clip has zero channels".into());
    }
    let count = frames
        .checked_mul(channels as usize)
        .ok_or("frame count overflow")?;
    let mut samples = Vec::with_capacity(count);
    let mut prev = vec![0i32; channels as usize];
    let mut pos = 15;
    for i in 0..count {
        let ch = i % channels as usize;
        let delta = unzigzag(get_varint(bytes, &mut pos)?);
        let s = prev[ch] + delta;
        let s = i16::try_from(s).map_err(|_| format!("sample {i} out of range"))?;
        prev[ch] = i32::from(s);
        samples.push(s);
    }
    if pos != bytes.len() {
        return Err(format!("{} trailing bytes", bytes.len() - pos));
    }
    Ok(Clip {
        sample_rate,
        channels,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejects_corruption() {
        let samples: Vec<i16> = (0..2000)
            .map(|i| {
                let t = i as f32 / 100.0;
                if i % 2 == 0 {
                    (t.sin() * 30000.0) as i16
                } else {
                    ((t * 3.0).cos() * 12000.0) as i16 + (i % 7) as i16
                }
            })
            .chain([i16::MAX, i16::MIN, i16::MIN, i16::MAX])
            .collect();
        let clip = Clip {
            sample_rate: 48000,
            channels: 2,
            samples,
        };
        let bytes = encode(&clip);
        let back = decode(&bytes).expect("decodes");
        assert_eq!(back.sample_rate, 48000);
        assert_eq!(back.channels, 2);
        assert_eq!(back.samples, clip.samples);

        assert_eq!(
            decode(&bytes[..bytes.len() - 1]).unwrap_err(),
            "truncated varint"
        );
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(decode(&extra).unwrap_err(), "1 trailing bytes");
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert_eq!(decode(&bad_magic).unwrap_err(), "not a TCLP clip");
    }
}
