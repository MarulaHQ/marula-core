//! The parent's `http_request` transport (Wave 53 lane 12, phase 2): screen,
//! resolve, decrypt, inject, send, bound.
//!
//! ## Why this lives here and not in `plugin-host`
//!
//! `plugin-host` is linked into the worker binary, the process that runs
//! untrusted wasm. A client there would be a client in that process — along with
//! the DNS pinning, the retry-free request builder and the code path that would
//! ever hold a credential. Phase 2 moved every `http_request` here, so the
//! worker ends the wave with no HTTP client, no credential bytes, no decryption
//! key and no network library at all: `reqwest` is not a dependency of
//! `plugin-host`, and `cargo tree -p plugin-host` proves it.
//!
//! What the worker keeps is the ABI edge and the gate. What this module owns is
//! the only place a request is built, guarded and sent.
//!
//! ## Order, and what each step stops
//!
//! 1. **The method**, from the ABI's closed set. `TRACE` and friends are a frame
//!    error, not a write someone forgot to gate.
//! 2. **The guest's headers.** A name the transport owns (`Host`,
//!    `Content-Length`, the hop-by-hop set) is refused rather than overridden —
//!    a guest setting `Host` is picking a virtual host the allowlist never
//!    named. A guest naming the header its own credential is injected as is
//!    refused for the same reason: it must not be able to add a second value to
//!    an authenticated header.
//! 3. **Resolution, once**, on a blocking thread. Every answer is judged by
//!    `core_domain::egress::check_resolved`, and the client is pinned to exactly
//!    the addresses that passed. There is no second lookup between the check and
//!    the connect, which is the window a DNS-rebinding attacker needs. The pin
//!    is a `resolve_to_addrs` entry, so TLS still verifies the certificate
//!    against the hostname rather than an address.
//! 4. **The credential**, decrypted here and nowhere else: the vault seals fields
//!    against `secret_field:{org}:{record}:{field}` and core-api owns the key.
//!    The value goes into one header and is never returned, logged or put in a
//!    reply.
//! 5. **The send**, bounded by the caller's remaining transport budget (which is
//!    already at most [`abi::HTTP_TIMEOUT_MS`]), with redirects refused and the
//!    body capped while it is read rather than after it is buffered. A response
//!    header that echoes the credential's own name is dropped: the guest never
//!    had the value, so a reply header must not hand it one.
//!
//! The URL itself was screened by the caller against the install's consented
//! scopes and arrives as an [`EgressTarget`]; this module fetches that string
//! and never re-parses the guest's.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use core_domain::egress::{self, EgressTarget};
use plugin_host::abi::{self, HttpRequest, HttpResponse, Refusal};
use plugin_host::egress::{HttpEgress, SealedCredential};
use reqwest::redirect::Policy;
use reqwest::Method;

use crate::config::SecretKey;
use crate::crypto;

/// Headers the TRANSPORT owns. A guest that names one is refused rather than
/// overridden, because `Host` picks a virtual host the allowlist never named and
/// the framing headers decide how the bytes are read.
///
/// The one list: `routes::plugin_credentials` refuses to bind a credential to a
/// name on it, and this module refuses to send a request that carries one.
pub const HOST_OWNED_HEADERS: [&str; 8] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "proxy-connection",
    "te",
    "trailer",
];

/// Methods the ABI accepts. A method outside this set is a frame error rather
/// than a write: [`HttpRequest::is_write`] treats everything unknown as a write,
/// so silently accepting one would grant a capability question to an accident.
const METHODS: [&str; 7] = ["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"];

const MAX_GUEST_HEADERS: usize = 32;
const MAX_HEADER_NAME_BYTES: usize = 64;
const MAX_HEADER_VALUE_BYTES: usize = 4096;
const MAX_HEADER_BYTES: usize = 8192;
const MAX_RESPONSE_HEADERS: usize = 64;
/// Pinned clients kept per process, keyed by host. A connector talks to a handful
/// of SaaS hosts; the cap bounds a plugin that rotates names.
const MAX_CACHED_CLIENTS: usize = 32;

/// What `prepare` answers: the guest's judged headers, and the host's own when a
/// credential was bound. Kept a pair rather than a struct because the two halves
/// have different laws — one may be logged, the other prints names only — and the
/// call site consumes them in that order.
type PreparedHeaders = (Vec<(String, String)>, Option<HostCredentials>);

/// The headers the HOST adds after the guest's have been judged.
///
/// Deliberately not `Debug`: a derived output would put the token in a log line
/// the first time anyone formatted a request, and "phase 2 adds a log line" is
/// exactly where a secret escapes. The manual impl prints names only. This type
/// is the one thing phase 1's `plugin-host` copy was for, moved here with the
/// transport that uses it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HostCredentials {
    headers: Vec<(String, String)>,
}

impl HostCredentials {
    /// One host-owned header. The name and the value are validated the same way
    /// a guest header is, so a vault value with a newline in it fails here
    /// rather than in a request.
    pub fn header(name: &str, value: &str) -> Result<Self, Refusal> {
        let name = name.to_ascii_lowercase();
        if !is_header_name(&name) || !is_header_value(value) {
            return Err(Refusal::CredentialRefused);
        }
        Ok(HostCredentials {
            headers: vec![(name, value.to_string())],
        })
    }
}

impl std::fmt::Debug for HostCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCredentials")
            .field(
                "headers",
                &self.headers.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// The parent's transport. One per invocation: it holds the org's at-rest key,
/// which is `Copy`, and no per-call state.
pub struct ParentEgress {
    key: SecretKey,
}

impl ParentEgress {
    pub fn new(key: SecretKey) -> Self {
        ParentEgress { key }
    }
}

impl HttpEgress for ParentEgress {
    fn send<'a>(
        &'a mut self,
        target: &'a EgressTarget,
        request: &'a HttpRequest,
        credential: Option<&'a SealedCredential>,
        timeout: Duration,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, Refusal>> + Send + 'a>,
    > {
        Box::pin(async move { self.perform(target, request, credential, timeout).await })
    }
}

impl ParentEgress {
    async fn perform(
        &self,
        target: &EgressTarget,
        request: &HttpRequest,
        credential: Option<&SealedCredential>,
        timeout: Duration,
    ) -> Result<HttpResponse, Refusal> {
        // Everything that can refuse without a socket refuses first: a request
        // this module would reject must not resolve a name, open a connection,
        // or touch the vault (the credential is decrypted in `prepare`, after
        // the guest's headers have been judged).
        let (headers, credentials) = self.prepare(request, credential)?;
        let addrs = resolve(target).await?;
        let client = client_for(target.host(), &addrs)?;
        send_with(
            &client,
            target.as_str(),
            request,
            &headers,
            credentials.as_ref(),
            credential.map(|credential| credential.header_name.as_str()),
            timeout,
        )
        .await
    }

    /// The guest's headers, judged, and the host's own if there is a credential.
    ///
    /// The two are returned apart, not merged: the guest's headers are the ones a
    /// debug log may print, and the host's are the ones whose `Debug` prints names
    /// only. Order matters too — the guest's headers are validated BEFORE the
    /// credential is decrypted, so a guest that names a host-owned header cannot
    /// make the host open a vault record.
    fn prepare(
        &self,
        request: &HttpRequest,
        credential: Option<&SealedCredential>,
    ) -> Result<PreparedHeaders, Refusal> {
        let headers = guest_headers(request, credential)?;
        let credentials = match credential {
            Some(credential) => {
                let (name, value) = self.present(credential)?;
                Some(HostCredentials::header(&name, &value)?)
            }
            None => None,
        };
        Ok((headers, credentials))
    }

    /// Decrypts one sealed credential into the header it is injected as.
    ///
    /// The AAD is rebuilt from the row the resolve read
    /// (`secret_field:{org}:{record}:{field}`), with the legacy no-AAD open as
    /// the fallback for importer-forwarded rows — the same dance the reveal
    /// endpoint does. On any failure the refusal is
    /// [`Refusal::CredentialRefused`] and the cause goes to a log line, never to
    /// the guest and never to the audit row's seven frozen keys.
    fn present(&self, credential: &SealedCredential) -> Result<(String, String), Refusal> {
        let aad = crate::routes::secrets::field_aad(
            credential.org_id,
            credential.record_id,
            &credential.field_key,
        );
        let plaintext = match crypto::open_with_aad(&self.key, &credential.sealed, &aad) {
            Ok(value) => value,
            Err(_) => match crypto::open(&self.key, &credential.sealed) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        record_id = %credential.record_id,
                        error = %error,
                        cause = "unseal_failed",
                        "a bound credential could not be decrypted"
                    );
                    return Err(Refusal::CredentialRefused);
                }
            },
        };
        let Ok(value) = String::from_utf8(plaintext) else {
            tracing::warn!(
                record_id = %credential.record_id,
                cause = "not_utf8",
                "a bound credential is not a UTF-8 header value"
            );
            return Err(Refusal::CredentialRefused);
        };
        if !is_header_name(&credential.header_name) || !is_header_value(&value) {
            // A newline in the value, or a header name the transport could not
            // send: the record is unusable as a credential, which is not the
            // guest's doing.
            tracing::warn!(
                record_id = %credential.record_id,
                cause = "header_shape",
                "a bound credential is not a valid header"
            );
            return Err(Refusal::CredentialRefused);
        }
        Ok((credential.header_name.to_ascii_lowercase(), value))
    }
}

/// Resolves a non-literal host once and judges every answer. An IP-literal target
/// needs no lookup: the guard already classified it.
async fn resolve(target: &EgressTarget) -> Result<Vec<SocketAddr>, Refusal> {
    if target.is_ip_literal() {
        return Ok(Vec::new());
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((target.host(), target.port()))
        .await
        .map_err(|_| Refusal::EgressHostDenied)?
        .collect();
    let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
    egress::check_resolved(&ips).map_err(plugin_host::abi::refusal_for)?;
    Ok(addrs)
}

/// One pinned client and the address set it was built for.
struct Pinned {
    addrs: Vec<SocketAddr>,
    client: reqwest::Client,
}

/// The client for `host`, pinned to `addrs`, reused while the address set is
/// unchanged. Building a client per request would pay for a TLS configuration and
/// a connection pool every call; reusing one across a changed address set would
/// drop the pin, so the key is the host and the value is the set that was
/// checked.
fn client_for(host: &str, addrs: &[SocketAddr]) -> Result<reqwest::Client, Refusal> {
    static CLIENTS: OnceLock<Mutex<HashMap<String, Pinned>>> = OnceLock::new();
    let cache = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(pinned) = guard.get(host) {
        if pinned.addrs == addrs {
            return Ok(pinned.client.clone());
        }
    }
    let client = build_client(host, addrs)?;
    if guard.len() >= MAX_CACHED_CLIENTS {
        guard.clear();
    }
    guard.insert(
        host.to_string(),
        Pinned {
            addrs: addrs.to_vec(),
            client: client.clone(),
        },
    );
    Ok(client)
}

/// The request and response mechanics, separated from the guard and the pin so a
/// local listener can drive them without a name or a certificate.
async fn send_with(
    client: &reqwest::Client,
    url: &str,
    request: &HttpRequest,
    headers: &[(String, String)],
    credentials: Option<&HostCredentials>,
    bound_header: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, Refusal> {
    let method = parse_method(&request.method)?;
    let mut builder = client.request(method, url);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if let Some(credentials) = credentials {
        for (name, value) in &credentials.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
    }
    if let Some(body) = &request.body {
        builder = builder.body(body.clone());
    }

    // The caller's budget is at most `HTTP_TIMEOUT_MS`; this is the bound, not
    // the client's own, so a caller with less left gets less.
    let mut response = tokio::time::timeout(timeout, builder.send())
        .await
        .map_err(|_| Refusal::EgressHostDenied)?
        .map_err(|_| Refusal::EgressHostDenied)?;

    let status = response.status().as_u16();
    // `Policy::none` means a redirect arrives here as a response. It is not
    // followed: the target is a host the install never consented to.
    if egress::is_redirect(status) {
        return Err(Refusal::EgressHostDenied);
    }
    if response
        .content_length()
        .is_some_and(|len| len > abi::HTTP_RESPONSE_MAX_BYTES as u64)
    {
        return Err(Refusal::EgressResponseTooLarge);
    }
    let mut headers = response_headers(&response);
    // A vendor that echoes the credential back must not have it forwarded to
    // the guest: the guest never had the value, and a reply header is a reply
    // field. Cheap to close, and a header is the likely shape of an echo.
    if let Some(bound) = bound_header {
        headers.remove(bound);
    }
    let mut body = Vec::new();
    // Read one chunk at a time and stop at the cap, so a body that lies about
    // its length is refused while it is read rather than after it is buffered.
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Refusal::EgressHostDenied)?
    {
        body.extend_from_slice(&chunk);
        if body.len() > abi::HTTP_RESPONSE_MAX_BYTES {
            return Err(Refusal::EgressResponseTooLarge);
        }
    }
    Ok(HttpResponse {
        status,
        headers,
        // v1 bodies are UTF-8 (the ABI's `body: String`). A binary body is
        // replaced rather than refused, and a `body_b64` field is the additive
        // fix the ABI comment already names.
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn build_client(host: &str, addrs: &[SocketAddr]) -> Result<reqwest::Client, Refusal> {
    let mut builder = reqwest::Client::builder()
        // The per-call bound is the caller's `timeout`; this is the ceiling a
        // client can ever be asked for, so a caller cannot exceed the ABI's own
        // number by passing a larger one.
        .timeout(Duration::from_millis(abi::HTTP_TIMEOUT_MS))
        .connect_timeout(Duration::from_millis(abi::HTTP_TIMEOUT_MS))
        .redirect(Policy::none())
        .user_agent(concat!("marula-plugin-host/", env!("CARGO_PKG_VERSION")));
    if !addrs.is_empty() {
        builder = builder.resolve_to_addrs(host, addrs);
    }
    builder.build().map_err(|_| Refusal::EgressHostDenied)
}

fn parse_method(raw: &str) -> Result<Method, Refusal> {
    let upper = raw.trim().to_ascii_uppercase();
    if !METHODS.contains(&upper.as_str()) {
        return Err(Refusal::FrameInvalid);
    }
    Method::from_bytes(upper.as_bytes()).map_err(|_| Refusal::FrameInvalid)
}

/// Validates the guest's headers and drops nothing silently: a header the
/// transport owns is a frame error, not an override.
fn guest_headers(
    request: &HttpRequest,
    credential: Option<&SealedCredential>,
) -> Result<Vec<(String, String)>, Refusal> {
    if request.headers.len() > MAX_GUEST_HEADERS {
        return Err(Refusal::FrameInvalid);
    }
    let mut total = 0usize;
    let mut headers = Vec::with_capacity(request.headers.len());
    for (name, value) in &request.headers {
        let name = name.to_ascii_lowercase();
        if !is_header_name(&name) || !is_header_value(value) {
            return Err(Refusal::FrameInvalid);
        }
        if HOST_OWNED_HEADERS.contains(&name.as_str()) {
            return Err(Refusal::FrameInvalid);
        }
        if credential.is_some_and(|credential| credential.header_name.eq_ignore_ascii_case(&name)) {
            // The guest may not add a second value to the header its own
            // credential is injected as.
            return Err(Refusal::FrameInvalid);
        }
        total += name.len() + value.len();
        if total > MAX_HEADER_BYTES {
            return Err(Refusal::FrameInvalid);
        }
        headers.push((name, value.clone()));
    }
    Ok(headers)
}

/// An RFC 9110 token, at most 64 bytes — the same ceiling the wire codec applies
/// to a frame.
pub fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_HEADER_NAME_BYTES
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn is_header_value(value: &str) -> bool {
    value.len() <= MAX_HEADER_VALUE_BYTES
        && !value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
}

/// Truncates on a character boundary, so a multi-byte character straddling the
/// cap cannot panic the host.
fn truncate_utf8(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn response_headers(response: &reqwest::Response) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers().iter().take(MAX_RESPONSE_HEADERS) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        let value = truncate_utf8(value, MAX_HEADER_VALUE_BYTES);
        headers
            .entry(name.as_str().to_ascii_lowercase())
            .and_modify(|existing: &mut String| {
                existing.push_str(", ");
                existing.push_str(value);
            })
            .or_insert_with(|| value.to_string());
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn key() -> SecretKey {
        SecretKey([7u8; 32])
    }

    fn request(url: &str) -> HttpRequest {
        HttpRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Default::default(),
            body: None,
        }
    }

    /// A loopback listener that answers one request and hands back the bytes it
    /// received, so "the credential reached the wire" is an assertion about
    /// bytes rather than an inference from a code path.
    fn serve_once(status: &str, body: &[u8]) -> (SocketAddr, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let body = body.to_vec();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 8192];
            let read = stream.read(&mut buf).expect("read the request");
            stream
                .write_all(response.as_bytes())
                .and_then(|()| stream.write_all(&body))
                .expect("write");
            String::from_utf8_lossy(&buf[..read]).into_owned()
        });
        (addr, handle)
    }

    fn sealed_for(org: uuid::Uuid, record: uuid::Uuid, value: &str) -> SealedCredential {
        SealedCredential {
            org_id: org,
            record_id: record,
            field_key: "secret".into(),
            header_name: "x-api-key".into(),
            sealed: crypto::seal_with_aad(
                &key(),
                value.as_bytes(),
                &crate::routes::secrets::field_aad(org, record, "secret"),
            )
            .expect("seal"),
        }
    }

    /// THE PROPERTY: the credential reaches the wire, and the reply carries
    /// nothing about it.
    ///
    /// `prepare` is the half of `perform` that decrypts and injects; `send_with`
    /// is the half that talks to a socket. A loopback listener answers a plain
    /// request because the guard would never allow one — the guard's own proof
    /// is in `core_domain::egress` and in the composed refusal tests below.
    #[tokio::test]
    async fn the_credential_reaches_the_wire_and_never_comes_back() {
        let (addr, seen) = serve_once("200 OK", b"{\"ok\":true}");
        let credential = sealed_for(uuid::Uuid::now_v7(), uuid::Uuid::now_v7(), "s3cret-token");
        let egress = ParentEgress::new(key());
        let (headers, credentials) = egress
            .prepare(&request(&format!("http://{addr}/v1")), Some(&credential))
            .expect("the credential is presentable");
        let credentials = credentials.expect("the credential was presented");
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("test client");
        let response = send_with(
            &client,
            &format!("http://{addr}/v1"),
            &request(&format!("http://{addr}/v1")),
            &headers,
            Some(&credentials),
            Some("x-api-key"),
            Duration::from_millis(abi::HTTP_TIMEOUT_MS),
        )
        .await
        .expect("a bounded response");

        let wire = seen.join().expect("listener thread");
        assert!(
            wire.to_ascii_lowercase()
                .contains("x-api-key: s3cret-token"),
            "the bound header reached the wire: {wire}"
        );
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "{\"ok\":true}");
        let rendered = format!("{response:?}");
        assert!(
            !rendered.contains("s3cret-token"),
            "the credential is never part of the reply: {rendered}"
        );
        assert!(
            !format!("{credential:?}").contains("s3cret-token"),
            "and never part of the credential's own Debug"
        );
        // The header list IS the wire form, so it holds the value by
        // construction — what must not hold it is the reply and the type's
        // Debug, asserted above.
    }

    /// A guest may not set the header its credential is injected as, nor a
    /// header the transport owns — and both refusals happen before any socket,
    /// because `prepare` is called first.
    #[test]
    fn a_guest_header_that_collides_is_refused_before_the_socket() {
        let credential = SealedCredential {
            org_id: uuid::Uuid::now_v7(),
            record_id: uuid::Uuid::now_v7(),
            field_key: "secret".into(),
            header_name: "authorization".into(),
            sealed: vec![0u8; 16],
        };
        let egress = ParentEgress::new(key());
        let mut extra = request("https://api.acme.test/v1");
        extra
            .headers
            .insert("Authorization".into(), "Bearer guest".into());
        assert_eq!(
            egress.prepare(&extra, Some(&credential)),
            Err(Refusal::FrameInvalid),
            "the guest must not add a value to the authenticated header"
        );
        for owned in HOST_OWNED_HEADERS {
            let mut req = request("https://api.acme.test/v1");
            req.headers.insert(owned.into(), "x".into());
            assert_eq!(
                egress.prepare(&req, None),
                Err(Refusal::FrameInvalid),
                "{owned}"
            );
        }
        // A newline in a value is a smuggling attempt, not a header.
        let mut smuggled = request("https://api.acme.test/v1");
        smuggled
            .headers
            .insert("x-test".into(), "a\r\nHost: evil".into());
        assert_eq!(egress.prepare(&smuggled, None), Err(Refusal::FrameInvalid));
    }

    /// A sealed credential the key cannot open refuses with code 25's refusal.
    /// `prepare` runs before `resolve` and before the client exists, so the
    /// request never leaves: there is no socket in this test to leave from.
    #[test]
    fn an_unopenable_credential_refuses() {
        let credential = SealedCredential {
            org_id: uuid::Uuid::now_v7(),
            record_id: uuid::Uuid::now_v7(),
            field_key: "secret".into(),
            header_name: "x-api-key".into(),
            sealed: b"not ciphertext".to_vec(),
        };
        let egress = ParentEgress::new(key());
        assert_eq!(
            egress.prepare(&request("https://api.acme.test/v1"), Some(&credential)),
            Err(Refusal::CredentialRefused)
        );
    }

    /// A credential whose value a header cannot carry refuses as a credential
    /// problem, not as a guest frame problem.
    #[test]
    fn a_credential_that_is_not_a_header_value_refuses_as_a_credential() {
        let org = uuid::Uuid::now_v7();
        let record = uuid::Uuid::now_v7();
        let credential = sealed_for(org, record, "line\r\nInjected: yes");
        let egress = ParentEgress::new(key());
        assert_eq!(
            egress.prepare(&request("https://api.acme.test/v1"), Some(&credential)),
            Err(Refusal::CredentialRefused)
        );
    }

    /// A vendor that echoes the credential in a response header must not have it
    /// forwarded to the guest: the guest never had the value, and a reply header
    /// is a reply field.
    #[tokio::test]
    async fn a_response_header_echoing_the_credential_is_stripped() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nX-Api-Key: s3cret-token\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            );
        });
        let credential = sealed_for(uuid::Uuid::now_v7(), uuid::Uuid::now_v7(), "s3cret-token");
        let egress = ParentEgress::new(key());
        let (headers, credentials) = egress
            .prepare(&request(&format!("http://{addr}/v1")), Some(&credential))
            .expect("presentable");
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("test client");
        let response = send_with(
            &client,
            &format!("http://{addr}/v1"),
            &request(&format!("http://{addr}/v1")),
            &headers,
            credentials.as_ref(),
            Some("x-api-key"),
            Duration::from_millis(abi::HTTP_TIMEOUT_MS),
        )
        .await
        .expect("a bounded response");
        handle.join().expect("listener");
        assert!(
            !response.headers.contains_key("x-api-key"),
            "the echo is stripped: {:?}",
            response.headers
        );
        assert_eq!(response.body, "ok");
        assert!(
            !format!("{response:?}").contains("s3cret-token"),
            "and nothing else in the reply carries it"
        );
    }

    /// The type that holds the value must not print it. Phase 2 adds a log line;
    /// this is where a secret usually gets into one.
    #[test]
    fn the_host_credential_type_prints_names_not_values() {
        let credentials =
            HostCredentials::header("Authorization", "Bearer s3cret-token").expect("a header");
        let rendered = format!("{credentials:?}");
        assert!(rendered.contains("authorization"));
        assert!(
            !rendered.contains("s3cret-token"),
            "a formatted credential must not carry its value: {rendered}"
        );
        assert!(
            credentials
                .headers
                .iter()
                .all(|(name, _)| name == "authorization"),
            "the name is lowercased, so the guest's spelling cannot dodge the collision check"
        );
        // A value a header cannot carry is refused as a credential, not as a
        // guest frame.
        assert_eq!(
            HostCredentials::header("authorization", "a\r\nInjected: yes"),
            Err(Refusal::CredentialRefused)
        );
    }

    #[test]
    fn an_unknown_method_is_a_frame_error() {
        assert_eq!(parse_method("TRACE"), Err(Refusal::FrameInvalid));
        assert_eq!(parse_method("get"), Ok(Method::GET));
        assert_eq!(parse_method("delete"), Ok(Method::DELETE));
    }

    #[tokio::test]
    async fn a_body_over_the_cap_is_refused_while_reading() {
        // No Content-Length: the cap is enforced by the read loop, not by a
        // header a server can lie about.
        let cap = abi::HTTP_RESPONSE_MAX_BYTES;
        let (addr, _seen) = serve_once("200 OK", &vec![b'a'; cap + 1]);
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("test client");
        assert_eq!(
            send_with(
                &client,
                &format!("http://{addr}/v1"),
                &request(&format!("http://{addr}/v1")),
                &[],
                None,
                None,
                Duration::from_millis(abi::HTTP_TIMEOUT_MS),
            )
            .await,
            Err(Refusal::EgressResponseTooLarge)
        );
    }

    #[tokio::test]
    async fn a_redirect_is_refused_rather_than_followed() {
        // If the client followed, it would leave for `evil.example` and the call
        // would return Ok or a connection error. It must see the 302 instead.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("test client");
        assert_eq!(
            send_with(
                &client,
                &format!("http://{addr}/v1"),
                &request(&format!("http://{addr}/v1")),
                &[],
                None,
                None,
                Duration::from_millis(abi::HTTP_TIMEOUT_MS),
            )
            .await,
            Err(Refusal::EgressHostDenied)
        );
        handle.join().expect("listener");
    }

    /// A name that does not resolve is refused rather than attempted, and a name
    /// that resolves into a private range is refused too — the `check_resolved`
    /// half, which cannot be driven from a listener because the guard refuses a
    /// loopback target before a socket exists.
    #[tokio::test]
    async fn a_name_that_does_not_resolve_is_refused() {
        let target = egress::screen("https://no-such-host.invalid/v1").expect("a public name");
        assert_eq!(resolve(&target).await, Err(Refusal::EgressHostDenied));
    }

    #[test]
    fn the_domain_verdict_maps_onto_the_closed_egress_vocabulary() {
        use core_domain::egress::EgressDenial;
        assert_eq!(
            plugin_host::abi::refusal_for(EgressDenial::EmptyAllowlist),
            Refusal::EgressProfileMissing
        );
        assert_eq!(
            plugin_host::abi::refusal_for(EgressDenial::SchemeDenied),
            Refusal::EgressSchemeDenied
        );
        for denial in [
            EgressDenial::HostDenied,
            EgressDenial::PathDenied,
            EgressDenial::PrivateAddress,
            EgressDenial::UserInfoDenied,
            EgressDenial::PortDenied,
            EgressDenial::RedirectDenied,
            EgressDenial::Unresolved,
            EgressDenial::ScopeMalformed,
        ] {
            assert_eq!(
                plugin_host::abi::refusal_for(denial),
                Refusal::EgressHostDenied,
                "{denial:?}"
            );
        }
    }
}
