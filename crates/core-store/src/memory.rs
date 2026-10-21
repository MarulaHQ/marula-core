//! Test doubles backing unit and integration tests (§8): an in-memory
//! store with a full operation log (`MemoryStore`), a recording wrapper
//! around any other implementation (`RecordingStore`) for asserting that
//! callers issue exactly the puts/deletes/presigns they should — e.g. that
//! dedup skips the second PUT — and a presigning wrapper
//! (`PresigningStore`) that answers the one call local-fs cannot make, so a
//! caller's presign-then-stream decision is testable without a bucket.
//!
//! Non-purpose: no persistence across process restarts, no concurrency
//! guarantees beyond a mutex, no production use.

use crate::{validate_key, GetObject, ObjectMeta, ObjectRecordedOp, ObjectStore, StoreError};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Inner {
    objects: HashMap<String, (String, Bytes)>,
    ops: Vec<ObjectRecordedOp>,
}

/// In-memory `ObjectStore` used by tests: real put/get/delete semantics,
/// zero durability, and a recorded operation log.
#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: Arc<Mutex<Inner>>,
    fail_writes: Arc<std::sync::atomic::AtomicBool>,
}

impl MemoryStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// When armed, every `put` fails with [`StoreError::Unavailable`] —
    /// lets pipeline tests exercise fail-closed storage outages.
    pub fn arm_write_failures(&self) {
        self.fail_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Number of successful puts so far (dedup assertions read this).
    #[must_use]
    pub fn put_count(&self) -> usize {
        self.inner
            .lock()
            .map(|g| {
                g.ops
                    .iter()
                    .filter(|op| matches!(op, ObjectRecordedOp::Put { .. }))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Snapshot of the recorded operations in call order.
    #[must_use]
    pub fn ops(&self) -> Vec<ObjectRecordedOp> {
        self.inner.lock().map(|g| g.ops.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl ObjectStore for MemoryStore {
    fn name(&self) -> &'static str {
        "memory"
    }

    async fn put(
        &self,
        key: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<ObjectMeta, StoreError> {
        validate_key(key)?;
        if self.fail_writes.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StoreError::Unavailable("armed failure".into()));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| StoreError::Unavailable("poisoned".into()))?;
        guard.ops.push(ObjectRecordedOp::Put {
            key: key.to_string(),
        });
        let size = body.len() as u64;
        guard
            .objects
            .insert(key.to_string(), (content_type.to_string(), body));
        Ok(ObjectMeta {
            key: key.to_string(),
            size_bytes: size,
            content_type: content_type.to_string(),
            last_modified: std::time::SystemTime::now().into(),
        })
    }

    async fn get(&self, key: &str) -> Result<GetObject, StoreError> {
        validate_key(key)?;
        let guard = self
            .inner
            .lock()
            .map_err(|_| StoreError::Unavailable("poisoned".into()))?;
        let (content_type, body) = guard
            .objects
            .get(key)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(key.to_string()))?;
        let size = body.len() as u64;
        Ok(GetObject {
            size_bytes: size,
            content_type,
            stream: Box::pin(stream::once(async move { Ok(body) })),
        })
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        validate_key(key)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| StoreError::Unavailable("poisoned".into()))?;
        guard.ops.push(ObjectRecordedOp::Delete {
            key: key.to_string(),
        });
        guard.objects.remove(key);
        Ok(())
    }

    async fn presign_get(&self, _key: &str, _ttl: Duration) -> Result<Option<String>, StoreError> {
        Ok(None)
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StoreError> {
        if !prefix.is_empty() {
            validate_key(prefix.trim_end_matches('/'))?;
        }
        let guard = self
            .inner
            .lock()
            .map_err(|_| StoreError::Unavailable("poisoned".into()))?;
        let mut out: Vec<ObjectMeta> = guard
            .objects
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, (content_type, body))| ObjectMeta {
                key: key.clone(),
                size_bytes: body.len() as u64,
                content_type: content_type.clone(),
                last_modified: None,
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }
}

/// Wrapper recording every forwarded call — wraps ANY implementation so
/// tests can assert interaction patterns against production adapters too.
pub struct RecordingStore<T>(pub T);

#[async_trait]
impl<T: ObjectStore> ObjectStore for RecordingStore<T> {
    fn name(&self) -> &'static str {
        "recording"
    }

    async fn put(
        &self,
        key: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<ObjectMeta, StoreError> {
        self.0.put(key, content_type, body).await
    }

    async fn get(&self, key: &str) -> Result<GetObject, StoreError> {
        self.0.get(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.0.delete(key).await
    }

    async fn presign_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, StoreError> {
        self.0.presign_get(key, ttl).await
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StoreError> {
        self.0.list(prefix).await
    }
}

/// Wrapper that behaves like an S3-backed store for the one call local-fs
/// cannot make: `presign_get` answers with a deterministic fake URL, so the
/// presign-then-stream decision is exercised without a bucket. `MemoryStore`
/// is `Clone` over a shared inner, so wrapping a clone forwards the SAME
/// bytes the unwrapped store holds.
pub struct PresigningStore<T>(pub T);

impl<T> PresigningStore<T> {
    /// The URL this wrapper would mint for `key` — tests assert against it
    /// rather than re-implementing the format.
    #[must_use]
    pub fn url_for(key: &str, ttl: Duration) -> String {
        format!(
            "https://objects.test/{key}?X-Signature=test&X-Expires={}",
            ttl.as_secs()
        )
    }
}

#[async_trait]
impl<T: ObjectStore> ObjectStore for PresigningStore<T> {
    fn name(&self) -> &'static str {
        "presigning"
    }

    async fn put(
        &self,
        key: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<ObjectMeta, StoreError> {
        self.0.put(key, content_type, body).await
    }

    async fn get(&self, key: &str) -> Result<GetObject, StoreError> {
        self.0.get(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.0.delete(key).await
    }

    async fn presign_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, StoreError> {
        // The object must exist: a URL for absent bytes is a 404 at the
        // bucket, which is a different failure from "this backend cannot
        // presign" and should not be faked.
        self.0.get(key).await?;
        Ok(Some(Self::url_for(key, ttl)))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StoreError> {
        self.0.list(prefix).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_and_not_found() {
        let st = MemoryStore::new();
        st.put("k", "text/plain", Bytes::from_static(b"abc"))
            .await
            .expect("put");
        let obj = st.get("k").await.expect("get");
        assert_eq!(obj.size_bytes, 3);
        assert_eq!(obj.content_type, "text/plain");
        assert!(matches!(
            st.get("missing").await.expect_err("nf"),
            StoreError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn armed_failure_fails_closed() {
        let st = MemoryStore::new();
        st.arm_write_failures();
        assert!(matches!(
            st.put("k", "a/b", Bytes::from_static(b"z"))
                .await
                .expect_err("fail"),
            StoreError::Unavailable(_)
        ));
        assert_eq!(st.put_count(), 0);
    }
}
