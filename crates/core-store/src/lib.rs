//! core-store — the `ObjectStore` edge trait (§4.7 traits-at-the-edge) and
//! its adapters: every byte Marula persists outside Postgres crosses this
//! one boundary, so no vendor SDK or filesystem detail leaks past it.
//!
//! Purpose:
//! - the [`ObjectStore`] trait (put / get-stream / delete / presign-or-
//!   stream) with content-type and size carried as first-class metadata;
//! - adapters: [`local::LocalFsStore`] (the documented filesystem "none"
//!   mode so a self-host runs with no object store at all), [`s3::S3Store`]
//!   (S3/MinIO/Cloudflare-R2 via the vetted `object_store` crate), and
//!   [`memory::MemoryStore`] + [`memory::RecordingStore`] test doubles;
//! - key validation shared by every adapter (no traversal, no surprises).
//!
//! Non-purpose: no upload authorization, no quota accounting, no virus
//! scanning, no multipart/resumable uploads, no bucket lifecycle — callers
//! (core-api) own policy; this crate moves bytes. Downloads stream through
//! the API in v1; presigned URLs are offered where the backend supports
//! them but nothing in Marula hands them to browsers yet.

pub mod local;
pub mod memory;
pub mod s3;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Metadata describing one stored object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    /// Store-validated key (the same string passed to `put`).
    pub key: String,
    /// Exact byte length of the stored payload.
    pub size_bytes: u64,
    /// Content type recorded with the object.
    pub content_type: String,
}

/// A streamed object read: metadata plus an open body stream.
pub struct GetObject {
    /// Byte length reported by the store.
    pub size_bytes: u64,
    /// Content type recorded at put time.
    pub content_type: String,
    /// The body, chunked; consumers must stream, never buffer blindly.
    pub stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, StoreError>> + Send>>,
}

impl fmt::Debug for GetObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GetObject")
            .field("size_bytes", &self.size_bytes)
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}

/// Errors crossing the store boundary. `Unavailable` is the fail-closed
/// variant: callers must treat unreachable storage like any other outage,
/// never silently succeed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("invalid object key: {0}")]
    InvalidKey(String),
    #[error("store rejected request: {0}")]
    Rejected(String),
    #[error("object store unavailable: {0}")]
    Unavailable(String),
    #[error("io error: {0}")]
    Io(String),
}

/// The single seam for durable binary objects (§2 Storage row).
///
/// Implementations: [`local::LocalFsStore`] (dev/self-host none mode),
/// [`s3::S3Store`] (S3-compatible: MinIO, AWS S3, R2), [`memory`] doubles
/// for tests. All methods take validated keys; see [`validate_key`].
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Adapter name for logs/ledgers (`local`, `s3`, `memory`, `noop`).
    fn name(&self) -> &'static str;

    /// Stores one object atomically; concurrent puts of the same key are
    /// last-writer-wins per adapter semantics (Marula keys are
    /// content-addressed, so writers of one key write identical bytes).
    async fn put(
        &self,
        key: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<ObjectMeta, StoreError>;

    /// Opens one object for streaming read.
    async fn get(&self, key: &str) -> Result<GetObject, StoreError>;

    /// Deletes one object. Idempotent: deleting a missing key succeeds
    /// (callers race dedup/refcounting and must not depend on order).
    async fn delete(&self, key: &str) -> Result<(), StoreError>;

    /// Presigned GET URL when the backend supports it (`None` ⇒ caller
    /// must stream through the API). Marula v1 never issues public
    /// URLs; the hook exists so later stages can opt in per deployment.
    async fn presign_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, StoreError>;
}

/// Validates a storage key for every adapter: non-empty, printable ASCII
/// subset `[A-Za-z0-9/._-]`, no leading/trailing slash, no empty or `.`
/// / `..` segments — path traversal is structurally impossible.
///
/// # Errors
/// [`StoreError::InvalidKey`] on any violation.
pub fn validate_key(key: &str) -> Result<(), StoreError> {
    let ok = !key.is_empty()
        && !key.starts_with('/')
        && !key.ends_with('/')
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
        && key
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..");
    if ok {
        Ok(())
    } else {
        Err(StoreError::InvalidKey(key.to_string()))
    }
}

/// Builds the canonical attachment storage key: content-addressed by
/// sha256 within the org's namespace, which makes dedup a pure lookup and
/// keeps keys opaque to clients.
#[must_use]
pub fn attachment_key(org_id: uuid::Uuid, sha256_hex: &str) -> String {
    format!("org/{org_id}/attachments/{sha256_hex}")
}

/// Which backend a process should use. Parsed from config; unknown values
/// degrade to the documented none mode rather than failing boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreChoice {
    /// Filesystem directory — the self-host default.
    Local(String),
    /// Any S3-compatible endpoint (MinIO, AWS S3, R2).
    S3(S3Config),
}

/// Connection settings for S3-compatible stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Endpoint base URL (`http://minio:9000`, R2 account URL, …). Empty
    /// means real AWS S3 default endpoints.
    pub endpoint: Option<String>,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

/// One recorded store operation — the log entry type behind the test
/// doubles' interaction assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectRecordedOp {
    Put { key: String },
    Delete { key: String },
}

/// Constructs the configured adapter. Construction is infallible: a bad
/// S3 config surfaces as `Unavailable` on first use, mirroring how
/// MailStack degrades to none modes instead of refusing to boot.
#[must_use]
pub fn build(choice: &StoreChoice) -> Arc<dyn ObjectStore> {
    match choice {
        StoreChoice::Local(root) => Arc::new(local::LocalFsStore::new(root)),
        StoreChoice::S3(cfg) => Arc::new(s3::S3Store::new(cfg.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_bad_segments() {
        for bad in [
            "", "/lead", "trail/", "a/../b", "./a", "a/./b", "a//b", "a b", "a\\b", "é", "a?",
        ] {
            assert!(validate_key(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn accepts_canonical_keys() {
        assert!(validate_key("org/0192ff/attachments/ab12").is_ok());
        assert!(validate_key("a.b_c-d/e").is_ok());
    }

    #[test]
    fn attachment_key_is_content_addressed_per_org() {
        let k = attachment_key(uuid::Uuid::nil(), "cafe");
        assert_eq!(
            k,
            "org/00000000-0000-0000-0000-000000000000/attachments/cafe"
        );
        assert!(validate_key(&k).is_ok());
    }
}
