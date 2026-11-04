use crate::error::{Error, Result};
use crate::read::{encode_varint, PacketReader};

/// A single protocol packet: an id plus its raw payload bytes.
#[derive(Debug)]
pub struct Packet {
    pub id: i32,
    pub data: Vec<u8>,
    /// High-water mark of what a parser read from this packet's payload.
    ///
    /// The outer frame is already isolated when a parser reads this slice.
    ///
    /// Atomic rather than a `Cell` because a `Packet` crosses await points on
    /// multi-threaded runtimes.
    consumed: std::sync::atomic::AtomicUsize,
}

impl Clone for Packet {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            data: self.data.clone(),
            consumed: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl PartialEq for Packet {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.data == other.data
    }
}

impl Eq for Packet {}

impl Packet {
    pub fn new(id: i32, data: Vec<u8>) -> Self {
        Self {
            id,
            data,
            consumed: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn empty(id: i32) -> Self {
        Self::new(id, Vec::new())
    }

    pub fn reader(&self) -> PacketReader<'_> {
        PacketReader::tracked(&self.data, &self.consumed)
    }

    /// Bytes a parser has read from `data`, saturating at the payload length.
    pub fn consumed(&self) -> usize {
        self.consumed
            .load(std::sync::atomic::Ordering::Relaxed)
            .min(self.data.len())
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
        Ok(Self::new(id, reader.into_remaining().to_vec()))
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

    #[test]
    fn a_reader_tracks_how_much_of_the_payload_it_read() {
        let packet = Packet::new(1, vec![1, 2, 3, 4]);
        assert_eq!(packet.consumed(), 0);

        {
            let mut reader = packet.reader();
            reader.read_u8().unwrap();
            reader.read_u16().unwrap();
        }

        assert_eq!(packet.consumed(), 3);
        let mut rest = packet.reader();
        rest.read_remaining();
        assert_eq!(packet.consumed(), 4);
    }
}
