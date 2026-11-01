use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("varint is longer than 5 bytes")]
    VarIntTooBig,

    #[error("varlong is longer than 10 bytes")]
    VarLongTooLong,

    #[error("string is {len} bytes but the limit is {max}")]
    StringTooLong { len: usize, max: usize },

    #[error("unexpected end of packet: needed {needed} more byte(s)")]
    UnexpectedEof { needed: usize },

    #[error("packet of {len} bytes exceeds the limit of {max} bytes")]
    PacketTooLarge { len: usize, max: usize },

    #[error("unexpected packet id 0x{id:02X} while in the {state} state")]
    UnexpectedPacket { id: i32, state: &'static str },

    #[error("malformed json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("compression failure: {0}")]
    Compression(String),

    #[error("server requested encryption, but this client is offline-mode only")]
    EncryptionUnsupported,

    #[error("server disconnected us: {0}")]
    Disconnected(String),

    #[error("could not resolve `{input}` to a host and port")]
    BadAddress { input: String },
}

pub type Result<T> = std::result::Result<T, Error>;
