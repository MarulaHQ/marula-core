//! The browser fallback host (Wave 60 lane W60-3, ADR 0027).
//!
//! Purpose: serve the same front-end the native app loads, over loopback, for
//! the machine where the app cannot be installed: a server with no desktop, an
//! air-gapped box whose operator copies one directory over, a CI image. The
//! native shell is the primary; this is the fallback that does not need a
//! webview, a signing identity or a package format.
//!
//! Security posture, in one place:
//!
//! * binds `127.0.0.1` only, and refuses a `Host` header that is not loopback,
//!   so a DNS name that points at this address cannot reach it;
//! * prints one URL carrying a 32-byte token, and that token is redeemed exactly
//!   once for an `HttpOnly`, `SameSite=Strict` session cookie;
//! * every `/api` call needs the cookie *and* an `x-marula-installer: 1` header,
//!   so a cross-site form post or a simple request cannot drive an install;
//! * serves files from one directory with traversal refused, behind a CSP that
//!   allows nothing off this machine;
//! * no telemetry, no outbound call: the only network the process touches is
//!   the loopback socket it was asked to open.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::install::view::Session;

/// What the fallback needs to start.
#[derive(Debug, Clone)]
pub struct Options {
    pub dir: PathBuf,
    pub compose_file: PathBuf,
    pub memory_mb: Option<u32>,
    /// `0` asks the OS for a free port, which is the default: two installers on
    /// one machine must not fight for 8081.
    pub port: u16,
    /// The built front-end (`apps/installer/dist`).
    pub ui_dir: PathBuf,
    /// Whether to open the operator's browser at the printed URL.
    pub open: bool,
}

#[derive(Clone)]
struct App {
    session: Session,
    ui: PathBuf,
    /// `Some` until redeemed, then `None` forever.
    token: Arc<Mutex<Option<String>>>,
    cookie: Arc<String>,
    host: Arc<String>,
    localhost: Arc<String>,
}

impl App {
    fn host_ok(&self, headers: &HeaderMap) -> bool {
        let Some(host) = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        host == self.host.as_str() || host == self.localhost.as_str()
    }

    fn cookie_ok(&self, headers: &HeaderMap) -> bool {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().strip_prefix("marula_installer="))
            .any(|offered| constant_time_eq(offered, &self.cookie))
    }
}

/// Starts the server and blocks until it stops.
pub async fn serve(options: Options) -> anyhow::Result<()> {
    let index = options.ui_dir.join("index.html");
    anyhow::ensure!(
        index.is_file(),
        "no installer UI at {}. Build it with `pnpm --dir apps/installer build`, or point \
         --ui-dir at the directory the release bundle ships.",
        index.display()
    );

    let session = Session::new(&options.dir, &options.compose_file, options.memory_mb);
    let token = generate();
    let cookie = generate();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", options.port)).await?;
    let addr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}/?t={token}", addr.port());

    let app = App {
        session,
        ui: options.ui_dir.clone(),
        token: Arc::new(Mutex::new(Some(token))),
        cookie: Arc::new(cookie),
        host: Arc::new(format!("127.0.0.1:{}", addr.port())),
        localhost: Arc::new(format!("localhost:{}", addr.port())),
    };

    println!("{}", open_line(&url));
    if options.open {
        open_in_browser(&url);
    }

    axum::serve(listener, router(app)).await?;
    Ok(())
}

/// The line an operator reads. The token is in the URL, so the sentence says
/// that the link is single-use rather than letting them paste it into a chat.
#[must_use]
pub fn open_line(url: &str) -> String {
    format!(
        "marula install — open this once, in a browser on this machine:\n  {url}\n\
         the link works once, the cookie it sets lasts until this process stops, and nothing \
         leaves this machine."
    )
}

fn router(app: App) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/assets/{*path}", get(asset))
        .route("/favicon.ico", get(favicon))
        .route("/api/state", get(api_state))
        .route("/api/plan", post(api_plan))
        .route("/api/apply", post(api_apply).get(api_apply_state))
        .route("/api/doctor", get(api_doctor))
        .with_state(app)
}

async fn root(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if app.cookie_ok(&headers) {
        return index_page(&app).await;
    }

    if let Some(offered) = query.get("t") {
        let expected = app.token.lock().ok().and_then(|mut token| token.take());
        if expected
            .as_deref()
            .is_some_and(|expected| constant_time_eq(expected, offered))
        {
            let mut response = Redirect::to("/").into_response();
            let cookie = format!(
                "marula_installer={}; HttpOnly; SameSite=Strict; Path=/",
                app.cookie
            );
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response.headers_mut().insert(header::SET_COOKIE, value);
            }
            harden(&mut response, true);
            return response;
        }
    }

    let mut response = (
        StatusCode::UNAUTHORIZED,
        Html(
            "<!doctype html><meta charset=\"utf-8\"><title>Marula installer</title>\
             <body style=\"font:15px/1.5 system-ui;padding:48px;max-width:44ch\">\
             <p><strong>This link needs the one-time token.</strong></p>\
             <p>Open the exact URL the terminal printed, in a browser on this machine. \
             It carries a token that works once.</p></body>",
        ),
    )
        .into_response();
    harden(&mut response, false);
    response
}

async fn index_page(app: &App) -> Response {
    match tokio::fs::read_to_string(app.ui.join("index.html")).await {
        Ok(html) => {
            let mut response = Html(html).into_response();
            harden(&mut response, false);
            response
        }
        Err(error) => {
            let mut response = (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the installer UI could not be read: {error}"),
            )
                .into_response();
            harden(&mut response, false);
            response
        }
    }
}

async fn favicon() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// Serves one file from the UI directory. A path with anything but plain names
/// is refused rather than normalised, so `..` cannot become a read outside it.
async fn asset(State(app): State<App>, UrlPath(path): UrlPath<String>) -> Response {
    // The wildcard captures what follows `/assets/`, so the prefix is restored
    // here rather than being kept in the request path.
    let mut relative = PathBuf::from("assets");
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains('\\')
            || component.contains(':')
            || component.contains('\0')
        {
            return (StatusCode::BAD_REQUEST, "bad asset path").into_response();
        }
        relative.push(component);
    }
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return (StatusCode::BAD_REQUEST, "bad asset path").into_response();
    }

    let full = app.ui.join(&relative);
    match tokio::fs::read(&full).await {
        Ok(bytes) => {
            let mut response =
                ([(header::CONTENT_TYPE, content_type(&relative))], bytes).into_response();
            // Vite writes content-hashed names, so a long cache is honest.
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
            response
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn content_type(path: &Path) -> HeaderValue {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let value = match extension {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    };
    HeaderValue::from_static(value)
}

/// The one gate every `/api` call passes. `None` means the request may proceed;
/// a response means it may not, and the caller returns it as it stands.
///
/// `Option` rather than `Result<(), Response>`: a `Response` is large, and
/// clippy is right that every `Ok` would then carry it.
fn guard(app: &App, headers: &HeaderMap) -> Option<Response> {
    if !app.host_ok(headers) {
        return Some(refused("this endpoint answers loopback only"));
    }
    if !app.cookie_ok(headers) {
        return Some(refused(
            "the session cookie is missing or wrong; reopen the printed link",
        ));
    }
    let marked = headers
        .get("x-marula-installer")
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if !marked {
        return Some(refused("the installer header is missing"));
    }
    None
}

#[derive(Debug, Deserialize)]
pub struct PlanRequest {
    pub profile: String,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
    /// `build` or `registry`.
    #[serde(default)]
    pub images: Option<String>,
    /// `unless-stopped`, `always` or `no`.
    #[serde(default)]
    pub restart: Option<String>,
    /// Developer mode may plan a shape this machine cannot carry.
    #[serde(default)]
    pub allow_unfit: bool,
    /// The registry prefix for a registry install, when a person types one.
    #[serde(default)]
    pub image_base: Option<String>,
}

impl PlanRequest {
    /// The session this request runs against, carrying its compose-shape choice.
    fn shaped(&self, session: Session) -> std::result::Result<Session, Box<Response>> {
        let images = match self.images.as_deref() {
            None => crate::install::plan::ImageSource::Build,
            Some(value) => crate::install::plan::ImageSource::parse(value).ok_or_else(|| {
                Box::new(refused(&format!(
                    "--images takes build or registry, not {value}"
                )))
            })?,
        };
        let restart = match self.restart.as_deref() {
            None => crate::install::plan::RestartPolicy::UnlessStopped,
            Some(value) => crate::install::plan::RestartPolicy::parse(value).ok_or_else(|| {
                Box::new(refused(&format!(
                    "--restart takes unless-stopped, always or no, not {value}"
                )))
            })?,
        };
        Ok(session
            .with_shape(images, restart, self.allow_unfit)
            .with_images(self.image_base.clone()))
    }
}

#[derive(Debug, Deserialize)]
pub struct DoctorQuery {
    pub profile: String,
}

async fn api_state(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(response) = guard(&app, &headers) {
        return response;
    }
    let session = app.session.clone();
    match tokio::task::spawn_blocking(move || session.state()).await {
        Ok(state) => ok(&state),
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn api_plan(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<PlanRequest>,
) -> Response {
    if let Some(response) = guard(&app, &headers) {
        return response;
    }
    let session = match body.shaped(app.session.clone()) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let planned =
        tokio::task::spawn_blocking(move || session.plan(&body.profile, &body.overrides)).await;
    match planned {
        Ok(Ok(plan)) => ok(&plan),
        Ok(Err(message)) => failed(StatusCode::UNPROCESSABLE_ENTITY, message),
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn api_apply(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<PlanRequest>,
) -> Response {
    if let Some(response) = guard(&app, &headers) {
        return response;
    }
    let session = match body.shaped(app.session.clone()) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let started =
        tokio::task::spawn_blocking(move || session.start_apply(&body.profile, &body.overrides))
            .await;
    match started {
        Ok(Ok(())) => ok(&json!({ "started": true })),
        Ok(Err(message)) => failed(StatusCode::UNPROCESSABLE_ENTITY, message),
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn api_apply_state(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(response) = guard(&app, &headers) {
        return response;
    }
    let session = app.session.clone();
    match tokio::task::spawn_blocking(move || session.apply_state()).await {
        Ok(state) => ok(&state),
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn api_doctor(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<DoctorQuery>,
) -> Response {
    if let Some(response) = guard(&app, &headers) {
        return response;
    }
    let session = app.session.clone();
    let report = tokio::task::spawn_blocking(move || session.doctor(&query.profile)).await;
    match report {
        Ok(Ok(doctor)) => ok(&doctor),
        Ok(Err(message)) => failed(StatusCode::UNPROCESSABLE_ENTITY, message),
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn ok<T: Serialize>(value: &T) -> Response {
    let mut response = Json(value).into_response();
    harden(&mut response, true);
    response
}

fn failed(status: StatusCode, message: String) -> Response {
    let mut response = (status, Json(json!({ "error": message }))).into_response();
    harden(&mut response, true);
    response
}

fn refused(message: &str) -> Response {
    failed(StatusCode::UNAUTHORIZED, message.to_string())
}

/// Headers every response carries. `api` responses are never cached and the
/// page may not be framed, sniffed or referred to.
fn harden(response: &mut Response, api: bool) {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; font-src 'self'; connect-src 'self'; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

fn generate() -> String {
    use rand::TryRng;
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG unavailable");
    hex::encode(bytes)
}

/// Comparing a token with a byte-at-a-time loop that never exits early, so a
/// wrong guess costs the same as a right one up to its first difference.
fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Opens the operator's browser, and says so quietly when it cannot: a headless
/// box is a normal place to run this, and failing the install over a missing
/// `xdg-open` would be absurd.
pub fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let command = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let command = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let command = ("xdg-open", vec![url]);

    if let Err(error) = std::process::Command::new(command.0)
        .args(command.1)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        eprintln!("could not open a browser ({error}); the URL above is the way in");
    }
}

/// Where the fallback should look for the built UI when nothing is passed.
#[must_use]
pub fn default_ui_dir() -> PathBuf {
    PathBuf::from("apps/installer/dist")
}

/// The address the printed URL names, for a test or a caller that wants it.
#[must_use]
pub fn loopback(addr: SocketAddr) -> bool {
    addr.ip().is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn ui_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marula-ui-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("assets")).expect("a scratch dir");
        std::fs::write(dir.join("index.html"), "<!doctype html><div id=root></div>")
            .expect("index");
        std::fs::write(dir.join("assets/app.js"), "console.log('installer')").expect("asset");
        dir
    }

    fn app(ui: &Path) -> App {
        let session = Session::new(
            std::env::temp_dir().join(format!("marula-install-{}", uuid::Uuid::now_v7())),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../deploy/docker-compose.yml")
                .canonicalize()
                .expect("the repo's compose file"),
            None,
        );
        App {
            session,
            ui: ui.to_path_buf(),
            token: Arc::new(Mutex::new(Some("token-in-the-url".to_string()))),
            cookie: Arc::new("session-cookie".to_string()),
            host: Arc::new("127.0.0.1:9123".to_string()),
            localhost: Arc::new("localhost:9123".to_string()),
        }
    }

    fn get(path: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header(header::HOST, "127.0.0.1:9123")
            .body(Body::empty())
            .expect("a request")
    }

    async fn body_text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("a body");
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn the_page_needs_the_cookie_and_the_token_redeems_it_once() {
        let ui = ui_dir();
        let router = router(app(&ui));

        let bare = router.clone().oneshot(get("/")).await.expect("a response");
        assert_eq!(bare.status(), StatusCode::UNAUTHORIZED);

        let redeemed = router
            .clone()
            .oneshot(get("/?t=token-in-the-url"))
            .await
            .expect("a response");
        assert_eq!(redeemed.status(), StatusCode::SEE_OTHER);
        let cookie = redeemed
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(cookie.starts_with("marula_installer=session-cookie"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));

        // The same token is dead on the second use.
        let replay = router
            .clone()
            .oneshot(get("/?t=token-in-the-url"))
            .await
            .expect("a response");
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

        let with_cookie = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::HOST, "127.0.0.1:9123")
                    .header(header::COOKIE, "marula_installer=session-cookie")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("a response");
        assert_eq!(with_cookie.status(), StatusCode::OK);
        assert!(body_text(with_cookie).await.contains("id=root"));

        let _ = std::fs::remove_dir_all(&ui);
    }

    #[tokio::test]
    async fn a_host_that_is_not_loopback_is_refused() {
        let ui = ui_dir();
        let router = router(app(&ui));
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/apply")
                    .header(header::HOST, "marula.example.com")
                    .header(header::COOKIE, "marula_installer=session-cookie")
                    .header("x-marula-installer", "1")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("a response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(&ui);
    }

    #[tokio::test]
    async fn the_api_needs_the_cookie_and_the_header() {
        let ui = ui_dir();
        let router = router(app(&ui));

        let unmarked = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/apply")
                    .header(header::HOST, "127.0.0.1:9123")
                    .header(header::COOKIE, "marula_installer=session-cookie")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("a response");
        assert_eq!(unmarked.status(), StatusCode::UNAUTHORIZED);

        let unauthenticated = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/apply")
                    .header(header::HOST, "127.0.0.1:9123")
                    .header("x-marula-installer", "1")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("a response");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let allowed = router
            .oneshot(
                Request::builder()
                    .uri("/api/apply")
                    .header(header::HOST, "127.0.0.1:9123")
                    .header(header::COOKIE, "marula_installer=session-cookie")
                    .header("x-marula-installer", "1")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("a response");
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            allowed
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let _ = std::fs::remove_dir_all(&ui);
    }

    #[tokio::test]
    async fn a_traversal_attempt_reads_nothing_and_a_real_asset_is_served() {
        let ui = ui_dir();
        let router = router(app(&ui));

        for path in [
            "/assets/../index.html",
            "/assets/..%2Findex.html",
            "/assets/%2e%2e/x",
        ] {
            let response = router.clone().oneshot(get(path)).await.expect("a response");
            assert!(
                response.status() == StatusCode::BAD_REQUEST
                    || response.status() == StatusCode::NOT_FOUND,
                "{path} answered {}",
                response.status()
            );
            let body = body_text(response).await;
            assert!(!body.contains("id=root"), "{path} leaked the index page");
        }

        let asset = router
            .oneshot(get("/assets/app.js"))
            .await
            .expect("a response");
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(
            asset
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        let _ = std::fs::remove_dir_all(&ui);
    }

    #[tokio::test]
    async fn every_response_carries_a_csp_that_allows_nothing_off_this_machine() {
        let ui = ui_dir();
        let router = router(app(&ui));
        let response = router.oneshot(get("/")).await.expect("a response");
        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(csp.contains("default-src 'self'"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
        assert!(!csp.contains("http://"), "{csp}");
        let _ = std::fs::remove_dir_all(&ui);
    }

    #[test]
    fn a_token_comparison_does_not_leak_through_its_shape() {
        assert!(constant_time_eq("abcd", "abcd"));
        assert!(!constant_time_eq("abcd", "abce"));
        assert!(!constant_time_eq("abcd", "abcde"));
        assert!(!constant_time_eq("", "a"));
    }
}
