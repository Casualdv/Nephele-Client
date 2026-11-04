use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os().nth(1).map(PathBuf::from).ok_or(
        "usage: cargo run -p nephele-launcher --example generate_update_key -- <private-key.pem>",
    )?;
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let private = RsaPrivateKey::new(&mut rand::rngs::OsRng, 3072)?;
    let public = private.to_public_key();
    let pem = private.to_pkcs8_pem(LineEnding::LF)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(pem.as_bytes())?;
    file.sync_all()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    println!(
        "Private update-signing key saved outside the repository at {}",
        path.display()
    );
    println!("Back up this private key securely; it cannot be recovered if lost.");
    println!(
        "NEPHELE_UPDATE_PUBLIC_MODULUS_HEX={}",
        public.n().to_str_radix(16)
    );
    Ok(())
}
