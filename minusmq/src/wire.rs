//! Length-prefixed wire format: [u32 big-endian length][payload bytes].

use anyhow::{Context, Result};
use std::io::{Read, Write};

/// Write a length-prefixed message to the writer.
pub fn write_msg(writer: &mut impl Write, data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).context("message too large (>4GB)")?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(data)?;
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
