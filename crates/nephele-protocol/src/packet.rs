use crate::error::{Error, Result};
use crate::read::{encode_varint, PacketReader};

/// A single protocol packet: an id plus its raw payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub id: i32,
    pub data: Vec<u8>,
}

impl Packet {
    pub fn new(id: i32, data: Vec<u8>) -> Self {
        Self { id, data }
    }

    pub fn empty(id: i32) -> Self {
        Self {
            id,
            data: Vec::new(),
        }
    }

    pub fn reader(&self) -> PacketReader<'_> {
        PacketReader::new(&self.data)
    }

    /// `VarInt id` followed by the payload — what the framer wraps.
    pub fn to_payload(&self) -> Vec<u8> {
        let mut out = encode_varint(self.id);
        out.extend_from_slice(&self.data);
        out
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self> {
        let mut reader = PacketReader::new(payload);
        let id = reader.read_varint()?;
        Ok(Self {
            id,
            data: reader.into_remaining().to_vec(),
        })
    }

    pub fn check_id(&self, expected: i32, state: &'static str) -> Result<()> {
        if self.id != expected {
            return Err(Error::UnexpectedPacket { id: self.id, state });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let original = Packet::new(0x21, vec![1, 2, 3, 4]);
        let bytes = original.to_payload();
        assert_eq!(bytes[0], 0x21);
        assert_eq!(Packet::from_payload(&bytes).unwrap(), original);
    }

    #[test]
    fn two_byte_packet_id() {
        let original = Packet::new(300, vec![9]);
        let bytes = original.to_payload();
        assert_eq!(&bytes[..2], &[0xAC, 0x02]);
        assert_eq!(Packet::from_payload(&bytes).unwrap(), original);
    }
}
