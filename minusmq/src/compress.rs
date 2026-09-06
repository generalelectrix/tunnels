//! Payload compression: [u32 little-endian decompressed length][LZ4 block].
//!
//! A compressed stream carries nothing to say that it is one. Which streams
//! compress is configuration held at both ends rather than a flag on the wire,
//! so a peer of a stream that does not compress sees the same bytes it always
//! did.

use anyhow::{Context, Result, ensure};
use lz4_flex::block::{
    compress_into as compress_block, decompress_into as decompress_block, get_maximum_output_size,
    uncompressed_size,
};

/// Compress `data` into `out`, replacing what it holds.
///
/// The buffer is refilled rather than regrown for a payload no larger than the
/// largest one compressed into it before, so a steady stream of messages costs
/// no allocation of its own.
pub fn compress_into(out: &mut Vec<u8>, data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).context("payload too large to compress (>4GB)")?;
    out.clear();
    out.extend_from_slice(&len.to_le_bytes());

    // Compression writes into the buffer directly, so it needs room for the
    // worst case the block format can produce before it starts.
    let block = out.len();
    out.resize(block + get_maximum_output_size(data.len()), 0);
    let compressed = compress_block(data, &mut out[block..]).context("could not compress")?;
    out.truncate(block + compressed);
    Ok(())
}

/// Expand `data` into `out`, replacing what it holds.
///
/// A payload declaring an expansion beyond `max_len` fails before anything is
/// sized to it: the declared length arrives from whoever is on the other end
/// of the connection, and a length alone is not a reason to ask the allocator
/// for one.
///
/// The buffer is refilled rather than regrown for a payload no larger than the
/// largest one expanded into it before. What it holds after a failure is
/// whatever the failure left there, and is not a payload.
pub fn decompress_into(out: &mut Vec<u8>, data: &[u8], max_len: usize) -> Result<()> {
    let (decompressed_len, block) =
        uncompressed_size(data).context("could not read a decompressed length")?;
    ensure!(
        decompressed_len <= max_len,
        "payload claiming to decompress to {decompressed_len} bytes exceeds the limit of {max_len} bytes"
    );

    // Only past the limit does the declared length, which came off the wire,
    // get to decide how much memory to hold.
    out.resize(decompressed_len, 0);
    let len = decompress_block(block, &mut out[..]).context("could not decompress")?;
    out.truncate(len);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload with enough structure to compress, of a given size.
    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 7) as u8).collect()
    }

    /// A limit no payload in a test reaches, for the tests that are about
    /// something other than the limit.
    const UNBOUNDED: usize = usize::MAX;

    /// A payload survives compression, whatever the buffers held before it.
    ///
    /// Both buffers are refilled rather than reallocated, so a payload shorter
    /// than the one before it leaves some of that one's bytes behind: each end
    /// has to be held to what it wrote rather than to what its buffer holds.
    #[test]
    fn a_payload_survives_whatever_the_buffers_held_before() {
        let mut compressed = Vec::new();
        let mut expanded = Vec::new();
        // Long, then short, then long again, so that each pass writes into a
        // buffer both larger and smaller than the one it needs.
        for len in [64_000, 40, 64_000, 0, 3_000] {
            let data = payload(len);
            compress_into(&mut compressed, &data).unwrap();
            decompress_into(&mut expanded, &compressed, UNBOUNDED).unwrap();
            assert_eq!(expanded, data, "a {len}-byte payload did not survive");
        }
    }

    /// The bytes compression produces, stated without reference to the code
    /// that produces them: the decompressed length little-endian, then an LZ4
    /// block.
    ///
    /// The bytes are a contract between applications rather than an internal
    /// detail: both ends of a stream read them, from binaries built at
    /// different times.
    #[test]
    fn a_compressed_payload_is_byte_for_byte_an_lz4_block() {
        let mut compressed = Vec::new();
        for len in [64_000, 40, 0] {
            let data = payload(len);
            compress_into(&mut compressed, &data).unwrap();
            assert_eq!(compressed, lz4_flex::compress_prepend_size(&data));
        }
    }

    /// A payload claiming to expand past the limit is refused, and refused
    /// before the buffer is sized to it.
    #[test]
    fn an_expansion_past_the_limit_is_refused_without_allocating() {
        const LIMIT: usize = 1024;

        let mut compressed = Vec::new();
        let mut expanded = Vec::new();
        compress_into(&mut compressed, &payload(LIMIT)).unwrap();
        decompress_into(&mut expanded, &compressed, LIMIT).unwrap();

        compress_into(&mut compressed, &payload(LIMIT + 1)).unwrap();
        let err = decompress_into(&mut expanded, &compressed, LIMIT).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "payload claiming to decompress to {} bytes exceeds the limit of {LIMIT} bytes",
                LIMIT + 1
            )
        );

        // A length prefix claiming the largest expansion the format can
        // describe, with no block behind it at all.
        let mut oversized = u32::MAX.to_le_bytes().to_vec();
        let mut untouched = Vec::new();
        assert!(decompress_into(&mut untouched, &oversized, LIMIT).is_err());
        assert!(
            untouched.capacity() <= LIMIT,
            "a refused payload still grew the buffer to {} bytes",
            untouched.capacity()
        );

        // The same prefix in front of a real block, so that only the limit
        // stands between the claim and the allocation.
        compress_into(&mut compressed, &payload(64)).unwrap();
        oversized.extend_from_slice(&compressed[4..]);
        assert!(decompress_into(&mut untouched, &oversized, LIMIT).is_err());
    }

    /// Bytes that are not a compressed payload are refused rather than
    /// expanded.
    #[test]
    fn bytes_that_are_not_a_payload_are_refused() {
        let mut expanded = Vec::new();
        // Too short to carry even the length prefix.
        assert!(decompress_into(&mut expanded, &[0xAB, 0xCD], UNBOUNDED).is_err());

        // A prefix, and a block that does not describe what it claims.
        let mut compressed = Vec::new();
        compress_into(&mut compressed, &payload(4_000)).unwrap();
        let truncated = &compressed[..compressed.len() / 2];
        assert!(decompress_into(&mut expanded, truncated, UNBOUNDED).is_err());
    }
}
