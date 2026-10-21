//! `S3Store` — adapter for any S3-compatible endpoint: MinIO in
//! self-host compose, AWS S3, Cloudflare R2 (§2 Storage row).
//!
//! Crate choice (documented decision): the Apache-Arrow-maintained
//! `object_store` crate drives the wire protocol instead of a hand-rolled
//! SigV4 signer or the very heavy `aws-sdk-s3`. It is purpose-built,
//! streams bodies, supports endpoint override (MinIO/R2), and keeps its
//! types behind OUR [`ObjectStore`] trait — no vendor type leaks past the
//! boundary (§4.7).
//!
//! Non-purpose: no bucket creation, lifecycle rules, or multipart upload
//! sessions; objects are content-addressed and bounded by the API's size
//! cap, so single-shot PUTs cover every Marula write path.

use crate::{validate_key, GetObject, ObjectMeta, ObjectStore, S3Config, StoreError};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use object_store::{
    signer::Signer, Attribute, Attributes, ObjectStore as OsObjectStore, PutOptions, PutPayload,
};
use std::time::Duration;

/// S3-compatible adapter wrapping one bucket.
pub struct S3Store {
    inner: object_store::aws::AmazonS3,
    /// False when the endpoint is not a valid http(s) URL — every call
    /// then reports `Unavailable` WITHOUT touching the client (the
    /// underlying crate panics on malformed URIs, so we gate first).
    usable: bool,
}

impl S3Store {
    /// Builds from connection settings. Infallible by contract: a bad
    /// endpoint makes the instance permanently `Unavailable` (fail-closed,
    /// logged loudly at boot) rather than blocking startup or panicking.
    #[must_use]
    pub fn new(config: S3Config) -> Self {
        let allow_http = config
            .endpoint
            .as_deref()
            .is_some_and(|e| e.starts_with("http://"));
        let usable = match &config.endpoint {
            None => true,
            Some(endpoint) => match reqwest::Url::parse(endpoint) {
                Ok(url) if matches!(url.scheme(), "http" | "https") && !url.cannot_be_a_base() => {
                    allow_http || url.scheme() == "https"
                }
                _ => false,
            },
        };
        let mut builder = object_store::aws::AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_region(&config.region)
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key)
            .with_allow_http(allow_http);
        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        match builder.build() {
            Ok(inner) => Self { inner, usable },
            Err(e) => {
                // Deferred-failure contract: log loudly now; first use
                // reports Unavailable against a placeholder client.
                tracing::error!(error = %e, "s3 store misconfigured");
                Self {
                    inner: placeholder_client(),
                    usable: false,
                }
            }
        }
    }

    fn guard(&self) -> Result<(), StoreError> {
        if self.usable {
            Ok(())
        } else {
            Err(StoreError::Unavailable(
                "s3 store is misconfigured (invalid endpoint or credentials)".into(),
            ))
        }
    }

    fn path(&self, key: &str) -> Result<object_store::path::Path, StoreError> {
        validate_key(key)?;
        object_store::path::Path::parse(key)
            .map_err(|e| StoreError::InvalidKey(format!("{key}: {e}")))
    }
}

/// A syntactically-valid-but-unroutable client for the misconfigured
/// placeholder path (never dialed: `guard()` short-circuits first).
fn placeholder_client() -> object_store::aws::AmazonS3 {
    object_store::aws::AmazonS3Builder::new()
        .with_bucket_name("marula-invalid-bucket")
        .with_endpoint("http://127.0.0.1:1")
        .with_access_key_id("invalid")
        .with_secret_access_key("invalid")
        .build()
        .unwrap_or_else(|_| unreachable!("placeholder builder is static"))
}

fn map_err(key: &str, e: object_store::Error) -> StoreError {
    match e {
        object_store::Error::NotFound { .. } => StoreError::NotFound(key.to_string()),
        other => StoreError::Unavailable(format!("s3: {other}")),
    }
}

#[async_trait]
impl ObjectStore for S3Store {
    fn name(&self) -> &'static str {
        "s3"
    }

    async fn put(
        &self,
        key: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<ObjectMeta, StoreError> {
        self.guard()?;
        let path = self.path(key)?;
        let size_bytes = body.len() as u64;
        let opts = PutOptions {
            attributes: Attributes::from_iter([(Attribute::ContentType, content_type.to_string())]),
            ..PutOptions::default()
        };
        self.inner
            .put_opts(&path, PutPayload::from_bytes(body), opts)
            .await
            .map_err(|e| map_err(key, e))?;
        Ok(ObjectMeta {
            key: key.to_string(),
            size_bytes,
            content_type: content_type.to_string(),
            last_modified: std::time::SystemTime::now().into(),
        })
    }

    async fn get(&self, key: &str) -> Result<GetObject, StoreError> {
        self.guard()?;
        let path = self.path(key)?;
        let key = key.to_string();
        let result = self.inner.get(&path).await.map_err(|e| map_err(&key, e))?;
        let size_bytes = result.meta.size;
        let content_type = result
            .attributes
            .get(&Attribute::ContentType)
            .map(|v| v.as_ref().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let stream = result
            .into_stream()
            .map(move |chunk| chunk.map_err(|e| map_err(&key, e)));
        Ok(GetObject {
            size_bytes,
            content_type,
            stream: Box::pin(stream),
        })
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.guard()?;
        let path = self.path(key)?;
        match self.inner.delete(&path).await {
            Ok(()) => Ok(()),
            // Idempotent by trait contract.
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(map_err(key, e)),
        }
    }

    async fn presign_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, StoreError> {
        self.guard()?;
        let path = self.path(key)?;
        let url = self
            .inner
            .signed_url(reqwest::Method::GET, &path, ttl)
            .await
            .map_err(|e| map_err(key, e))?;
        Ok(Some(url.to_string()))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StoreError> {
        self.guard()?;
        let prefix = prefix.trim_end_matches('/');
        let root = if prefix.is_empty() {
            None
        } else {
            Some(
                object_store::path::Path::parse(prefix)
                    .map_err(|e| StoreError::InvalidKey(format!("{prefix}: {e}")))?,
            )
        };
        let mut stream = self.inner.list(root.as_ref());
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let meta = item.map_err(|e| map_err(prefix, e))?;
            out.push(ObjectMeta {
                key: meta.location.to_string(),
                size_bytes: meta.size,
                // Listings do not carry user metadata, so the content type is
                // not known here without one HEAD per object. The integrity
                // sweep — the only consumer — needs the key, the size and the
                // age; a caller that needs the real type asks `get`.
                content_type: "application/octet-stream".to_string(),
                // The S3 listing timestamp is whole seconds since the epoch;
                // a pre-1970 value cannot be represented, and no object Marula
                // wrote has one.
                last_modified: u64::try_from(meta.last_modified.timestamp())
                    .ok()
                    .map(|secs| std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            });
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Misconfiguration must NOT crash boot: the deferred-failure contract
    /// surfaces Unavailable on first use instead.
    #[tokio::test]
    async fn bad_endpoint_fails_unavailable_not_panic() {
        let st = S3Store::new(S3Config {
            endpoint: Some("not a url".into()),
            bucket: "b".into(),
            region: "us-east-1".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
        });
        let err = st
            .get("org/x/attachments/abc")
            .await
            .expect_err("deferred failure");
        assert!(matches!(err, StoreError::Unavailable(_)));
    }

    /// Live smoke against a real MinIO (`--ignored`; see crate README).
    #[cfg(feature = "live-s3")]
    #[tokio::test]
    #[ignore = "requires a running MinIO; env-driven, run manually"]
    async fn s3_live_roundtrip_against_minio() {
        let cfg = S3Config {
            endpoint: std::env::var("S3_ENDPOINT").ok(),
            bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "marula".into()),
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key_id: std::env::var("S3_ACCESS_KEY_ID").expect("access key"),
            secret_access_key: std::env::var("S3_SECRET_ACCESS_KEY").expect("secret"),
        };
        let st = S3Store::new(cfg);
        let key = format!("smoke/{}", uuid::Uuid::now_v7());
        st.put(&key, "text/plain", Bytes::from_static(b"marula-live"))
            .await
            .expect("put");
        let obj = st.get(&key).await.expect("get");
        assert_eq!(obj.size_bytes, 13);
        st.delete(&key).await.expect("delete");
        assert!(matches!(
            st.get(&key).await.expect_err("gone"),
            StoreError::NotFound(_)
        ));
    }
}
