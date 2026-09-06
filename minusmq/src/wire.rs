//! Length-prefixed wire format: [u32 big-endian length][payload bytes].

use anyhow::{Context, Result};
use std::io::{Read, Write};

/// Frame a message: its length prefix and its payload, contiguous.
///
/// Fails for a payload too long to describe in the prefix.
pub fn frame_msg(data: &[u8]) -> Result<Vec<u8>> {
    let len = u32::try_from(data.len()).context("message too large (>4GB)")?;
    let prefix = len.to_be_bytes();
    let mut framed = Vec::with_capacity(prefix.len() + data.len());
    framed.extend_from_slice(&prefix);
    framed.extend_from_slice(data);
    Ok(framed)
}

/// Write a length-prefixed message to the writer, in a single write.
///
/// A prefix written separately from its payload can leave a socket with
/// `TCP_NODELAY` set as a packet of its own, so the two go together.
pub fn write_msg(writer: &mut impl Write, data: &[u8]) -> Result<()> {
    writer.write_all(&frame_msg(data)?)?;
    Ok(())
}

/// Read a length-prefixed message from the reader.
pub fn read_msg(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A writer that keeps every write it is handed, whole and in order.
    #[derive(Default)]
    struct RecordingWriter {
        writes: Vec<Vec<u8>>,
    }

    impl RecordingWriter {
        /// Everything written, as it would appear on the wire.
        fn written(&self) -> Vec<u8> {
            self.writes.concat()
        }
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.push(buf.to_vec());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The length prefix and the payload written separately, as an
    /// independent statement of the wire format to compare against.
    fn write_prefix_then_payload(writer: &mut impl Write, data: &[u8]) {
        let len = u32::try_from(data.len()).unwrap();
        writer.write_all(&len.to_be_bytes()).unwrap();
        writer.write_all(data).unwrap();
    }

    /// A message reaches the writer as one write, byte-identical to the
    /// prefix and payload written separately.
    ///
    /// A length prefix written on its own can leave a socket with
    /// `TCP_NODELAY` set as a packet of its own, so the number of writes is
    /// part of the behaviour; the bytes are a cross-application contract and
    /// are not.
    #[test]
    fn one_write_per_message() {
        for data in [b"".as_slice(), b"hello world".as_slice(), &[7u8; 5000]] {
            let mut writer = RecordingWriter::default();
            write_msg(&mut writer, data).unwrap();
            assert_eq!(
                writer.writes.len(),
                1,
                "a {}-byte message took {} writes",
                data.len(),
                writer.writes.len()
            );

            let mut separate = RecordingWriter::default();
            write_prefix_then_payload(&mut separate, data);
            assert_eq!(writer.written(), separate.written());
        }
    }

    #[test]
    fn round_trip() {
        let data = b"hello world";
        let mut buf = Vec::new();
        write_msg(&mut buf, data).unwrap();

        let mut cursor = Cursor::new(buf);
        let result = read_msg(&mut cursor).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn empty_message() {
        let mut buf = Vec::new();
        write_msg(&mut buf, b"").unwrap();

        let mut cursor = Cursor::new(buf);
        let result = read_msg(&mut cursor).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn large_message() {
        let data = vec![42u8; 1_000_000];
        let mut buf = Vec::new();
        write_msg(&mut buf, &data).unwrap();

        let mut cursor = Cursor::new(buf);
        let result = read_msg(&mut cursor).unwrap();
        assert_eq!(result, data);
    }
}
