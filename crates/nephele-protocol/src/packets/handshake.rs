use crate::packet::Packet;
use crate::state::NextState;
use crate::write::PacketWriter;
use crate::PROTOCOL_VERSION;

pub const ID: i32 = 0x00;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: NextState,
}

impl Handshake {
    pub fn new(server_address: impl Into<String>, server_port: u16, next_state: NextState) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            server_address: server_address.into(),
            server_port,
            next_state,
        }
    }

    pub fn to_packet(&self) -> Packet {
        let mut w = PacketWriter::new();
        w.write_varint(self.protocol_version)
            .write_string(&self.server_address)
            .write_u16(self.server_port)
            .write_varint(self.next_state as i32);
        Packet::new(ID, w.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::PacketReader;

    #[test]
    fn encodes_protocol_47_status_handshake() {
        let packet = Handshake::new("mc.example.net", 25565, NextState::Status).to_packet();
        assert_eq!(packet.id, 0x00);
        let mut r = packet.reader();
        assert_eq!(r.read_varint().unwrap(), 47);
        assert_eq!(r.read_string().unwrap(), "mc.example.net");
        assert_eq!(r.read_u16().unwrap(), 25565);
        assert_eq!(r.read_varint().unwrap(), 1);
        assert!(r.is_empty());
    }

    #[test]
    fn reads_handshake_back() {
        let original = Handshake::new("localhost", 25566, NextState::Login);
        let bytes = original.to_packet().data;
        let mut r = PacketReader::new(&bytes);
        assert_eq!(r.read_varint().unwrap(), 47);
        assert_eq!(r.read_string().unwrap(), "localhost");
        assert_eq!(r.read_u16().unwrap(), 25566);
        assert_eq!(r.read_varint().unwrap(), 2);
    }
}
