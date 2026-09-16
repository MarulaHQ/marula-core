//! At-rest secret encryption: AES-256-GCM with a 12-byte random nonce
//! prefixed to the ciphertext. Used for OIDC client secrets and TOTP
//! secrets — never returned after write, never logged (§6 secrets rule).
//!
//! Known gap (documented honestly): this is a single app-level key, not
//! full envelope encryption (per-org data keys wrapped by a master key).
//! Envelope mode is the upgrade path and does not change call sites.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("ciphertext malformed or tampered")]
    Corrupt,
    #[error("encryption unavailable")]
    Backend(String),
}

const NONCE_LEN: usize = 12;

fn cipher(key: &crate::config::SecretKey) -> Result<Aes256Gcm, CryptoError> {
    Aes256Gcm::new_from_slice(&key.0).map_err(|e| CryptoError::Backend(e.to_string()))
}

/// Encrypts `plaintext` into `nonce || ciphertext`.
pub fn seal(key: &crate::config::SecretKey, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let c = cipher(key)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = c
        .encrypt(&nonce, plaintext)
        .map_err(|e| CryptoError::Backend(e.to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a value produced by [`seal`]. Fails closed on any tampering.
pub fn open(key: &crate::config::SecretKey, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() <= NONCE_LEN {
        return Err(CryptoError::Corrupt);
    }
    let c = cipher(key)?;
    let (nonce_bytes, ct) = sealed.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    c.decrypt(nonce, ct).map_err(|_| CryptoError::Corrupt)
}

/// Encrypts `plaintext` into `nonce || ciphertext`, binding the
/// ciphertext to the given ADDITIONAL AUTHENTICATED DATA (the wave-16
/// brief: secretvault ciphertexts bind an AAD of
/// `(org_id, record_id, field_key)` so bits cannot be replayed across
/// coordinates). The wire format is identical to [`seal`] — the AAD is
/// positionally bound by the coordinate, never stored.
pub fn seal_with_aad(
    key: &crate::config::SecretKey,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let c = cipher(key)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = c
        .encrypt(
            &nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| CryptoError::Backend(e.to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a value produced by [`seal_with_aad`]. Fails closed on any
/// tampering or a coordinate mismatch (the AAD roundtrip).
pub fn open_with_aad(
    key: &crate::config::SecretKey,
    sealed: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() <= NONCE_LEN {
        return Err(CryptoError::Corrupt);
    }
    let c = cipher(key)?;
    let (nonce_bytes, ct) = sealed.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    c.decrypt(nonce, aes_gcm::aead::Payload { msg: ct, aad })
        .map_err(|_| CryptoError::Corrupt)
}

/// Fresh identifier for a stored secret row.
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> crate::config::SecretKey {
        crate::config::SecretKey([7u8; 32])
    }

    #[test]
    fn seal_open_roundtrip() {
        let sealed = seal(&key(), b"top-secret-value").unwrap();
        assert_ne!(sealed, b"top-secret-value");
        assert_eq!(open(&key(), &sealed).unwrap(), b"top-secret-value");
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut sealed = seal(&key(), b"payload").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(matches!(open(&key(), &sealed), Err(CryptoError::Corrupt)));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let sealed = seal(&key(), b"payload").unwrap();
        let other = crate::config::SecretKey([9u8; 32]);
        assert!(matches!(open(&other, &sealed), Err(CryptoError::Corrupt)));
    }

    #[test]
    fn nonces_are_fresh_per_seal() {
        let a = seal(&key(), b"same").unwrap();
        let b = seal(&key(), b"same").unwrap();
        assert_ne!(a, b, "nonce reuse would break GCM security");
    }
}
