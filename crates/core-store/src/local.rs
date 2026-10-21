//! `LocalFsStore` — the filesystem "none" mode (§2 Storage row): a
//! self-hosted Marula runs with NO object store service, persisting
//! attachments under a configured directory.
//!
//! Purpose: durable-enough single-node storage with the exact trait
//! semantics of the S3 adapter — atomic writes (temp file + rename),
//! content-type carried in a `.meta` JSON sidecar next to each object,
//! streamed reads, idempotent delete.
//!
//! Non-purpose: no multi-node consistency, no replication/backup, no
//! locking — deployments needing those run the S3/MinIO adapter instead.

use crate::{validate_key, GetObject, ObjectMeta, ObjectStore, StoreError};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use std::path::{Path, PathBuf};

/// Filesystem-backed adapter rooted at one directory. The root is created
/// lazily on first write; reads before any write surface as `NotFound`.
pub struct LocalFsStore {
    root: PathBuf,
}

impl LocalFsStore {
    /// Creates an adapter rooted at `root` (relative paths resolve against
    /// the process working directory).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn data_path(&self, key: &str) -> Result<PathBuf, StoreError> {
        validate_key(key)?;
        Ok(self.root.join(key))
    }

    /// Sidecar path built by SUFFIXING (never by replacing extensions, so
    /// dotted keys cannot collide with sibling objects).
    fn suffixed(path: &Path, suffix: &str) -> PathBuf {
        let mut s = path.as_os_str().to_os_string();
        s.push(suffix);
        PathBuf::from(s)
    }

    fn meta_path(&self, key: &str) -> Result<PathBuf, StoreError> {
        Ok(Self::suffixed(&self.data_path(key)?, ".meta.json"))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SidecarMeta {
    size_bytes: u64,
    content_type: String,
}

fn unavailable(e: std::io::Error) -> StoreError {
    if e.kind() == std::io::ErrorKind::NotFound {
        // Distinguish "no such object" from a broken store by message
        // shape only at mapping time; callers treat both as terminal.
        StoreError::Io(e.to_string())
    } else {
        StoreError::Unavailable(format!("local fs: {e}"))
    }
}

#[async_trait]
impl ObjectStore for LocalFsStore {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn put(
        &self,
        key: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<ObjectMeta, StoreError> {
        let data = self.data_path(key)?;
        let meta = self.meta_path(key)?;
        if let Some(dir) = data.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| StoreError::Unavailable(format!("local fs mkdir: {e}")))?;
        }
        // Atomic publish: write uniquely-named siblings, then rename both
        // into place so a crash never leaves a half-written object visible
        // under its key; unique tmp names keep concurrent puts of one key
        // from clobbering each other's staging files.
        let nonce = uuid::Uuid::now_v7().simple().to_string();
        let tmp_data = Self::suffixed(&data, &format!(".tmp-{nonce}"));
        let tmp_meta = Self::suffixed(&meta, &format!(".tmp-{nonce}"));
        tokio::fs::write(&tmp_data, &body)
            .await
            .map_err(unavailable)?;
        let sidecar = SidecarMeta {
            size_bytes: body.len() as u64,
            content_type: content_type.to_string(),
        };
        let json = serde_json::to_vec(&sidecar)
            .map_err(|e| StoreError::Io(format!("sidecar encode: {e}")))?;
        tokio::fs::write(&tmp_meta, json)
            .await
            .map_err(unavailable)?;
        tokio::fs::rename(&tmp_meta, &meta)
            .await
            .map_err(unavailable)?;
        tokio::fs::rename(&tmp_data, &data)
            .await
            .map_err(unavailable)?;
        Ok(ObjectMeta {
            key: key.to_string(),
            size_bytes: sidecar.size_bytes,
            content_type: sidecar.content_type,
            last_modified: std::time::SystemTime::now().into(),
        })
    }

    async fn get(&self, key: &str) -> Result<GetObject, StoreError> {
        let data = self.data_path(key)?;
        let meta_path = self.meta_path(key)?;
        if !tokio::fs::try_exists(&data)
            .await
            .map_err(|e| StoreError::Unavailable(format!("local fs stat: {e}")))?
        {
            return Err(StoreError::NotFound(key.to_string()));
        }
        let (size_bytes, content_type) = match tokio::fs::read(&meta_path).await {
            Ok(raw) => {
                let sidecar: SidecarMeta = serde_json::from_slice(&raw)
                    .map_err(|e| StoreError::Io(format!("sidecar decode for {key}: {e}")))?;
                (sidecar.size_bytes, sidecar.content_type)
            }
            Err(_) => {
                // Missing/corrupt sidecar: serve the bytes rather than
                // fail; length is derived from the file itself.
                let len = tokio::fs::metadata(&data)
                    .await
                    .map_err(|_| StoreError::NotFound(key.to_string()))?
                    .len();
                (len, "application/octet-stream".to_string())
            }
        };
        let file = tokio::fs::File::open(&data)
            .await
            .map_err(|_| StoreError::NotFound(key.to_string()))?;
        let stream = tokio_util::io::ReaderStream::new(file)
            .map(|chunk| chunk.map_err(|e| StoreError::Io(format!("local read: {e}"))));
        Ok(GetObject {
            size_bytes,
            content_type,
            stream: Box::pin(stream),
        })
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        let data = self.data_path(key)?;
        let meta = self.meta_path(key)?;
        // Idempotent by contract: absent files are success, other IO
        // failures are real errors.
        match tokio::fs::remove_file(&data).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StoreError::Unavailable(format!("local fs rm: {e}"))),
        }
        if let Err(e) = tokio::fs::remove_file(&meta).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(key, error = %e, "sidecar removal failed");
            }
        }
        prune_empty_dirs(&self.root, &data).await;
        Ok(())
    }

    async fn presign_get(
        &self,
        _key: &str,
        _ttl: std::time::Duration,
    ) -> Result<Option<String>, StoreError> {
        // A filesystem cannot hand out URLs; callers must stream.
        Ok(None)
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StoreError> {
        // A prefix is a key fragment, not an absolute path: validate it with
        // the same rule every other call uses (an empty prefix means "all").
        if !prefix.is_empty() {
            validate_key(prefix.trim_end_matches('/'))?;
        }
        let start = if prefix.is_empty() {
            self.root.clone()
        } else {
            self.root.join(prefix.trim_end_matches('/'))
        };
        let mut out = Vec::new();
        let mut stack = vec![start];
        while let Some(dir) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(entries) => entries,
                // A prefix that names no directory is an empty listing, not
                // a failure: the sweep asks per org prefix and most stores
                // have no such tree.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(StoreError::Unavailable(format!("local fs list: {e}")));
                }
            };
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| StoreError::Unavailable(format!("local fs list: {e}")))?
            {
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|e| StoreError::Unavailable(format!("local fs list: {e}")))?;
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // Staging files and metadata sidecars are not objects. The
                // sidecar is suffixed, so this is how they are told apart.
                if name.ends_with(".meta.json") || name.contains(".tmp-") {
                    continue;
                }
                let key = match path.strip_prefix(&self.root) {
                    Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                if validate_key(&key).is_err() {
                    continue;
                }
                // Content type and size come from the sidecar when it is
                // readable; the sweep only really needs size and age.
                let sidecar = match tokio::fs::read(Self::suffixed(&path, ".meta.json")).await {
                    Ok(raw) => serde_json::from_slice::<SidecarMeta>(&raw).ok(),
                    Err(_) => None,
                };
                let size_bytes = match &sidecar {
                    Some(meta) => meta.size_bytes,
                    None => tokio::fs::metadata(&path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0),
                };
                let last_modified = tokio::fs::metadata(&path)
                    .await
                    .ok()
                    .and_then(|m| m.modified().ok());
                out.push(ObjectMeta {
                    key,
                    size_bytes,
                    content_type: sidecar
                        .map(|s| s.content_type)
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                    last_modified,
                });
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }
}

/// Removes now-empty directories between the deleted object and the root,
/// keeping long-lived trees tidy. Best-effort: failures are ignored.
async fn prune_empty_dirs(root: &Path, mut dir: &Path) {
    while dir.starts_with(root) && dir != root {
        match tokio::fs::read_dir(dir).await {
            Ok(mut entries) => {
                if entries.next_entry().await.ok().flatten().is_some() {
                    return;
                }
                if tokio::fs::remove_dir(dir).await.is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    async fn read_all(obj: GetObject) -> Vec<u8> {
        let mut obj = obj;
        let mut out = Vec::new();
        while let Some(chunk) = futures_util::StreamExt::next(&mut obj.stream).await {
            out.extend_from_slice(&chunk.expect("chunk"));
        }
        out
    }

    fn store(dir: &Path) -> LocalFsStore {
        LocalFsStore::new(dir)
    }

    #[tokio::test]
    async fn put_get_roundtrip_preserves_content_type_and_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let st = store(dir.path());
        let meta = st
            .put(
                "org/x/attachments/a",
                "text/plain",
                Bytes::from_static(b"hello"),
            )
            .await
            .expect("put");
        assert_eq!(meta.size_bytes, 5);
        assert_eq!(meta.content_type, "text/plain");
        let obj = st.get("org/x/attachments/a").await.expect("get");
        assert_eq!(obj.size_bytes, 5);
        let buf = read_all(obj).await;
        assert_eq!(buf, b"hello");
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let st = store(dir.path());
        let err = st.get("org/x/none").await.expect_err("missing");
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_is_idempotent_and_removes_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let st = store(dir.path());
        st.put("k/one", "a/b", Bytes::from_static(b"z"))
            .await
            .expect("put");
        st.delete("k/one").await.expect("delete");
        st.delete("k/one").await.expect("delete again");
        assert!(st.get("k/one").await.is_err());
    }

    /// The integrity sweep walks the store; a listing that returned sidecars or
    /// staging files would report objects that no row can ever reference, and
    /// the sweep would delete them by definition of "orphan".
    #[tokio::test]
    async fn list_returns_objects_only_under_the_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let st = store(dir.path());
        for key in [
            "org/a/attachments/one",
            "org/a/quarantine/two",
            "org/b/attachments/three",
        ] {
            st.put(key, "text/plain", Bytes::from_static(b"xy"))
                .await
                .expect("put");
        }
        let all = st.list("").await.expect("list all");
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(all.iter().all(|m| m.last_modified.is_some()));
        let org_a = st.list("org/a").await.expect("list org a");
        assert_eq!(
            org_a.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            vec!["org/a/attachments/one", "org/a/quarantine/two"]
        );
        assert!(st
            .list("org/absent")
            .await
            .expect("absent prefix")
            .is_empty());
    }
    #[tokio::test]
    async fn traversal_keys_rejected_before_io() {
        let dir = tempfile::tempdir().expect("tempdir");
        let st = store(dir.path());
        let err = st
            .put("../escape", "x/y", Bytes::from_static(b"z"))
            .await
            .expect_err("traversal");
        assert!(matches!(err, StoreError::InvalidKey(_)));
    }

    /// Shared current-thread runtime so sync proptest cases can await.
    fn rt() -> &'static tokio::runtime::Runtime {
        static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
        })
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(16))]

        /// Property: put→get→delete is an EXACT roundtrip for arbitrary
        /// payloads and nested keys — byte equality, size, content type,
        /// then NotFound after delete.
        #[test]
        fn prop_roundtrip_arbitrary_bytes_and_keys(
            bytes in proptest::collection::vec(any::<u8>(), 0..(1 << 16)),
            key in r"[a-z0-9][a-z0-9._-]{0,7}(/[a-z0-9][a-z0-9._-]{0,11}){1,4}",
        ) {
            rt().block_on(async move {
                let dir = tempfile::tempdir().map_err(|e| {
                    proptest::test_runner::TestCaseError::fail(e.to_string())
                })?;
                let st = store(dir.path());
                let body = Bytes::from(bytes);
                let ct = "application/x-marula-test";
                let meta = st.put(&key, ct, body.clone()).await
                    .map_err(fail)?;
                proptest::prop_assert_eq!(meta.size_bytes, body.len() as u64);
                let mut obj = st.get(&key).await.map_err(fail)?;
                proptest::prop_assert_eq!(&obj.content_type, ct);
                proptest::prop_assert_eq!(obj.size_bytes, body.len() as u64);
                let mut got = Vec::with_capacity(body.len());
                while let Some(chunk) =
                    futures_util::StreamExt::next(&mut obj.stream).await
                {
                    got.extend_from_slice(&chunk.map_err(fail)?);
                }
                proptest::prop_assert_eq!(got, body.to_vec());
                st.delete(&key).await.map_err(fail)?;
                let after = st.get(&key).await;
                proptest::prop_assert!(matches!(after, Err(StoreError::NotFound(_))));
                Ok(())
            })?;
        }

        /// Property: keys outside the validated alphabet are ALWAYS
        /// rejected with `InvalidKey` before any filesystem IO.
        #[test]
        fn prop_invalid_keys_rejected_before_io(
            raw in proptest::collection::vec(
                proptest::sample::select(vec!['a', '/', '.', '\\', ' ', 'é']),
                0..32,
            )
                .prop_map(|chars| chars.into_iter().collect::<String>()),
        ) {
            rt().block_on(async move {
                if crate::validate_key(&raw).is_err() {
                    let res = st_for_prop(&raw).await;
                    proptest::prop_assert!(matches!(
                        res,
                        Err(StoreError::InvalidKey(_))
                    ));
                }
                Ok(())
            })?;
        }
    }

    fn fail(e: impl std::fmt::Display) -> proptest::test_runner::TestCaseError {
        proptest::test_runner::TestCaseError::fail(e.to_string())
    }

    /// One-shot put used by the rejection property: fresh tempdir per call
    /// keeps cases independent without leaking state across the run.
    async fn st_for_prop(key: &str) -> Result<ObjectMeta, StoreError> {
        let dir = tempfile::tempdir().expect("tempdir");
        store(dir.path())
            .put(key, "a/b", Bytes::from_static(b"x"))
            .await
    }
}
