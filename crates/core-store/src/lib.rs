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
//! (core-api) own policy; this crate moves bytes. Downloads ask for a
//! presigned URL first ([`ObjectStore::presign_get`]) and stream through
//! the API when the backend cannot mint one — local-fs always answers
//! `None`, which is why the stream is the fallback rather than a mode a
//! deployment has to enable. Whether an object MAY be handed out that way
//! is policy, and core-api answers it before the URL is asked for.

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
    /// When the store last wrote the object, when the backend can say.
    ///
    /// The integrity sweep needs an age it can trust: an object with no row
    /// pointing at it is only an ORPHAN once the upload that wrote it has had
    /// time to commit its row. Deleting on "not referenced right now" would
    /// race every in-flight upload. `None` means "this backend cannot say",
    /// and the sweep then refuses to treat the object as deletable.
    pub last_modified: Option<std::time::SystemTime>,
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
    /// must stream through the API). The URL carries its own signature for
    /// `ttl` and no Marula credential: anyone holding it can fetch the
    /// object until it expires, which is why the caller mints one only for
    /// an object it has already admitted, and keeps the TTL short.
    async fn presign_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, StoreError>;

    /// Lists every object under `prefix` (an empty prefix lists the whole
    /// store), with the metadata the backend can give WITHOUT one request per
    /// object: key, size and age are always present, and `content_type` may be
    /// a generic placeholder on backends whose listing does not carry it (the
    /// S3 adapter). A caller that needs the real type asks `get`.
    ///
    /// Only the integrity sweep consumes this, and it is the one call that has
    /// to walk the store rather than address one key — so it is deliberately
    /// not on any request path. The default reports `Unavailable` instead of
    /// an empty list: a backend that cannot enumerate must not be mistaken for
    /// a backend with nothing in it, because that reading is what would let a
    /// sweep mark every attachment row as missing.
    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StoreError> {
        Err(StoreError::Unavailable(format!(
            "{} store cannot list objects (prefix {prefix:?})",
            self.name()
        )))
    }
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

/// Builds the quarantine key for bytes an admission gate refused: the same
/// content address under a different namespace. Keeping the digest means a
/// released object moves back to its one true key rather than being
/// re-hashed, and the namespace prefix means a key alone still says which
/// side of the gate the object is on.
#[must_use]
pub fn quarantine_key(org_id: uuid::Uuid, sha256_hex: &str) -> String {
    format!("org/{org_id}/quarantine/{sha256_hex}")
}

/// Builds the executable-artifact key for one plugin module: content-addressed
/// by the module's sha256 inside the org's namespace.
///
/// The digest IS the pin. An install row stores the digest it adopted, so the
/// object it loads is addressed by that digest and nothing else; an update
/// writes a NEW key rather than mutating this one, which is what makes a
/// rollback a pointer move over immutable bytes. Two installs of the same
/// module share one object, and a digest that does not name the bytes is
/// refused before the object is written.
#[must_use]
pub fn plugin_artifact_key(org_id: uuid::Uuid, sha256_hex: &str) -> String {
    format!("org/{org_id}/plugins/{sha256_hex}")
}

/// Builds the key for one org's export bundle: identified by the export id,
/// namespaced per org, and deliberately not content-addressed.
///
/// The bundle is built once per export and its bytes never change, so the id is
/// the address. It lives here beside the other three builders because it is the
/// same kind of fact — where an object in this bucket lives — and because a
/// second copy in the route is how a key format and its inventory entry drift
/// apart.
#[must_use]
pub fn export_bundle_key(org_id: uuid::Uuid, export_id: uuid::Uuid) -> String {
    format!("org/{org_id}/exports/{export_id}/bundle.json")
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

impl StoreChoice {
    /// Resolves the store from the process environment.
    ///
    /// One parser for every process (core-api, worker, the CLI) because the
    /// integrity sweep and the erasure verb must address THE SAME store the API
    /// writes to. Two copies of this resolution, differing in one default, is
    /// how a sweep ends up "finding" an empty store and a purge ends up deleting
    /// nothing while reporting success.
    #[must_use]
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let env = |k: &str| lookup(k).filter(|v| !v.trim().is_empty());
        if env("OBJECT_STORE").as_deref() == Some("s3") {
            Self::S3(S3Config {
                endpoint: env("S3_ENDPOINT"),
                bucket: env("S3_BUCKET").unwrap_or_else(|| "marula".into()),
                region: env("S3_REGION").unwrap_or_else(|| "us-east-1".into()),
                access_key_id: env("S3_ACCESS_KEY_ID").unwrap_or_default(),
                secret_access_key: env("S3_SECRET_ACCESS_KEY").unwrap_or_default(),
            })
        } else {
            Self::Local(env("OBJECT_STORE_LOCAL_DIR").unwrap_or_else(|| "./data/objects".into()))
        }
    }

    /// The same resolution against the real environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }
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

    #[test]
    fn quarantine_key_shares_the_digest_but_not_the_namespace() {
        let org = uuid::Uuid::nil();
        let q = quarantine_key(org, "cafe");
        assert_eq!(
            q,
            "org/00000000-0000-0000-0000-000000000000/quarantine/cafe"
        );
        assert!(validate_key(&q).is_ok());
        // Same digest, different namespace: neither key can be mistaken for
        // the other, and the digest survives a release so the object can be
        // admitted under its canonical key without re-hashing.
        assert_ne!(q, attachment_key(org, "cafe"));
        assert!(q.ends_with("/cafe") && attachment_key(org, "cafe").ends_with("/cafe"));
    }

    #[test]
    fn plugin_artifact_key_is_content_addressed_and_never_collides_with_attachments() {
        let org = uuid::Uuid::nil();
        let k = plugin_artifact_key(org, "cafe");
        assert_eq!(k, "org/00000000-0000-0000-0000-000000000000/plugins/cafe");
        assert!(validate_key(&k).is_ok());
        // A plugin module and an attachment sharing a digest are still two
        // objects: the namespace says which one a key names, and the install
        // row's pin addresses only the plugin one.
        assert_ne!(k, attachment_key(org, "cafe"));
        assert_ne!(k, quarantine_key(org, "cafe"));
    }

    /// One parser for every process: an unset environment must land on the
    /// same local directory in the worker as in core-api, because the sweep and
    /// the API have to be looking at one store.
    #[test]
    fn store_choice_resolves_the_same_way_everywhere() {
        assert_eq!(
            StoreChoice::from_lookup(|_| None),
            StoreChoice::Local("./data/objects".into())
        );
        assert_eq!(
            StoreChoice::from_lookup(|k| (k == "OBJECT_STORE").then(|| "local".to_string())),
            StoreChoice::Local("./data/objects".into())
        );
        let s3 = StoreChoice::from_lookup(|k| match k {
            "OBJECT_STORE" => Some("s3".into()),
            "S3_BUCKET" => Some("attachments".into()),
            "S3_ENDPOINT" => Some("http://garage:3900".into()),
            _ => None,
        });
        match s3 {
            StoreChoice::S3(cfg) => {
                assert_eq!(cfg.bucket, "attachments");
                assert_eq!(cfg.endpoint.as_deref(), Some("http://garage:3900"));
                assert_eq!(cfg.region, "us-east-1");
            }
            other => panic!("expected s3, got {other:?}"),
        }
    }
}
