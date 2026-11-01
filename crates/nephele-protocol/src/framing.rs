//! Packet framing: `VarInt length` + optional zlib compression.
//!
//! Vanilla layout with compression disabled:
//! ```text
//! [VarInt packet length][VarInt packet id][data...]
//! ```
//! With compression enabled, the body after the length starts with the
//! *uncompressed* length; `0` means "stored uncompressed", anything else means
//! the remainder is a zlib stream of that many bytes.

use std::io::{Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::error::{Error, Result};
use crate::read::encode_varint;

/// Hard cap on a single framed packet body (vanilla uses 2 MiB).
pub const MAX_PACKET_LEN: usize = 2 * 1024 * 1024;
/// Hard cap on a decompressed packet; guards against zlib bombs.
pub const MAX_UNCOMPRESSED_LEN: usize = 8 * 1024 * 1024;

pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| Error::Compression(e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| Error::Compression(e.to_string()))
}

pub fn decompress(data: &[u8], max_len: usize) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data).take(max_len as u64 + 1);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| Error::Compression(e.to_string()))?;
    if out.len() > max_len {
        return Err(Error::Compression(format!(
            "decompressed packet exceeds {max_len} bytes"
        )));
    }
    Ok(out)
}

/// Encode a payload (packet id + data) into a length-prefixed frame body.
pub fn encode_frame(payload: &[u8], threshold: Option<u32>) -> Result<Vec<u8>> {
    if payload.len() > MAX_UNCOMPRESSED_LEN {
        return Err(Error::PacketTooLarge {
            len: payload.len(),
            max: MAX_UNCOMPRESSED_LEN,
        });
    }

    let mut body = Vec::with_capacity(payload.len() + 8);
    match threshold {
        None => body.extend_from_slice(payload),
        Some(threshold) if payload.len() < threshold as usize => {
            body.extend_from_slice(&encode_varint(0));
            body.extend_from_slice(payload);
        }
        Some(_) => {
            body.extend_from_slice(&encode_varint(payload.len() as i32));
            body.extend_from_slice(&compress(payload)?);
        }
    }

    let mut frame = Vec::with_capacity(body.len() + 5);
    frame.extend_from_slice(&encode_varint(body.len() as i32));
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decode the body of a frame (everything after the length prefix) back into a
/// payload. `threshold` must be the threshold in effect *before* this packet
/// was processed.
pub fn decode_body(body: &[u8], threshold: Option<u32>) -> Result<Vec<u8>> {
    let Some(_) = threshold else {
        if body.len() > MAX_PACKET_LEN {
            return Err(Error::PacketTooLarge {
                len: body.len(),
                max: MAX_PACKET_LEN,
            });
        }
        return Ok(body.to_vec());
    };

    let mut cursor = 0usize;
    let mut declared: u32 = 0;
    let mut shift = 0u32;
    loop {
        let Some(&byte) = body.get(cursor) else {
            return Err(Error::UnexpectedEof { needed: 1 });
        };
        cursor += 1;
        declared |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 32 {
            return Err(Error::VarIntTooBig);
        }
    }

    let rest = &body[cursor..];
    if declared == 0 {
        if rest.len() > MAX_PACKET_LEN {
            return Err(Error::PacketTooLarge {
                len: rest.len(),
                max: MAX_PACKET_LEN,
            });
        }
        return Ok(rest.to_vec());
    }

    let declared = declared as usize;
    if declared > MAX_UNCOMPRESSED_LEN {
        return Err(Error::PacketTooLarge {
            len: declared,
            max: MAX_UNCOMPRESSED_LEN,
        });
    }

    let payload = decompress(rest, declared)?;
    if payload.len() != declared {
        return Err(Error::Compression(format!(
            "declared {} bytes but got {}",
            declared,
            payload.len()
        )));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::PacketReader;
    use crate::write::PacketWriter;

    fn payload(id: i32, data: &[u8]) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_varint(id).write_bytes(data);
        w.into_bytes()
    }

    fn split_frame(frame: &[u8]) -> Vec<u8> {
        let mut r = PacketReader::new(frame);
        let len = r.read_varint().unwrap() as usize;
        assert_eq!(len, r.remaining(), "length prefix must match body");
        r.read_remaining().to_vec()
    }

    #[test]
    fn roundtrip_without_compression() {
        let p = payload(0x00, b"hello");
        let frame = encode_frame(&p, None).unwrap();
        let body = split_frame(&frame);
        assert_eq!(decode_body(&body, None).unwrap(), p);
    }

    #[test]
    fn small_payload_stored_uncompressed_when_threshold_set() {
        let p = payload(0x00, b"hi");
        let frame = encode_frame(&p, Some(256)).unwrap();
        let body = split_frame(&frame);
        assert_eq!(body[0], 0x00, "uncompressed-length marker");
        assert_eq!(decode_body(&body, Some(256)).unwrap(), p);
    }

    #[test]
    fn large_payload_compressed_when_threshold_set() {
        let data = vec![0xAAu8; 4096];
        let p = payload(0x21, &data);
        let frame = encode_frame(&p, Some(16)).unwrap();
        let body = split_frame(&frame);
        assert_ne!(body[0], 0x00, "should carry a real length prefix");
        assert!(body.len() < p.len(), "zlib should shrink repetitive data");
        assert_eq!(decode_body(&body, Some(16)).unwrap(), p);
    }

    #[test]
    fn rejects_declared_size_mismatch() {
        let p = payload(0x00, &[1u8; 100]);
        let frame = encode_frame(&p, Some(8)).unwrap();
        let body = split_frame(&frame);
        let tampered = &body[1..];
        let mut w = PacketWriter::new();
        w.write_varint(999).write_bytes(tampered);
        assert!(decode_body(&w.into_bytes(), Some(8)).is_err());
    }
}
