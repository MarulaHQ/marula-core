//! Argon2id password hashing (§6). PHC-string format, random salt per hash.

use std::sync::atomic::{AtomicU64, Ordering};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

/// Hashes a password with argon2id. Returns the PHC string for storage.
///
/// argon2 0.6 generates the salt inside `hash_password` (the `getrandom`
/// feature is on by default), so the salt is no longer ours to build.
pub fn hash(password: &str) -> Result<String, argon2::password_hash::Error> {
    Argon2::default()
        .hash_password(password.as_bytes())
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

// ---------------------------------------------------------------------------
// Unknown-address equalisation (Wave 54 lane S2c).
//
// A login arm that resolves no identity used to answer WITHOUT hashing
// anything, while the same arm with a real address paid a full argon2id
// verify before saying "invalid credentials". Two answers, same bytes, ~45 ms
// apart: the response time was the account-existence oracle the uniform 401
// exists to deny. The arms now both pay exactly one verify — the resolved one
// against the user's stored hash, the unresolved one against this fixed
// stand-in. Its answer is discarded; only its cost is used.
// ---------------------------------------------------------------------------

/// A fixed argon2id PHC string used as the hash for addresses that do not
/// exist. Generated with `Argon2::default()` through this crate's
/// `password_hash`, so it carries the same `m`/`t`/`p` as a stored user hash.
/// The password it was made from is irrelevant: nobody can sign in as it,
/// because no login arm reads this verify's answer.
pub const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$UZovfOQTuGaSBd1HHorDMQ$TttEENiLuYz9I8qdRCdxoMk1Y6T06MwCGBCrXrC++9s";

/// How many dummy verifies this process has performed. One increment is a
/// relaxed atomic, so the observable costs less than a nanosecond against a
/// verify that costs tens of milliseconds — and it is what lets a test prove
/// the unresolved arm really hashed instead of trusting the source text.
static DUMMY_VERIFIES: AtomicU64 = AtomicU64::new(0);

/// Dummy verifies performed by this process so far.
pub fn dummy_verify_count() -> u64 {
    DUMMY_VERIFIES.load(Ordering::Relaxed)
}

/// Pays one argon2id verify for a caller whose address resolved to nothing.
///
/// Returns nothing ON PURPOSE: there is no verdict to branch on, so a caller
/// cannot accidentally grant a session on a hash it never looked up. The
/// request path calls this immediately before its uniform 401.
pub async fn spend_dummy_verify(password: String) {
    let _ = verify_async(password, DUMMY_PHC.to_owned()).await;
    DUMMY_VERIFIES.fetch_add(1, Ordering::Relaxed);
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

    /// The dummy hash must cost what a real one costs. Asserted on the PHC
    /// parameters rather than on wall clock: if a store's hash and the stand-in
    /// disagree, the unresolved arm is cheap again and the oracle is back.
    #[test]
    fn dummy_phc_carries_the_same_cost_factors_as_a_real_hash() {
        let default = Argon2::default().params().clone();
        let from_dummy =
            argon2::Params::try_from(&PasswordHash::new(DUMMY_PHC).expect("dummy PHC parses"))
                .expect("dummy PHC params");
        let fresh_phc = hash("shape-check-password-1").expect("hash");
        let from_real =
            argon2::Params::try_from(&PasswordHash::new(&fresh_phc).expect("fresh PHC parses"))
                .expect("fresh PHC params");

        for params in [&from_dummy, &from_real] {
            assert_eq!(params.m_cost(), default.m_cost());
            assert_eq!(params.t_cost(), default.t_cost());
            assert_eq!(params.p_cost(), default.p_cost());
        }
        assert_eq!(from_dummy.m_cost(), from_real.m_cost());
        assert_eq!(from_dummy.t_cost(), from_real.t_cost());
        assert_eq!(from_dummy.p_cost(), from_real.p_cost());

        let parsed = PasswordHash::new(DUMMY_PHC).expect("dummy PHC parses");
        assert_eq!(parsed.algorithm.as_str(), "argon2id");
        assert_eq!(
            parsed.version,
            Some(u32::from(argon2::Version::V0x13)),
            "argon2 v19 — the version `hash` writes"
        );
    }

    /// The stand-in is a hash of something nobody can supply, and nothing
    /// reads its verdict anyway — both halves matter, so both are pinned.
    #[tokio::test]
    async fn dummy_verify_costs_one_verify_and_never_answers() {
        let before = dummy_verify_count();
        spend_dummy_verify("correct horse battery staple".into()).await;
        assert_eq!(dummy_verify_count(), before + 1);
        assert!(!verify("anything a caller might send", DUMMY_PHC));
    }
}
