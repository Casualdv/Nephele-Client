use crate::error::{Error, Result};

/// Vanilla caps chat and identifier strings at 32767 UTF-16 code units.
pub const MAX_STRING_CHARS: usize = 32767;

#[derive(Debug, Clone)]
pub struct PacketReader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Set when this reader belongs to a [`crate::Packet`]; updated on every
    /// read so the caller can verify the parser consumed the whole payload.
    consumed: Option<&'a std::sync::atomic::AtomicUsize>,
}

impl<'a> PacketReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            consumed: None,
        }
    }

    pub(crate) fn tracked(buf: &'a [u8], consumed: &'a std::sync::atomic::AtomicUsize) -> Self {
        Self {
            buf,
            consumed: Some(consumed),
            pos: 0,
        }
    }

    fn note(&self) {
        if let Some(cell) = self.consumed {
            cell.fetch_max(self.pos, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::UnexpectedEof { needed: n });
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        self.note();
        Ok(slice)
    }

    pub fn read_remaining(&mut self) -> &'a [u8] {
        let slice = &self.buf[self.pos..];
        self.pos = self.buf.len();
        self.note();
        slice
    }

    pub fn into_remaining(self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(self.read_u16()? as i16)
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_i64(&mut self) -> Result<i64> {
        let b = self.take(8)?;
        Ok(i64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_i32()? as u32))
    }

    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_i64()? as u64))
    }

    /// Minecraft's signed 32-bit varint: little-endian base-128, max 5 bytes.
    pub fn read_varint(&mut self) -> Result<i32> {
        let mut value: u32 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Ok(value as i32);
            }
            shift += 7;
            if shift >= 32 {
                return Err(Error::VarIntTooBig);
            }
        }
    }

    pub fn read_varlong(&mut self) -> Result<i64> {
        let mut value: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            value |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(value as i64);
            }
            shift += 7;
            if shift >= 64 {
                return Err(Error::VarLongTooLong);
            }
        }
    }

    pub fn read_string(&mut self) -> Result<String> {
        self.read_string_max(MAX_STRING_CHARS)
    }

    pub fn read_string_max(&mut self, max_chars: usize) -> Result<String> {
        let len = self.read_varint()?;
        if len < 0 {
            return Err(Error::Protocol(format!("negative string length {len}")));
        }
        let len = len as usize;
        if len > max_chars.saturating_mul(4) {
            return Err(Error::StringTooLong {
                len,
                max: max_chars * 4,
            });
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|e| Error::Protocol(format!("string is not valid utf-8: {e}")))
    }

    /// Length-prefixed byte array (VarInt length followed by raw bytes).
    pub fn read_byte_array(&mut self) -> Result<Vec<u8>> {
        let len = self.read_varint()?;
        if len < 0 {
            return Err(Error::Protocol(format!("negative array length {len}")));
        }
        Ok(self.take(len as usize)?.to_vec())
    }

    pub fn read_uuid(&mut self) -> Result<[u8; 16]> {
        let b = self.take(16)?;
        let mut out = [0u8; 16];
        out.copy_from_slice(b);
        Ok(out)
    }

    /// 1.8.9 packed block position: x(26) | y(12) | z(26), two's complement.
    pub fn read_position(&mut self) -> Result<(i32, i32, i32)> {
        let val = self.read_i64()? as u64;
        let x = (val >> 38) as i32;
        let y = ((val >> 26) & 0xFFF) as i32;
        let z = (val & 0x3FF_FFFF) as i32;
        let sign_extend = |v: i32, bits: u32| -> i32 { (v << (32 - bits)) >> (32 - bits) };
        Ok((sign_extend(x, 26), sign_extend(y, 12), sign_extend(z, 26)))
    }
}

/// Encode a signed 32-bit value in Minecraft's varint form.
pub fn encode_varint(value: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    let mut v = value as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return out;
        }
        out.push(((v & 0x7F) as u8) | 0x80);
        v >>= 7;
    }
}

/// Number of bytes [`encode_varint`] would produce.
pub fn varint_len(value: i32) -> usize {
    encode_varint(value).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_known_encodings() {
        assert_eq!(encode_varint(0), vec![0x00]);
        assert_eq!(encode_varint(1), vec![0x01]);
        assert_eq!(encode_varint(127), vec![0x7F]);
        assert_eq!(encode_varint(128), vec![0x80, 0x01]);
        assert_eq!(encode_varint(255), vec![0xFF, 0x01]);
        assert_eq!(
            encode_varint(2_147_483_647),
            vec![0xFF, 0xFF, 0xFF, 0xFF, 0x07]
        );
        assert_eq!(encode_varint(-1), vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        assert_eq!(
            encode_varint(-2_147_483_648),
            vec![0x80, 0x80, 0x80, 0x80, 0x08]
        );
    }

    #[test]
    fn varint_roundtrip() {
        let samples = [
            0,
            1,
            -1,
            2,
            127,
            128,
            255,
            256,
            16_383,
            16_384,
            2_097_151,
            2_097_152,
            268_435_455,
            268_435_456,
            2_147_483_647,
            -2_147_483_648,
        ];
        for v in samples {
            let bytes = encode_varint(v);
            let mut r = PacketReader::new(&bytes);
            assert_eq!(r.read_varint().unwrap(), v, "value {v}");
            assert!(r.is_empty());
        }
    }

    #[test]
    fn varint_rejects_overlong() {
        let bytes = [0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
        let mut r = PacketReader::new(&bytes);
        assert!(matches!(r.read_varint(), Err(Error::VarIntTooBig)));
    }

    #[test]
    fn varlong_roundtrip() {
        for v in [0i64, 1, -1, i64::MAX, i64::MIN, 123_456_789_012] {
            let mut out = Vec::new();
            let mut x = v as u64;
            loop {
                if x & !0x7F == 0 {
                    out.push(x as u8);
                    break;
                }
                out.push(((x & 0x7F) as u8) | 0x80);
                x >>= 7;
            }
            let mut r = PacketReader::new(&out);
            assert_eq!(r.read_varlong().unwrap(), v);
        }
    }

    #[test]
    fn primitive_roundtrip() {
        let bytes = [0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x01];
        let mut r = PacketReader::new(&bytes);
        assert!(!r.read_bool().unwrap());
        assert!(r.read_bool().unwrap());
        assert_eq!(r.read_i16().unwrap(), 0);
        assert_eq!(r.read_i32().unwrap(), 42);
        assert!(r.read_bool().unwrap());
        assert!(r.is_empty());
    }

    #[test]
    fn string_roundtrip() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_varint(5));
        bytes.extend_from_slice(b"hello");
        let mut r = PacketReader::new(&bytes);
        assert_eq!(r.read_string().unwrap(), "hello");
    }

    #[test]
    fn string_rejects_negative_length() {
        let bytes = encode_varint(-1);
        let mut r = PacketReader::new(&bytes);
        assert!(matches!(r.read_string(), Err(Error::Protocol(_))));
    }

    #[test]
    fn position_roundtrip() {
        let x = 12_345;
        let y = 64;
        let z = -77;
        let packed = (((x as i64) & 0x3FF_FFFF) << 38)
            | (((y as i64) & 0xFFF) << 26)
            | ((z as i64) & 0x3FF_FFFF);
        let bytes = packed.to_be_bytes();
        let mut r = PacketReader::new(&bytes);
        assert_eq!(r.read_position().unwrap(), (x, y, z));
    }
}
