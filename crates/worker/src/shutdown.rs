//! One shutdown path for both binaries: signal → refuse new work → drain
//! what is in flight → exit, bounded by a grace window.
//!
//! Purpose: a container runtime stops a process with SIGTERM and kills it
//! after `terminationGracePeriodSeconds`. A process that waits on `ctrl_c`
//! only (SIGINT) never sees that signal, so a rolling deploy is a hard kill:
//! the listener vanishes, in-flight requests die mid-write, and the worker
//! abandons whatever its loops were holding.
//!
//! Contract, identical for `core-api` and `worker`:
//!
//! - SIGINT or SIGTERM starts the drain; whichever arrives first wins.
//!   SIGINT is the developer path (Ctrl-C), SIGTERM is the orchestrator's.
//! - `MARULA_SHUTDOWN_GRACE_SECS` (default [`DEFAULT_GRACE_SECS`], clamped to
//!   [`MIN_GRACE_SECS`]..=[`MAX_GRACE_SECS`]) bounds how long in-flight work
//!   may take. The listener stops accepting immediately, so requests that
//!   arrive after the signal are refused rather than queued behind a
//!   draining process.
//! - When the window elapses the process exits anyway and logs what it cut.
//!   An overrun is a bug to read from that line, never a hung container.
//!
//! What counts as in-flight: a request the process has accepted and not yet
//! answered ([`Tracker`]). Event streams are the documented exception — they
//! are answered by closing, and a client reconnects and resyncs from its
//! snapshot (`core_api::realtime` module docs), so a stream that outlives
//! the window is cut, not a lost write.

use std::future::IntoFuture;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::watch;

/// Grace window when the environment says nothing.
pub const DEFAULT_GRACE_SECS: u64 = 20;
/// A window under a second cannot finish an in-flight request that has already
/// started; treat it as a configuration mistake, not a choice.
pub const MIN_GRACE_SECS: u64 = 1;
/// Upper clamp: past this the orchestrator's own grace period is the real
/// bound, and a longer window only delays the kill.
pub const MAX_GRACE_SECS: u64 = 600;
/// Headroom between the drain window closing and the hard stop, so the
/// watchdog's error line reaches the log before the process is gone.
pub const HARD_STOP_MARGIN_SECS: u64 = 5;

/// Environment key bounding the drain.
pub const GRACE_ENV: &str = "MARULA_SHUTDOWN_GRACE_SECS";

/// Which signal asked for the stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Ctrl-C, or a `docker stop` on a terminal.
    Interrupt,
    /// The orchestrator's stop: Kubernetes, `docker stop`, systemd.
    Terminate,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Interrupt => "SIGINT",
            Reason::Terminate => "SIGTERM",
        }
    }
}

/// The bounded drain window, from the environment.
pub fn grace_window() -> Duration {
    grace_window_with(|key| std::env::var(key).ok())
}

/// Lookup-injectable form of [`grace_window`], for tests.
pub fn grace_window_with(env: impl Fn(&str) -> Option<String>) -> Duration {
    let configured = env(GRACE_ENV)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_GRACE_SECS);
    let resolved = configured.clamp(MIN_GRACE_SECS, MAX_GRACE_SECS);
    if resolved != configured {
        tracing::warn!(
            env = GRACE_ENV,
            configured,
            resolved,
            "shutdown grace window out of range; clamped"
        );
    }
    Duration::from_secs(resolved)
}

/// Resolves when a stop signal arrives, naming which one.
#[cfg(unix)]
pub async fn signal() -> Reason {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(
                %error,
                "could not install the SIGTERM handler; this process stops on SIGINT only"
            );
            return interrupt().await;
        }
    };
    let mut interrupt = match signal(SignalKind::interrupt()) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(%error, "could not install the SIGINT handler");
            let _ = terminate.recv().await;
            return Reason::Terminate;
        }
    };
    tokio::select! {
        _ = interrupt.recv() => Reason::Interrupt,
        _ = terminate.recv() => Reason::Terminate,
    }
}

#[cfg(not(unix))]
pub async fn signal() -> Reason {
    // Windows has no SIGTERM to translate; the console interrupt is the
    // only stop a supervisor can deliver here.
    let _ = tokio::signal::ctrl_c().await;
    Reason::Interrupt
}

async fn interrupt() -> Reason {
    let _ = tokio::signal::ctrl_c().await;
    Reason::Interrupt
}

/// Installs the stop handler and returns the flag every half of the process
/// watches. Also starts the hard stop: if the drain overruns the window, the
/// process exits and says so instead of waiting for the orchestrator's kill.
pub fn install() -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    let window = grace_window();
    tokio::spawn(async move {
        let reason = signal().await;
        tracing::info!(
            signal = reason.as_str(),
            grace_secs = window.as_secs_f64(),
            env = GRACE_ENV,
            "stop signal received: refusing new work, draining in-flight work"
        );
        let _ = sender.send(true);
    });
    hard_stop_after(receiver.clone(), window);
    receiver
}

/// The process-level floor under the drain: a loop that ignores the flag, or a
/// `Runtime::drop` waiting on a blocking task, must not outlive the window.
fn hard_stop_after(shutdown: watch::Receiver<bool>, window: Duration) {
    std::thread::spawn(move || {
        while !*shutdown.borrow() {
            std::thread::sleep(Duration::from_millis(200));
            if shutdown.has_changed().is_err() {
                return; // sender gone: the process is already on its way out
            }
        }
        std::thread::sleep(window + Duration::from_secs(HARD_STOP_MARGIN_SECS));
        tracing::error!(
            grace_secs = window.as_secs_f64(),
            "shutdown grace window exceeded; exiting now — in-flight work is cut"
        );
        std::process::exit(0);
    });
}

/// Resolves when the shared flag flips true, or the sender goes away.
pub async fn wait(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// Waits for an in-process task to finish, bounded by `window`. Returns `false`
/// when it was still running at the deadline.
///
/// All-in-one hosting needs this: the API drains its requests and the embedded
/// worker drains its loops against the same flag, so exiting the moment the
/// request drain ends would cut the worker's work at the API's pace.
pub async fn wait_for_task(handle: tokio::task::JoinHandle<()>, window: Duration) -> bool {
    match tokio::time::timeout(window, handle).await {
        Ok(_) => true,
        Err(_) => {
            tracing::warn!(
                window_secs = window.as_secs_f64(),
                "in-process task still running at the drain deadline; exiting"
            );
            false
        }
    }
}

/// What a drain finished and what it cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainReport {
    pub window: Duration,
    /// Units that finished inside the window (`before` minus `cut`).
    pub finished: Vec<&'static str>,
    /// Units still running when the window closed.
    pub cut: Vec<&'static str>,
}

impl DrainReport {
    pub fn new(window: Duration, before: &[&'static str], cut: Vec<&'static str>) -> Self {
        let mut remaining = cut.clone();
        let mut finished = Vec::new();
        for name in before {
            if let Some(index) = remaining.iter().position(|n| n == name) {
                remaining.remove(index);
            } else {
                finished.push(*name);
            }
        }
        Self {
            window,
            finished,
            cut,
        }
    }

    /// A drain with nothing cut.
    pub fn clean(&self) -> bool {
        self.cut.is_empty()
    }
}

/// The one log line: what drained, what was cut, and over what window.
pub fn log_drain(report: &DrainReport, service: &str) {
    if report.clean() {
        tracing::info!(
            service,
            window_secs = report.window.as_secs_f64(),
            finished = ?report.finished,
            "drain complete: every in-flight unit finished inside the window"
        );
    } else {
        tracing::warn!(
            service,
            window_secs = report.window.as_secs_f64(),
            finished = ?report.finished,
            cut = ?report.cut,
            "drain window elapsed: cutting the units still running"
        );
    }
}

/// Polls `running` until it reports nothing or `window` elapses, returning the
/// last non-empty answer.
pub async fn wait_until_idle(
    window: Duration,
    running: impl Fn() -> Vec<&'static str>,
) -> Vec<&'static str> {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let now = running();
        if now.is_empty() || tokio::time::Instant::now() >= deadline {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// In-flight accounting for work a process owes an answer for.
///
/// A [`Ticket`] is removed by its `Drop`, so a cancelled or panicked request
/// stops counting the same way a finished one does.
pub struct Tracker {
    closing: AtomicBool,
    in_flight: Mutex<Vec<&'static str>>,
}

impl Tracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            closing: AtomicBool::new(false),
            in_flight: Mutex::new(Vec::new()),
        })
    }

    /// Starts a unit of work, or `None` once the process is closing: that is
    /// the "refuse new work" half of the contract.
    pub fn try_begin(self: &Arc<Self>, name: &'static str) -> Option<Ticket> {
        if self.closing.load(Ordering::SeqCst) {
            return None;
        }
        self.in_flight
            .lock()
            .expect("in-flight list poisoned")
            .push(name);
        // Re-check after the push: a close that raced this push would
        // otherwise wait on a unit it already decided to refuse.
        if self.closing.load(Ordering::SeqCst) {
            self.end(name);
            return None;
        }
        Some(Ticket {
            tracker: Arc::clone(self),
            name,
        })
    }

    fn end(&self, name: &'static str) {
        let mut in_flight = self.in_flight.lock().expect("in-flight list poisoned");
        if let Some(index) = in_flight.iter().position(|n| *n == name) {
            in_flight.remove(index);
        }
    }

    /// Stops accepting: every later [`Tracker::try_begin`] answers `None`.
    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
    }

    pub fn closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    /// Names of the units still owed an answer, with repeats.
    pub fn in_flight(&self) -> Vec<&'static str> {
        self.in_flight
            .lock()
            .expect("in-flight list poisoned")
            .clone()
    }

    /// Waits up to `window` for the in-flight units to finish.
    pub async fn drain(self: &Arc<Self>, window: Duration) -> DrainReport {
        let before = self.in_flight();
        let cut = wait_until_idle(window, || self.in_flight()).await;
        DrainReport::new(window, &before, cut)
    }
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            closing: AtomicBool::new(false),
            in_flight: Mutex::new(Vec::new()),
        }
    }
}

/// One unit of tracked work. Dropping it ends the unit.
pub struct Ticket {
    tracker: Arc<Tracker>,
    name: &'static str,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.tracker.end(self.name);
    }
}

/// Axum middleware: refuse new work once the process is closing, otherwise
/// count the request as in-flight until its response is complete.
///
/// Layered OUTSIDE the API's own envelope middleware on purpose — a process
/// that is stopping is not answering under the API contract, and a client
/// reads the 503 with `Retry-After` as "go to another replica".
pub async fn track_and_refuse(
    State(tracker): State<Arc<Tracker>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(_ticket) = tracker.try_begin("request") else {
        let mut response = (
            StatusCode::SERVICE_UNAVAILABLE,
            "core-api is shutting down; retry against another replica",
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        return response;
    };
    next.run(request).await
}

/// Serves until `shutdown` fires, then gives in-flight requests `window` to
/// finish. The caller applies `.with_graceful_shutdown(wait(shutdown))` to the
/// server it passes in, so the listener closes at the signal and the serve
/// future keeps driving the accepted requests to completion.
pub async fn serve_and_drain<F>(
    service: &'static str,
    serve: F,
    shutdown: watch::Receiver<bool>,
    tracker: Arc<Tracker>,
    window: Duration,
) -> std::io::Result<DrainReport>
where
    F: IntoFuture<Output = std::io::Result<()>>,
{
    let draining = {
        let tracker = Arc::clone(&tracker);
        async move {
            wait(shutdown).await;
            tracker.close();
            tracker.drain(window).await
        }
    };
    let serving = serve.into_future();
    tokio::pin!(serving);
    tokio::pin!(draining);
    tokio::select! {
        result = &mut serving => {
            // The server ended on its own (bind or accept failure): nothing
            // to drain, and the caller reports the error.
            result.map(|()| DrainReport::new(window, &[], Vec::new()))
        }
        report = &mut draining => {
            log_drain(&report, service);
            Ok(report)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn grace_window_defaults_and_clamps() {
        let empty: HashMap<&str, String> = HashMap::new();
        let window = grace_window_with(|k| empty.get(k).cloned());
        assert_eq!(window, Duration::from_secs(DEFAULT_GRACE_SECS));

        let mut set = HashMap::new();
        set.insert(GRACE_ENV, "7".to_string());
        assert_eq!(
            grace_window_with(|k| set.get(k).cloned()),
            Duration::from_secs(7)
        );

        set.insert(GRACE_ENV, "0".to_string());
        assert_eq!(
            grace_window_with(|k| set.get(k).cloned()),
            Duration::from_secs(MIN_GRACE_SECS)
        );

        set.insert(GRACE_ENV, "99999".to_string());
        assert_eq!(
            grace_window_with(|k| set.get(k).cloned()),
            Duration::from_secs(MAX_GRACE_SECS)
        );

        // A value nobody can parse falls back to the default rather than a
        // zero window that would cut every request at the signal.
        set.insert(GRACE_ENV, "soon".to_string());
        assert_eq!(
            grace_window_with(|k| set.get(k).cloned()),
            Duration::from_secs(DEFAULT_GRACE_SECS)
        );
    }

    /// The signal a container runtime sends starts the drain. Raising SIGTERM
    /// at this process is safe: the handler is installed before the raise, so
    /// the default "terminate" disposition never applies.
    #[cfg(unix)]
    #[tokio::test]
    async fn sigterm_is_a_stop_signal() {
        let raised = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let pid = std::process::id();
            std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()
                .expect("raise SIGTERM");
        });
        let reason = tokio::time::timeout(Duration::from_secs(5), signal())
            .await
            .expect("SIGTERM must resolve the stop signal");
        raised.await.expect("raiser");
        assert_eq!(reason, Reason::Terminate);
        assert_eq!(reason.as_str(), "SIGTERM");
    }

    #[tokio::test]
    async fn drain_finishes_in_flight_work_and_refuses_new_work() {
        let tracker = Tracker::new();
        let ticket = tracker.try_begin("request").expect("first unit");
        let drainer = {
            let tracker = Arc::clone(&tracker);
            tokio::spawn(async move {
                tracker.close();
                tracker.drain(Duration::from_secs(2)).await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(tracker.closing(), "closing refuses new work");
        assert!(
            tracker.try_begin("request").is_none(),
            "a unit started after the close must be refused"
        );
        drop(ticket);
        let report = drainer.await.expect("drain");
        assert!(report.clean());
        assert_eq!(report.finished, vec!["request"]);
    }

    #[tokio::test]
    async fn drain_cuts_after_the_window() {
        let tracker = Tracker::new();
        let _stuck = tracker.try_begin("request").expect("unit");
        tracker.close();
        let report = tracker.drain(Duration::from_millis(80)).await;
        assert!(!report.clean());
        assert_eq!(report.cut, vec!["request"]);
        assert!(report.finished.is_empty());
    }

    /// The server leg: an accepted request finishes after the stop, and a
    /// connection opened after it is refused.
    #[tokio::test]
    async fn server_finishes_an_accepted_request_and_refuses_new_ones() {
        use axum::routing::get;
        use axum::Router;

        let tracker = Tracker::new();
        let app = Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    "finished"
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                Arc::clone(&tracker),
                track_and_refuse,
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (sender, receiver) = watch::channel(false);
        let serving = axum::serve(listener, app).with_graceful_shutdown(wait(receiver.clone()));
        let server = tokio::spawn(serve_and_drain(
            "test",
            serving,
            receiver,
            Arc::clone(&tracker),
            Duration::from_secs(5),
        ));

        // Request A is accepted before the stop and must be answered.
        let first = tokio::spawn(async move { http_get(addr, "/slow").await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(tracker.in_flight().len(), 1, "request A is in flight");
        sender.send(true).expect("signal");

        let body = tokio::time::timeout(Duration::from_secs(3), first)
            .await
            .expect("request A must not be cut")
            .expect("join")
            .expect("request A answered");
        assert!(body.contains("finished"), "got {body:?}");

        // Request B opens after the stop: the listener is closed, so either
        // the connect fails or the request is refused with 503.
        match http_get(addr, "/slow").await {
            Err(_) => {}
            Ok(response) => assert!(
                response.contains("503"),
                "a request after the stop must be refused, got {response:?}"
            ),
        }

        let report = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("server exit")
            .expect("join")
            .expect("io");
        assert!(report.clean(), "request A finished inside the window");
    }

    /// A minimal HTTP/1.1 GET: the test needs a real socket, not a router call,
    /// because the drain is about connections the server accepted.
    async fn http_get(addr: std::net::SocketAddr, path: &str) -> Result<String, String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|e| e.to_string())?;
        let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&response).to_string())
    }
}
