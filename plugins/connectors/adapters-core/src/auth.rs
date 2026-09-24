//! Vendor authentication flows. Credential material arrives just-in-time
//! from the operator's secret store and is never persisted or logged.

use std::time::{Duration, Instant};

use serde::Deserialize;

/// The auth flows the supported vendors use.
///
/// `Debug` redacts every secret — the derived impl would print key
/// material, so it is written by hand.
#[derive(Clone)]
pub enum VendorAuth {
    /// A static key under a named request header (IT Glue: `x-api-key`).
    ApiKeyHeader { header: String, key: String },
    /// A bearer token the operator supplies (ImmyBot, Hudu).
    Bearer(String),
    /// OAuth2 client-credentials grant (M365 Graph). The token is
    /// fetched once and cached in-process; it refreshes 60 s ahead of
    /// expiry so a long pull never rides an expiring token.
    ClientCredentials(ClientCredentials),
}

/// Client-credentials grant parameters.
#[derive(Clone)]
pub struct ClientCredentials {
    /// The token endpoint (M365: `https://login.microsoftonline.com/
    /// {tenant}/oauth2/v2.0/token`).
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Space-separated scope string (M365 Graph: `https://graph.microsoft.com/.default`).
    pub scope: String,
}

impl std::fmt::Debug for VendorAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VendorAuth::ApiKeyHeader { header, .. } => f
                .debug_struct("ApiKeyHeader")
                .field("header", header)
                .field("key", &"[REDACTED]")
                .finish(),
            VendorAuth::Bearer(_) => f.debug_tuple("Bearer").field(&"[REDACTED]").finish(),
            VendorAuth::ClientCredentials(c) => f
                .debug_struct("ClientCredentials")
                .field("token_url", &c.token_url)
                .field("client_id", &c.client_id)
                .field("client_secret", &"[REDACTED]")
                .field("scope", &c.scope)
                .finish(),
        }
    }
}

impl VendorAuth {
    /// Applies this auth to an outgoing request builder.
    pub async fn apply(
        &self,
        request: reqwest::RequestBuilder,
        http: &reqwest::Client,
    ) -> Result<reqwest::RequestBuilder, crate::client::VendorError> {
        match self {
            VendorAuth::ApiKeyHeader { header, key } => {
                Ok(request.header(header.as_str(), key.as_str()))
            }
            VendorAuth::Bearer(token) => Ok(request.bearer_auth(token)),
            VendorAuth::ClientCredentials(creds) => {
                let token = TokenCache::global().token(creds, http).await?;
                Ok(request.bearer_auth(token))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireToken {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Process-wide token cache for client-credentials grants. One entry:
/// connector pulls run per-org at most, and every org shares one tenant
/// token — a single cached grant keeps the pull to one token request.
#[derive(Default)]
struct TokenCache {
    inner: tokio::sync::Mutex<Vec<CachedToken>>,
}

struct CachedToken {
    /// Cache key: token endpoint + client id (distinct grants never
    /// share a token, and a rotated secret re-fetches — the secret
    /// itself is never part of the key).
    key: String,
    token: String,
    expires_at: Instant,
}

impl TokenCache {
    fn global() -> &'static TokenCache {
        use std::sync::OnceLock;
        static CACHE: OnceLock<TokenCache> = OnceLock::new();
        CACHE.get_or_init(TokenCache::default)
    }

    async fn token(
        &self,
        creds: &ClientCredentials,
        http: &reqwest::Client,
    ) -> Result<String, crate::client::VendorError> {
        let key = format!("{}\u{0}{}", creds.token_url, creds.client_id);
        let mut guard = self.inner.lock().await;
        if let Some(cached) = guard.iter().find(|c| c.key == key) {
            if cached.expires_at > Instant::now() {
                return Ok(cached.token.clone());
            }
        }
        guard.retain(|c| c.key != key);
        let response = http
            .post(&creds.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", creds.client_id.as_str()),
                ("client_secret", creds.client_secret.as_str()),
                ("scope", creds.scope.as_str()),
            ])
            .send()
            .await
            .map_err(|e| crate::client::VendorError::Network(format!("token request: {e}")))?;
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
        if status.is_success() {
            let parsed: WireToken = serde_json::from_value(body).map_err(|e| {
                crate::client::VendorError::Rejected(format!("token response unreadable: {e}"))
            })?;
            // Refresh a minute early: a pull started near expiry must
            // not fail mid-flight on a stale bearer.
            let ttl =
                Duration::from_secs(parsed.expires_in.unwrap_or(3600).saturating_sub(60).max(60));
            guard.push(CachedToken {
                key,
                token: parsed.access_token.clone(),
                expires_at: Instant::now() + ttl,
            });
            Ok(parsed.access_token)
        } else if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            Err(crate::client::VendorError::Unauthorized(format!(
                "token endpoint refused the grant ({status})"
            )))
        } else {
            Err(crate::client::VendorError::Rejected(format!(
                "token endpoint returned {status}"
            )))
        }
    }
}
