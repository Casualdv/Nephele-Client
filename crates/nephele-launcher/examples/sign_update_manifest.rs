use std::path::PathBuf;

use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use sha2::Sha256;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let key_path = PathBuf::from(args.next().ok_or("missing private key path")?);
    let manifest_path = PathBuf::from(args.next().ok_or("missing manifest path")?);
    let signature_path = PathBuf::from(args.next().ok_or("missing signature path")?);
    if args.next().is_some() {
        return Err("usage: sign_update_manifest <key.pem> <manifest.json> <signature.bin>".into());
    }

    let key_pem = std::fs::read_to_string(key_path)?;
    let private_key = RsaPrivateKey::from_pkcs8_pem(&key_pem)?;
    let expected_modulus = std::env::var("NEPHELE_UPDATE_PUBLIC_MODULUS_HEX")?;
    let actual_modulus = private_key.to_public_key().n().to_str_radix(16);
    if !actual_modulus.eq_ignore_ascii_case(expected_modulus.trim()) {
        return Err(format!(
            "update signing key mismatch; actual public modulus: {actual_modulus}"
        )
        .into());
    }

    let payload = std::fs::read(manifest_path)?;
    let signing_key = SigningKey::<Sha256>::new(private_key);
    let signature = signing_key.sign(&payload);
    std::fs::write(signature_path, signature.to_vec())?;
    Ok(())
}
