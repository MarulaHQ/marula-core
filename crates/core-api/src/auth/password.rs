//! Argon2id password hashing (§6). PHC-string format, random salt per hash.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;

/// Hashes a password with argon2id. Returns the PHC string for storage.
pub fn hash(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
}

/// Constant-time verification against a stored PHC string.
pub fn verify(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Blocking-pool wrappers (Wave 49 lane R3).
//
// Argon2id is deliberately memory-hard: one hash holds ~19 MiB and costs tens
// of milliseconds of CPU. Left on the async runtime that parks a worker thread
// for the whole window, so a burst of signups, logins or password resets caps
// the process at (cores / hash cost) and starves every other task sharing the
// worker. These wrappers run the same sync `hash`/`verify` on the blocking
// pool, which exists for exactly this kind of work. The request-path callers
// use them; the sync forms stay for tests and any genuinely synchronous use.
// ---------------------------------------------------------------------------

/// Async argon2id hash: runs on the blocking pool.
pub async fn hash_async(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash(&password).map_err(|e| e.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!("argon2 hashing task failed: {e}"))?
        .map_err(|e| anyhow::anyhow!("argon2 hashing failed: {e}"))
}

/// Async batch hash in one blocking-pool task. A request that mints several
/// secrets at once (MFA recovery codes) pays the argon2 cost once per item,
/// so it is worse off the runtime than a single hash is.
pub async fn hash_many_async(passwords: Vec<String>) -> anyhow::Result<Vec<String>> {
    tokio::task::spawn_blocking(move || {
        passwords
            .iter()
            .map(|p| hash(p).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, String>>()
    })
    .await
    .map_err(|e| anyhow::anyhow!("argon2 hashing task failed: {e}"))?
    .map_err(|e| anyhow::anyhow!("argon2 hashing failed: {e}"))
}

/// Async constant-time verification on the blocking pool. A task that panics
/// or is cancelled denies the login (returns false), never grants it.
pub async fn verify_async(password: String, phc: String) -> bool {
    tokio::task::spawn_blocking(move || verify(&password, &phc))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let h = hash("correct horse battery staple").unwrap();
        assert!(verify("correct horse battery staple", &h));
        assert!(!verify("wrong", &h));
    }

    #[test]
    fn malformed_phc_is_rejected_not_panic() {
        assert!(!verify("x", "not-a-hash"));
    }

    #[tokio::test]
    async fn async_wrappers_roundtrip_on_the_blocking_pool() {
        let h = hash_async("correct horse battery staple".into())
            .await
            .unwrap();
        assert!(verify_async("correct horse battery staple".into(), h.clone()).await);
        assert!(!verify_async("wrong".into(), h).await);
    }

    #[tokio::test]
    async fn hash_many_hashes_each_input_in_order() {
        let out = hash_many_async(vec!["first".into(), "second".into()])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert!(verify_async("first".into(), out[0].clone()).await);
        assert!(verify_async("second".into(), out[1].clone()).await);
    }
}
