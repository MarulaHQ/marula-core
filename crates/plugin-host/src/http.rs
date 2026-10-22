//! The transport half of `http_request` (W53-7): the socket, the limits, and
//! the one place a credential may be added.
//!
//! The authorization half is already decided before this module is reached.
//! [`crate::host_fns::HostGate::Egress`] resolves the capability, checks that
//! the named profile is in the install's consent, and runs
//! [`core_domain::egress`] against the request URL. What is left is the part
//! that needs a socket, and every rule below comes from ADR 0023 Decision 4:
//!
//! | Rule | Value | Where |
//! |---|---|---|
//! | Connect and read timeout, total | 5,000 ms | [`abi::HTTP_TIMEOUT_MS`] |
//! | Response body cap | 2 MiB | [`abi::HTTP_RESPONSE_MAX_BYTES`] |
//! | Redirects followed | 0 | [`core_domain::egress::MAX_REDIRECTS`] |
//!
//! ## Resolution happens once, and the answer is pinned
//!
//! A name is resolved here, every answer is judged by
//! [`core_domain::egress::check_resolved`], and the client is pinned to exactly
//! the addresses that passed. There is no second lookup between the check and
//! the connect, which is the window a DNS-rebinding attacker needs. The pin is
//! a `resolve_to_addrs` entry on the client, so TLS still verifies the
//! certificate against the hostname rather than an address.
//!
//! ## Credentials
//!
//! The guest never sees a credential, cannot set one, and cannot read one back.
//! [`HostCredentials`] is applied by the host after the guard has passed and
//! after the guest's headers, and a guest that names a header the host owns is
//! refused rather than overridden. That is the whole point of the `secret_get`
//! deferral in ADR 0023: reading a secret and posting it out is one call, and
//! handing the guest a header value would be the same hole with extra steps.
//!
//! **The source of those credentials is not wired in this wave.** The handler
//! receives no install-scoped vault handle — `HostCall` is
//! `fn(&mut InvocationState, &dyn GuestMemory, &[i32])` and `InvocationState`
//! carries no credential — so [`send`] is called with `None` and a connector
//! that needs a credential cannot yet be served. The injection path below is
//! built and tested so the wiring is a data question, not a security one.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use core_domain::egress::{self, EgressDenial, EgressTarget};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::redirect::Policy;
use reqwest::Method;

use crate::abi::{self, HttpRequest, HttpResponse, Refusal};

/// Headers the host owns: the guest may not set them. `Host` would let a guest
/// pick a different virtual host behind an allowlisted address, and the framing
/// headers are the request-smuggling surface.
const HOST_OWNED_HEADERS: [&str; 8] = [
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
/// than a write: `HttpRequest::is_write` treats everything unknown as a write,
/// so silently accepting one would grant a capability question to an accident.
const METHODS: [&str; 7] = ["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"];

const MAX_GUEST_HEADERS: usize = 32;
const MAX_HEADER_NAME_BYTES: usize = 64;
const MAX_HEADER_VALUE_BYTES: usize = 4096;
const MAX_HEADER_BYTES: usize = 8192;
const MAX_RESPONSE_HEADERS: usize = 64;
/// Pinned clients kept per process, keyed by host. A connector talks to a
/// handful of SaaS hosts; the cap bounds a plugin that rotates names.
const MAX_CACHED_CLIENTS: usize = 32;

/// Maps a domain verdict onto the plugin refusal vocabulary. The mapping lives
/// here because `plugin-host` owns the ABI strings, and the `Egress` gate and
/// the handler must answer with the same code for the same fact.
pub(crate) fn refusal_for(denial: EgressDenial) -> Refusal {
    match denial {
        EgressDenial::EmptyAllowlist => Refusal::EgressProfileMissing,
        EgressDenial::SchemeDenied => Refusal::EgressSchemeDenied,
        EgressDenial::MalformedUrl | EgressDenial::UrlTooLong => Refusal::FrameInvalid,
        // The ABI's egress vocabulary is closed at four strings (ADR 0023
        // Decision 3), so every host-shaped denial shares one. The domain
        // verdict keeps them apart for the audit detail; the guest sees the
        // coarse code.
        EgressDenial::UserInfoDenied
        | EgressDenial::PortDenied
        | EgressDenial::HostDenied
        | EgressDenial::PathDenied
        | EgressDenial::PrivateAddress
        | EgressDenial::RedirectDenied
        | EgressDenial::Unresolved
        | EgressDenial::ScopeMalformed => Refusal::EgressHostDenied,
        // `EgressDenial` is `non_exhaustive`: a variant a later wave adds fails
        // closed here rather than falling through to a success.
        _ => Refusal::EgressHostDenied,
    }
}

/// Headers the HOST adds after the guard has passed.
///
/// Deliberately not `Debug`: the derived output would put the token in a log
/// line the first time anyone formatted a request. The manual impl prints names
/// only.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct HostCredentials {
    headers: Vec<(String, String)>,
}

impl HostCredentials {
    /// A bearer token, the shape a connector profile's OAuth grant takes.
    #[allow(dead_code)] // built and tested; the install-scoped source is the gap.
    pub(crate) fn bearer(token: &str) -> Result<Self, Refusal> {
        Self::header("authorization", &format!("Bearer {token}"))
    }

    /// One host-owned header. Name and value are validated the same way a guest
    /// header is, so a profile with a newline in it fails at construction.
    #[allow(dead_code)] // built and tested; the install-scoped source is the gap.
    pub(crate) fn header(name: &str, value: &str) -> Result<Self, Refusal> {
        let name = name.to_ascii_lowercase();
        if !is_header_name(&name) || !is_header_value(value) {
            return Err(Refusal::FrameInvalid);
        }
        Ok(HostCredentials {
            headers: vec![(name, value.to_string())],
        })
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.headers.iter().map(|(name, _)| name.as_str())
    }

    fn apply(&self, request: RequestBuilder) -> RequestBuilder {
        self.headers.iter().fold(request, |builder, (name, value)| {
            builder.header(name.as_str(), value.as_str())
        })
    }
}

impl fmt::Debug for HostCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostCredentials")
            .field(
                "headers",
                &self.headers.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// One pinned client and the address set it was built for.
struct Pinned {
    addrs: Vec<SocketAddr>,
    client: Client,
}

/// Sends one authorized request and returns the bounded response.
///
/// The caller passes the [`EgressTarget`] the guard produced, so the transport
/// fetches the exact URL that was screened rather than re-parsing the guest's
/// string. `credentials` is host-only state; the guest neither supplies it nor
/// sees the result.
pub(crate) fn send(
    target: &EgressTarget,
    request: &HttpRequest,
    credentials: Option<&HostCredentials>,
) -> Result<HttpResponse, Refusal> {
    let addrs = resolve(target)?;
    let client = client_for(target.host(), &addrs)?;
    send_with(&client, target.as_str(), request, credentials)
}

/// Resolves a non-literal host once and judges every answer. An IP-literal
/// target needs no lookup: the guard already classified it.
fn resolve(target: &EgressTarget) -> Result<Vec<SocketAddr>, Refusal> {
    if target.is_ip_literal() {
        return Ok(Vec::new());
    }
    let addrs: Vec<SocketAddr> = (target.host(), target.port())
        .to_socket_addrs()
        .map_err(|_| Refusal::EgressHostDenied)?
        .collect();
    let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
    egress::check_resolved(&ips).map_err(refusal_for)?;
    Ok(addrs)
}

/// The client for `host`, pinned to `addrs`, reused while the address set is
/// unchanged. Building a client per request would pay for a TLS configuration
/// and a connection pool every call; reusing one would drop the pin, so the key
/// is the host and the value is the set that was checked.
fn client_for(host: &str, addrs: &[SocketAddr]) -> Result<Client, Refusal> {
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

fn build_client(host: &str, addrs: &[SocketAddr]) -> Result<Client, Refusal> {
    let mut builder = Client::builder()
        // One budget for the whole exchange, connect included: the plugin must
        // fail before the consumer's retry cap, not after it.
        .timeout(Duration::from_millis(abi::HTTP_TIMEOUT_MS))
        .connect_timeout(Duration::from_millis(abi::HTTP_TIMEOUT_MS))
        .redirect(Policy::none())
        .user_agent(concat!("marula-plugin-host/", env!("CARGO_PKG_VERSION")));
    if !addrs.is_empty() {
        builder = builder.resolve_to_addrs(host, addrs);
    }
    builder.build().map_err(|_| Refusal::EgressHostDenied)
}

/// The request and response mechanics, separated from resolution so a local
/// listener can drive them without a name or a certificate.
fn send_with(
    client: &Client,
    url: &str,
    request: &HttpRequest,
    credentials: Option<&HostCredentials>,
) -> Result<HttpResponse, Refusal> {
    let method = parse_method(&request.method)?;
    let headers = guest_headers(request, credentials)?;
    let mut builder = client.request(method, url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    if let Some(credentials) = credentials {
        builder = credentials.apply(builder);
    }
    if let Some(body) = &request.body {
        builder = builder.body(body.clone());
    }

    let response = builder.send().map_err(|_| Refusal::EgressHostDenied)?;
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
    let headers = response_headers(&response);
    let mut body = Vec::new();
    // Read one byte past the cap so a body that is exactly at the cap is kept
    // and one byte over is refused, without buffering an unbounded stream.
    let mut reader = response.take(abi::HTTP_RESPONSE_MAX_BYTES as u64 + 1);
    reader
        .read_to_end(&mut body)
        .map_err(|_| Refusal::EgressHostDenied)?;
    if body.len() > abi::HTTP_RESPONSE_MAX_BYTES {
        return Err(Refusal::EgressResponseTooLarge);
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

fn parse_method(raw: &str) -> Result<Method, Refusal> {
    let upper = raw.trim().to_ascii_uppercase();
    if !METHODS.contains(&upper.as_str()) {
        return Err(Refusal::FrameInvalid);
    }
    Method::from_bytes(upper.as_bytes()).map_err(|_| Refusal::FrameInvalid)
}

/// Validates the guest's headers and drops nothing silently: a header the host
/// owns is a frame error, not an override, because a guest setting `Host` is
/// trying to pick a virtual host the allowlist never named.
fn guest_headers(
    request: &HttpRequest,
    credentials: Option<&HostCredentials>,
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
        if credentials.is_some_and(|creds| creds.names().any(|owned| owned == name)) {
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

fn is_header_name(name: &str) -> bool {
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

fn response_headers(response: &Response) -> std::collections::BTreeMap<String, String> {
    let mut headers = std::collections::BTreeMap::new();
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
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    fn request(method: &str, url: &str) -> HttpRequest {
        HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: Default::default(),
            body: None,
        }
    }

    /// A plain-HTTP local listener, so the transport mechanics are exercised
    /// without a name, a certificate or the internet.
    fn serve_once(response: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(&response);
            }
        });
        addr
    }

    fn plain_client() -> Client {
        Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(Policy::none())
            .build()
            .expect("test client")
    }

    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut out =
            format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n").into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn a_redirect_is_refused_rather_than_followed() {
        // If the client followed, the second listener would answer 200 and the
        // call would return Ok. It must not be contacted at all.
        let destination = serve_once(response("200 OK", "Content-Length: 2\r\n", b"ok"));
        let redirect = serve_once(response(
            "302 Found",
            &format!("Location: http://{destination}/steal\r\nContent-Length: 0\r\n"),
            b"",
        ));
        let error = send_with(
            &plain_client(),
            &format!("http://{redirect}/"),
            &request("GET", &format!("http://{redirect}/")),
            None,
        )
        .expect_err("a redirect is not followed");
        assert_eq!(error, Refusal::EgressHostDenied);
    }

    #[test]
    fn a_body_over_the_cap_is_refused_before_it_is_buffered() {
        let cap = abi::HTTP_RESPONSE_MAX_BYTES;
        let addr = serve_once(response(
            "200 OK",
            &format!("Content-Length: {}\r\n", cap + 1),
            b"",
        ));
        assert_eq!(
            send_with(
                &plain_client(),
                &format!("http://{addr}/"),
                &request("GET", &format!("http://{addr}/")),
                None,
            ),
            Err(Refusal::EgressResponseTooLarge)
        );

        // No Content-Length: the cap is enforced while reading, not by a header
        // a server can lie about.
        let addr = serve_once(response("200 OK", "", &vec![b'a'; cap + 1]));
        assert_eq!(
            send_with(
                &plain_client(),
                &format!("http://{addr}/"),
                &request("GET", &format!("http://{addr}/")),
                None,
            ),
            Err(Refusal::EgressResponseTooLarge)
        );
    }

    #[test]
    fn a_small_response_round_trips_status_headers_and_body() {
        let addr = serve_once(response(
            "201 Created",
            "Content-Type: application/json\r\nContent-Length: 11\r\n",
            b"{\"ok\":true}",
        ));
        let response = send_with(
            &plain_client(),
            &format!("http://{addr}/v1/tickets"),
            &request("GET", &format!("http://{addr}/v1/tickets")),
            None,
        )
        .expect("a bounded response");
        assert_eq!(response.status, 201);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(response.body, "{\"ok\":true}");
    }

    #[test]
    fn the_guest_cannot_set_a_host_owned_header() {
        let mut req = request("GET", "https://api.example.com/v1");
        req.headers.insert("Host".into(), "evil.example.net".into());
        assert_eq!(
            guest_headers(&req, None),
            Err(Refusal::FrameInvalid),
            "Host is the host's to set"
        );
        for owned in HOST_OWNED_HEADERS {
            let mut req = request("GET", "https://api.example.com/v1");
            req.headers.insert(owned.into(), "x".into());
            assert_eq!(
                guest_headers(&req, None),
                Err(Refusal::FrameInvalid),
                "{owned}"
            );
        }
        let mut req = request("GET", "https://api.example.com/v1");
        req.headers
            .insert("x-test".into(), "a\r\nHost: evil".into());
        assert_eq!(
            guest_headers(&req, None),
            Err(Refusal::FrameInvalid),
            "a newline in a value is a smuggling attempt"
        );
    }

    #[test]
    fn a_credential_is_added_by_the_host_and_never_comes_back() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let seen = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 8192];
            let read = stream.read(&mut buf).expect("read the request");
            stream
                .write_all(&response("200 OK", "Content-Length: 2\r\n", b"ok"))
                .expect("write");
            String::from_utf8_lossy(&buf[..read]).into_owned()
        });
        let credentials = HostCredentials::bearer("s3cret-token").expect("valid credential");
        let response = send_with(
            &plain_client(),
            &format!("http://{addr}/v1"),
            &request("GET", &format!("http://{addr}/v1")),
            Some(&credentials),
        )
        .expect("the host adds the credential");
        let wire = seen.join().expect("listener thread");
        assert!(
            wire.to_ascii_lowercase()
                .contains("authorization: bearer s3cret-token"),
            "the host header reached the wire: {wire}"
        );
        assert_eq!(response.body, "ok");
        assert!(
            !format!("{response:?}").contains("s3cret-token"),
            "the credential is never part of the reply"
        );
        // Debug of the credential itself prints names only.
        assert!(!format!("{credentials:?}").contains("s3cret-token"));
        // A guest naming the same header is refused rather than overridden.
        let mut req = request("GET", &format!("http://{addr}/v1"));
        req.headers
            .insert("authorization".into(), "Bearer guest".into());
        assert_eq!(
            guest_headers(&req, Some(&credentials)),
            Err(Refusal::FrameInvalid)
        );
    }

    #[test]
    fn an_unknown_method_is_a_frame_error() {
        assert_eq!(parse_method("TRACE"), Err(Refusal::FrameInvalid));
        assert_eq!(parse_method("get"), Ok(Method::GET));
        assert_eq!(parse_method("delete"), Ok(Method::DELETE));
    }

    #[test]
    fn an_ip_literal_target_needs_no_resolution() {
        let target = egress::screen("https://93.184.216.34/v1").expect("a public literal");
        assert!(target.is_ip_literal());
        assert_eq!(resolve(&target), Ok(Vec::new()));
    }

    #[test]
    fn a_name_that_does_not_resolve_is_refused() {
        // `.invalid` is reserved and never resolves, so this is deterministic
        // and touches no network beyond the resolver's own NXDOMAIN answer.
        let target = egress::screen("https://no-such-host.invalid/v1").expect("a public name");
        assert_eq!(resolve(&target), Err(Refusal::EgressHostDenied));
    }

    #[test]
    fn the_domain_verdict_maps_onto_the_closed_egress_vocabulary() {
        assert_eq!(
            refusal_for(EgressDenial::EmptyAllowlist),
            Refusal::EgressProfileMissing
        );
        assert_eq!(
            refusal_for(EgressDenial::SchemeDenied),
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
            assert_eq!(refusal_for(denial), Refusal::EgressHostDenied, "{denial:?}");
        }
    }
}
