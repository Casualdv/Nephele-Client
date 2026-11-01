use crate::read::encode_varint;

/// Scratch buffer for building packet payloads with Minecraft's wire types.
#[derive(Debug, Default, Clone)]
pub struct PacketWriter {
    buf: Vec<u8>,
}

impl PacketWriter {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(64),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn write_u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn write_i8(&mut self, v: i8) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    pub fn write_bool(&mut self, v: bool) -> &mut Self {
        self.buf.push(u8::from(v));
        self
    }

    pub fn write_u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn write_i16(&mut self, v: i16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn write_i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn write_i64(&mut self, v: i64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn write_f32(&mut self, v: f32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn write_f64(&mut self, v: f64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn write_varint(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&encode_varint(v));
        self
    }

    pub fn write_varlong(&mut self, value: i64) -> &mut Self {
        let mut v = value as u64;
        loop {
            if v & !0x7F == 0 {
                self.buf.push(v as u8);
                break;
            }
            self.buf.push(((v & 0x7F) as u8) | 0x80);
            v >>= 7;
        }
        self
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    /// VarInt byte length followed by UTF-8 bytes (vanilla `String`).
    pub fn write_string(&mut self, s: &str) -> &mut Self {
        self.write_varint(s.len() as i32);
        self.buf.extend_from_slice(s.as_bytes());
        self
    }

    /// VarInt byte length followed by the raw bytes (vanilla `byte array`).
    pub fn write_byte_array(&mut self, bytes: &[u8]) -> &mut Self {
        self.write_varint(bytes.len() as i32);
        self.buf.extend_from_slice(bytes);
        self
    }

    pub fn write_uuid(&mut self, uuid: &[u8; 16]) -> &mut Self {
        self.buf.extend_from_slice(uuid);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::PacketReader;

    #[test]
    fn string_has_varint_length_prefix() {
        let mut w = PacketWriter::new();
        w.write_string("hi");
        assert_eq!(w.as_slice(), &[0x02, b'h', b'i']);
    }

    #[test]
    fn byte_array_has_varint_length_prefix() {
        let mut w = PacketWriter::new();
        w.write_byte_array(&[1, 2, 3]);
        assert_eq!(w.as_slice(), &[0x03, 1, 2, 3]);
    }

    #[test]
    fn writer_reader_roundtrip() {
        let mut w = PacketWriter::new();
        w.write_varint(300)
            .write_string("nephele")
            .write_bool(true)
            .write_i64(-9_999_999_999);

        let bytes = w.into_bytes();
        let mut r = PacketReader::new(&bytes);
        assert_eq!(r.read_varint().unwrap(), 300);
        assert_eq!(r.read_string().unwrap(), "nephele");
        assert!(r.read_bool().unwrap());
        assert_eq!(r.read_i64().unwrap(), -9_999_999_999);
        assert!(r.is_empty());
    }
}
