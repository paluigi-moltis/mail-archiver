use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Nonce,
};
use anyhow::Result;

/// Encrypt plaintext using AES-256-GCM with a random nonce.
/// Returns (ciphertext, nonce) — both as Vec<u8>.
#[allow(dead_code)]
pub fn encrypt(master_key_hex: &str, plaintext: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let key_bytes = hex::decode(master_key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)?;
    let nonce_bytes = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce_bytes, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encryption error: {e}"))?;
    Ok((ciphertext, nonce_bytes.to_vec()))
}

/// Decrypt ciphertext using AES-256-GCM with the given nonce.
#[allow(dead_code)]
pub fn decrypt(master_key_hex: &str, nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
    let key_bytes = hex::decode(master_key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)?;
    let nonce = Nonce::from_slice(nonce);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Decryption error: {e}"))?;
    Ok(String::from_utf8(plaintext)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let master_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let plaintext = "my-secret-imap-password";
        let (ct, nonce) = encrypt(master_key, plaintext).unwrap();
        let decrypted = decrypt(master_key, &nonce, &ct).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let master_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let wrong_key = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let plaintext = "my-secret-imap-password";
        let (ct, nonce) = encrypt(master_key, plaintext).unwrap();
        assert!(decrypt(wrong_key, &nonce, &ct).is_err());
    }
}
