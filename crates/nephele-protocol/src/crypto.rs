//! AES-128/CFB8 stream cipher, as used by Minecraft's login encryption.
//!
//! Both directions share the same key and IV but evolve independent shift
//! registers, so a connection owns one [`Cfb8`] for reading and one for writing.

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::{Aes128, Block};
use sha1::{Digest, Sha1};

pub struct Cfb8 {
    cipher: Aes128,
    register: [u8; 16],
}

impl Cfb8 {
    pub fn new(key: &[u8; 16], iv: &[u8; 16]) -> Self {
        let cipher = Aes128::new_from_slice(key).expect("AES-128 takes a 16-byte key");
        Self {
            cipher,
            register: *iv,
        }
    }

    fn keystream(&self) -> u8 {
        let mut block: Block = Block::clone_from_slice(&self.register);
        self.cipher.encrypt_block(&mut block);
        block[0]
    }

    fn shift_in(&mut self, byte: u8) {
        self.register.copy_within(1..16, 0);
        self.register[15] = byte;
    }

    pub fn encrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let ciphertext = *byte ^ self.keystream();
            *byte = ciphertext;
            self.shift_in(ciphertext);
        }
    }

    pub fn decrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let ciphertext = *byte;
            *byte = ciphertext ^ self.keystream();
            self.shift_in(ciphertext);
        }
    }
}

/// The `serverId` hash vanilla computes for `joinServer` (SHA-1, Java
/// BigInteger base-16 formatting with two's complement).
pub fn minecraft_server_hash(server_id: &str, shared_secret: &[u8], public_key: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(shared_secret);
    hasher.update(public_key);
    let digest: [u8; 20] = hasher.finalize().into();
    java_hex(&digest)
}

fn java_hex(digest: &[u8; 20]) -> String {
    let negative = digest[0] & 0x80 != 0;
    let bytes = if negative {
        // two's complement negate
        let mut inv = [0u8; 20];
        let mut carry = 1u16;
        for i in (0..20).rev() {
            let v = (!digest[i]) as u16 + carry;
            inv[i] = v as u8;
            carry = v >> 8;
        }
        inv
    } else {
        *digest
    };
    let mut hex = hex::encode(bytes);
    while hex.starts_with('0') && hex.len() > 1 {
        hex.remove(0);
    }
    if negative {
        format!("-{hex}")
    } else {
        hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_bytes(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    /// NIST SP 800-38A F.3.8 (CFB8) example vector.
    #[test]
    fn nist_cfb8_vector() {
        let key: [u8; 16] = hex_bytes("2b7e151628aed2a6abf7158809cf4f3c")
            .try_into()
            .unwrap();
        let iv: [u8; 16] = hex_bytes("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .unwrap();
        let plaintext = hex_bytes("6bc1bee22e409f96e93d7e117393172a");
        let expected = "3b79424c9c0dd436bace9e0ed4586a4f";

        let mut enc = Cfb8::new(&key, &iv);
        let mut buf = plaintext.clone();
        enc.encrypt(&mut buf);
        assert_eq!(hex::encode(&buf), expected);

        let mut dec = Cfb8::new(&key, &iv);
        dec.decrypt(&mut buf);
        assert_eq!(buf, plaintext);
    }

    #[test]
    fn keystream_depends_on_stream_position() {
        let key = [7u8; 16];
        let iv = [9u8; 16];

        let mut fresh = Cfb8::new(&key, &iv);
        let mut at_start = vec![0u8; 3];
        fresh.encrypt(&mut at_start);

        let mut advanced = Cfb8::new(&key, &iv);
        let mut discard = vec![0u8; 5];
        advanced.encrypt(&mut discard);
        let mut later = vec![0u8; 3];
        advanced.encrypt(&mut later);

        assert_ne!(at_start, later);
    }

    #[test]
    fn chunked_writes_match_a_single_write() {
        let key = [3u8; 16];
        let iv = [4u8; 16];
        let plaintext: Vec<u8> = (0..64).collect();

        let mut whole = Cfb8::new(&key, &iv);
        let mut expected = plaintext.clone();
        whole.encrypt(&mut expected);

        let mut chunked = Cfb8::new(&key, &iv);
        let mut actual = plaintext;
        for chunk in actual.chunks_mut(7) {
            chunked.encrypt(chunk);
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn roundtrip_multiple_packets_on_one_stream() {
        let key = [1u8; 16];
        let iv = [2u8; 16];
        let mut enc = Cfb8::new(&key, &iv);
        let mut dec = Cfb8::new(&key, &iv);

        for size in [1usize, 17, 64, 513] {
            let plaintext: Vec<u8> = (0..size).map(|i| (i * 37 % 251) as u8).collect();
            let mut wire = plaintext.clone();
            enc.encrypt(&mut wire);
            dec.decrypt(&mut wire);
            assert_eq!(wire, plaintext, "size {size}");
        }
    }
}

/// A fresh 16-byte shared secret for the login handshake.
pub fn new_shared_secret() -> [u8; 16] {
    use rand::RngCore;
    let mut secret = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    secret
}

/// `CryptManager.getServerIdHash`, hashed and rendered the way the client sends
/// it to the session server.
///
/// The digest is `SHA-1(serverId as ISO-8859-1 || secret || publicKey)`, and the
/// string is Java's `new BigInteger(digest).toString(16)`: big-endian two's
/// complement, so a digest whose top bit is set comes out with a leading `-`
/// and magnitudes lose their leading zeros. Servers see exactly this string.
pub fn server_id_hash(server_id: &str, secret: &[u8], public_key: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(secret);
    hasher.update(public_key);
    java_big_integer_hex(&hasher.finalize())
}

fn java_big_integer_hex(bytes: &[u8]) -> String {
    let negative = bytes.first().is_some_and(|byte| byte & 0x80 != 0);

    let mut magnitude = bytes.to_vec();
    if negative {
        for byte in magnitude.iter_mut() {
            *byte = !*byte;
        }
        let mut carry = 1u16;
        for byte in magnitude.iter_mut().rev() {
            let sum = u16::from(*byte) + carry;
            *byte = (sum & 0xFF) as u8;
            carry = sum >> 8;
        }
    }

    let start = magnitude
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(magnitude.len());
    let digits: String = magnitude[start..]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let digits = digits.trim_start_matches('0');

    if digits.is_empty() {
        "0".to_string()
    } else if negative {
        format!("-{digits}")
    } else {
        digits.to_string()
    }
}

/// RSA-encrypt with the server's public key, as `Cipher.getInstance("RSA/ECB/PKCS1Padding")`
/// does on the vanilla client.
pub fn rsa_encrypt(public_key_der: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use rsa::pkcs8::DecodePublicKey;
    use rsa::{Pkcs1v15Encrypt, RsaPublicKey};

    let key = RsaPublicKey::from_public_key_der(public_key_der)
        .map_err(|err| format!("server public key: {err}"))?;
    key.encrypt(&mut rand::rngs::OsRng, Pkcs1v15Encrypt, data)
        .map_err(|err| format!("RSA encryption failed: {err}"))
}

#[cfg(test)]
mod login_tests {
    use super::*;

    #[test]
    fn a_positive_digest_is_plain_hex() {
        // 0x01 0x02 -> "102"
        assert_eq!(java_big_integer_hex(&[0x01, 0x02]), "102");
        assert_eq!(java_big_integer_hex(&[0x7f, 0xff]), "7fff");
    }

    #[test]
    fn a_negative_digest_starts_with_a_minus() {
        // 0xff is -1 in two's complement.
        assert_eq!(java_big_integer_hex(&[0xff]), "-1");
        // 0x80 0x00 is -32768.
        assert_eq!(java_big_integer_hex(&[0x80, 0x00]), "-8000");
    }

    #[test]
    fn leading_zeros_are_dropped() {
        assert_eq!(java_big_integer_hex(&[0x00, 0x00, 0x2a]), "2a");
        assert_eq!(java_big_integer_hex(&[0x00]), "0");
    }

    #[test]
    fn the_hash_is_stable_and_hex() {
        let secret = [7u8; 16];
        let key = [9u8; 162];
        let hash = server_id_hash("", &secret, &key);
        assert_eq!(hash, server_id_hash("", &secret, &key));
        assert!(hash
            .trim_start_matches('-')
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn the_server_id_is_part_of_the_hash() {
        let secret = [1u8; 16];
        let key = [2u8; 162];
        assert_ne!(
            server_id_hash("", &secret, &key),
            server_id_hash("x", &secret, &key)
        );
    }

    #[test]
    fn a_fresh_secret_is_sixteen_bytes_and_not_constant() {
        let a = new_shared_secret();
        let b = new_shared_secret();
        assert_eq!(a.len(), 16);
        assert_ne!(a, b);
    }
}
