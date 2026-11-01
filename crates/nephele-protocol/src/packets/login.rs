use crate::error::Result;
use crate::packet::Packet;
use crate::packets::status::flatten_chat;
use crate::write::PacketWriter;

pub const SERVERBOUND_LOGIN_START: i32 = 0x00;
pub const SERVERBOUND_ENCRYPTION_RESPONSE: i32 = 0x01;

pub const CLIENTBOUND_DISCONNECT: i32 = 0x00;
pub const CLIENTBOUND_ENCRYPTION_REQUEST: i32 = 0x01;
pub const CLIENTBOUND_LOGIN_SUCCESS: i32 = 0x02;
pub const CLIENTBOUND_SET_COMPRESSION: i32 = 0x03;

pub fn login_start(username: &str) -> Packet {
    let mut w = PacketWriter::new();
    w.write_string(username);
    Packet::new(SERVERBOUND_LOGIN_START, w.into_bytes())
}

#[derive(Debug, Clone)]
pub struct LoginSuccess {
    pub uuid: String,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct EncryptionRequest {
    pub server_id: String,
    pub public_key: Vec<u8>,
    pub verify_token: Vec<u8>,
}

pub fn encryption_response(shared_secret: &[u8], verify_token: &[u8]) -> Packet {
    let mut w = PacketWriter::new();
    w.write_byte_array(shared_secret);
    w.write_byte_array(verify_token);
    Packet::new(SERVERBOUND_ENCRYPTION_RESPONSE, w.into_bytes())
}

pub fn parse_login_success(packet: &Packet) -> Result<LoginSuccess> {
    let mut r = packet.reader();
    Ok(LoginSuccess {
        uuid: r.read_string()?,
        username: r.read_string()?,
    })
}

pub fn parse_disconnect(packet: &Packet) -> Result<String> {
    let json = packet.reader().read_string()?;
    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(value) => Ok(flatten_chat(&value)),
        Err(_) => Ok(json),
    }
}

pub fn parse_set_compression(packet: &Packet) -> Result<u32> {
    let threshold = packet.reader().read_varint()?;
    Ok(threshold.max(0) as u32)
}

pub fn parse_encryption_request(packet: &Packet) -> Result<EncryptionRequest> {
    let mut r = packet.reader();
    let server_id = r.read_string()?;
    let public_key = r.read_byte_array()?;
    let verify_token = r.read_byte_array()?;
    Ok(EncryptionRequest {
        server_id,
        public_key,
        verify_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_start_carries_the_username() {
        let packet = login_start("Nephele");
        assert_eq!(packet.id, 0x00);
        assert_eq!(packet.reader().read_string().unwrap(), "Nephele");
    }

    #[test]
    fn parses_login_success_uuid_and_name() {
        let mut w = PacketWriter::new();
        w.write_string("069a79f4-44e9-4726-a5be-fca90e38aaf5")
            .write_string("Notch");
        let success =
            parse_login_success(&Packet::new(CLIENTBOUND_LOGIN_SUCCESS, w.into_bytes())).unwrap();
        assert_eq!(success.username, "Notch");
        assert_eq!(success.uuid, "069a79f4-44e9-4726-a5be-fca90e38aaf5");
    }

    #[test]
    fn parses_disconnect_chat_json() {
        let mut w = PacketWriter::new();
        w.write_string(r#"{"text":"You are not whitelisted"}"#);
        let message =
            parse_disconnect(&Packet::new(CLIENTBOUND_DISCONNECT, w.into_bytes())).unwrap();
        assert_eq!(message, "You are not whitelisted");
    }

    #[test]
    fn parses_set_compression_threshold() {
        let mut w = PacketWriter::new();
        w.write_varint(256);
        assert_eq!(
            parse_set_compression(&Packet::new(CLIENTBOUND_SET_COMPRESSION, w.into_bytes()))
                .unwrap(),
            256
        );
    }
}
