// SPDX-License-Identifier: AGPL-3.0-or-later
//! Encryption at rest for third-party credentials (payment gateway key, webhook secret).
//! The key lives outside the database: `CUPPA_SECRET_KEY` (64 hex chars) or a generated `secret.key` file (0600).

use aes_gcm::{
    Aes256Gcm, Key, KeyInit,
    aead::{Aead, Nonce},
};
use std::path::Path;

pub type SecretKey = [u8; 32];

pub fn load_or_create(db_url: &str) -> anyhow::Result<SecretKey> {
    if let Ok(v) = std::env::var("CUPPA_SECRET_KEY") {
        let b = hex::decode(v.trim())?;
        return b
            .try_into()
            .map_err(|_| anyhow::anyhow!("CUPPA_SECRET_KEY must be 64 hex characters"));
    }
    let Some(dir) = crate::backup::dir_for(db_url).map(|b| b.with_file_name("secret.key")) else {
        // in-memory database (tests): ephemeral key
        let mut k = [0u8; 32];
        getrandom::fill(&mut k).map_err(|e| anyhow::anyhow!("rng: {e}"))?;
        return Ok(k);
    };
    read_or_write(&dir)
}

fn read_or_write(path: &Path) -> anyhow::Result<SecretKey> {
    if let Ok(s) = std::fs::read_to_string(path) {
        return hex::decode(s.trim())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("corrupt secret.key"));
    }
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).map_err(|e| anyhow::anyhow!("rng: {e}"))?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(hex::encode(k).as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, hex::encode(k))?;
    Ok(k)
}

pub fn encrypt(key: &SecretKey, plaintext: &str) -> String {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::try_from(&key[..]).expect("32-byte key"));
    let mut nonce_bytes = [0u8; 12];
    getrandom::fill(&mut nonce_bytes).expect("OS random source unavailable");
    let nonce = Nonce::<Aes256Gcm>::try_from(&nonce_bytes[..]).expect("12-byte nonce");
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .expect("AES-GCM encryption cannot fail for in-memory data");
    hex::encode([nonce_bytes.as_slice(), ct.as_slice()].concat())
}

pub fn decrypt(key: &SecretKey, blob: &str) -> Option<String> {
    let raw = hex::decode(blob).ok()?;
    if raw.len() < 12 + 16 {
        return None;
    }
    let (n, ct) = raw.split_at(12);
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::try_from(&key[..]).expect("32-byte key"));
    let pt = cipher.decrypt(&Nonce::<Aes256Gcm>::try_from(n).ok()?, ct).ok()?;
    String::from_utf8(pt).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper_detection() {
        let key = [7u8; 32];
        let blob = encrypt(&key, "SB-Mid-server-secret");
        assert!(!blob.contains("SB-Mid"));
        assert_eq!(decrypt(&key, &blob).as_deref(), Some("SB-Mid-server-secret"));
        // different nonce each time
        assert_ne!(encrypt(&key, "x"), encrypt(&key, "x"));
        // wrong key / flipped byte fail closed
        assert!(decrypt(&[8u8; 32], &blob).is_none());
        let mut bad = hex::decode(&blob).unwrap();
        *bad.last_mut().unwrap() ^= 1;
        assert!(decrypt(&key, &hex::encode(bad)).is_none());
    }
}
