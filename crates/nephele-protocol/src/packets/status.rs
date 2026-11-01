use std::time::Duration;

use serde_json::Value;

use crate::error::Result;
use crate::packet::Packet;
use crate::write::PacketWriter;

pub const SERVERBOUND_REQUEST: i32 = 0x00;
pub const SERVERBOUND_PING: i32 = 0x01;
pub const CLIENTBOUND_RESPONSE: i32 = 0x00;
pub const CLIENTBOUND_PONG: i32 = 0x01;

pub fn request_packet() -> Packet {
    Packet::empty(SERVERBOUND_REQUEST)
}

pub fn ping_packet(nonce: i64) -> Packet {
    let mut w = PacketWriter::new();
    w.write_i64(nonce);
    Packet::new(SERVERBOUND_PING, w.into_bytes())
}

#[derive(Debug, Clone, Default)]
pub struct StatusResponse {
    pub version_name: Option<String>,
    pub protocol: Option<i32>,
    pub max_players: Option<i32>,
    pub online_players: Option<i32>,
    pub description: String,
    pub favicon: Option<String>,
    pub latency: Option<Duration>,
    pub raw: Value,
}

pub fn parse_response(packet: &Packet) -> Result<StatusResponse> {
    let json = packet.reader().read_string()?;
    let raw: Value = serde_json::from_str(&json)?;

    let version = raw.get("version");
    let players = raw.get("players");

    Ok(StatusResponse {
        version_name: version
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        protocol: version
            .and_then(|v| v.get("protocol"))
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        max_players: players
            .and_then(|p| p.get("max"))
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        online_players: players
            .and_then(|p| p.get("online"))
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        description: raw.get("description").map(flatten_chat).unwrap_or_default(),
        favicon: raw
            .get("favicon")
            .and_then(Value::as_str)
            .map(str::to_owned),
        latency: None,
        raw,
    })
}

pub fn parse_pong(packet: &Packet) -> Result<i64> {
    packet.reader().read_i64()
}

/// Flatten a chat component into plain text (honours `text`, `extra`,
/// `translate`, `with` and array forms).
pub fn flatten_chat(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => items.iter().map(flatten_chat).collect(),
        Value::Object(map) => {
            let mut out = String::new();
            if let Some(Value::String(text)) = map.get("text") {
                out.push_str(text);
            } else if let Some(Value::String(translate)) = map.get("translate") {
                out.push_str(translate);
            }
            if let Some(Value::Array(with)) = map.get("with") {
                for item in with {
                    out.push_str(&flatten_chat(item));
                }
            }
            if let Some(Value::Array(extra)) = map.get("extra") {
                for item in extra {
                    out.push_str(&flatten_chat(item));
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_vanilla_style_response() {
        let json = r#"{"version":{"name":"1.8.9","protocol":47},"players":{"max":20,"online":3},"description":{"text":"A Nephele Test ","extra":[{"text":"Server","color":"red"}]}}"#;
        let mut w = PacketWriter::new();
        w.write_string(json);
        let packet = Packet::new(CLIENTBOUND_RESPONSE, w.into_bytes());

        let status = parse_response(&packet).unwrap();
        assert_eq!(status.version_name.as_deref(), Some("1.8.9"));
        assert_eq!(status.protocol, Some(47));
        assert_eq!(status.max_players, Some(20));
        assert_eq!(status.online_players, Some(3));
        assert_eq!(status.description, "A Nephele Test Server");
    }

    #[test]
    fn handles_plain_string_description() {
        let json = r#"{"description":"Just a string"}"#;
        let mut w = PacketWriter::new();
        w.write_string(json);
        let status = parse_response(&Packet::new(CLIENTBOUND_RESPONSE, w.into_bytes())).unwrap();
        assert_eq!(status.description, "Just a string");
    }

    #[test]
    fn ping_payload_is_a_long() {
        let packet = ping_packet(-1234567);
        assert_eq!(packet.reader().read_i64().unwrap(), -1234567);
    }
}
