//! `ScanProvider` — the malware-scan boundary in front of anything Marula
//! persists from outside (§2 email row, §2 storage row). A verdict gates
//! whether an inbound message may create tickets/comments, and whether an
//! uploaded file may be admitted to the object store at all.
//!
//! Two legs, one verdict type: [`ScanProvider::scan`] takes an extracted
//! text payload (the inbound-email path), [`ScanProvider::scan_bytes`]
//! takes the raw octets an upload carries. The second is what binary
//! attachments need — a lossy UTF-8 view of a `.docx` is not the file
//! clamd has to see, and a signature in the bytes past the first
//! replacement character would be invisible to the text leg.
//!
//! Non-purpose: no quarantine storage (core-api owns where rejected bytes
//! go) and no size policy (the caller's cap applies before this runs).

use async_trait::async_trait;

/// Verdict for one scanned payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanVerdict {
    pub infected: bool,
    /// No scanner examined these bytes: the documented none mode. A skip is
    /// NOT a clean bill of health, and the three verdicts stay distinct all
    /// the way to the record — `skipped` says "nobody looked", `clean` says
    /// "a scanner looked and passed it".
    pub skipped: bool,
    /// Scanner detail string (e.g. signature name) when available.
    pub detail: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// Scanner unreachable — callers must fail closed (persist as infected
    /// or defer), never treat unknown as clean.
    #[error("scanner unavailable: {0}")]
    Unavailable(String),
    /// The body is past the size the scanner will stream. Distinct from
    /// `Unavailable` on purpose: a caller can tell the user "too large to
    /// scan" instead of blaming a scanner that is working. Before this, the
    /// daemon's own `INSTREAM size limit exceeded` reply arrived as
    /// `Unavailable`, so a body the app had accepted looked like an outage
    /// (Wave 62 lane `p62-attachment-verdicts`).
    #[error("body is {len_bytes} bytes, past the scanner's {limit_bytes}-byte stream limit")]
    TooLarge {
        /// The configured stream cap.
        limit_bytes: u64,
        /// What the caller offered.
        len_bytes: u64,
    },
}

/// The single seam for malware scanning. Implementations: clamd TCP,
/// noop none-mode double.
#[async_trait]
pub trait ScanProvider: Send + Sync {
    /// Adapter name recorded on the message ledger (`clamd`, `none`, …).
    fn name(&self) -> &'static str;

    async fn scan(&self, body_text: &str) -> Result<ScanVerdict, ScanError>;

    /// Scans raw octets. The default implementation falls back to the text
    /// leg on a lossy UTF-8 view, which is right for text-only providers
    /// (`none`) and WRONG for anything that inspects binary structure —
    /// providers that can see bytes must override it. `clamd` does, because
    /// it takes the payload as octets over INSTREAM and never needed a
    /// string in the first place.
    async fn scan_bytes(&self, bytes: &[u8]) -> Result<ScanVerdict, ScanError> {
        self.scan(&String::from_utf8_lossy(bytes)).await
    }
}

/// The documented none mode: reports clean with scan_status recorded as a
/// skip so ledgers distinguish "scanned clean" from "no scanner present".
#[derive(Default)]
pub struct NoopScan;

impl NoopScan {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ScanProvider for NoopScan {
    fn name(&self) -> &'static str {
        "none"
    }

    async fn scan(&self, _body_text: &str) -> Result<ScanVerdict, ScanError> {
        Ok(ScanVerdict {
            infected: false,
            skipped: true,
            detail: Some("scan provider disabled (none mode)".to_string()),
        })
    }
}

/// ClamAV clamd TCP adapter speaking the INSTREAM protocol: `zINSTREAM\0`
/// then length-prefixed chunks terminated by a zero-length chunk; the
/// reply is `stream: OK\0` or `stream: <signature> FOUND\0`.
///
/// A third reply exists and is not a verdict: a stream past the daemon's
/// `StreamMaxLength` comes back as `INSTREAM size limit exceeded. ERROR`.
/// It ends in neither `OK` nor `FOUND`, so it fails closed to
/// [`ScanError::Unavailable`] and the gate refuses the upload. Those bytes
/// are refused rather than scanned, which is the safe direction and is not
/// the same thing as a clean scan. The live leg in
/// `tests/clamd_live.rs` pins the daemon's actual words.
#[derive(Debug, Clone)]
pub struct ClamdScan {
    /// The cap the adapter refuses above. It is the app's own upload cap by
    /// default; an operator whose daemon streams less must say so with
    /// `with_max_stream_bytes`, because sending a body the daemon will refuse
    /// only produces a misleading `Unavailable`.
    max_stream_bytes: u64,
    pub addr: String,
    /// Whole-exchange deadline. See [`RESPONSE_TIMEOUT`]; [`Self::with_timeout`]
    /// exists for tests and for a deployment whose scanner is remote.
    timeout: std::time::Duration,
}

const CHUNK_SIZE: usize = 4096;

/// One deadline for the WHOLE exchange: connect, every chunk write, and the
/// reply read share it.
///
/// Bounding only the ends is not a bound. The writes were outside it, so a
/// clamd that accepted the connection and stopped reading filled the socket
/// buffer, `write_all` never returned, and the upload task parked until the
/// client gave up. `crates/core-api` has no request-level `TimeoutLayer` to
/// catch that either, so the bound has to live here.
const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The stock `StreamMaxLength` a clamd ships with. The app's upload cap is the
/// real number; this is the floor the adapter assumes when nobody says
/// otherwise, and it matches `core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES`.
const DEFAULT_STREAM_MAX_BYTES: u64 = 25 * 1024 * 1024;

impl ClamdScan {
    #[must_use]
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            timeout: RESPONSE_TIMEOUT,
            max_stream_bytes: DEFAULT_STREAM_MAX_BYTES,
        }
    }

    /// Same adapter, different whole-exchange deadline.
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The size this scanner will stream. Set it from the deployment's upload
    /// cap; the daemon's `StreamMaxLength` must be at least this value.
    #[must_use]
    pub fn with_max_stream_bytes(mut self, max_stream_bytes: u64) -> Self {
        self.max_stream_bytes = max_stream_bytes;
        self
    }
}

#[async_trait]
impl ScanProvider for ClamdScan {
    fn name(&self) -> &'static str {
        "clamd"
    }

    async fn scan(&self, body_text: &str) -> Result<ScanVerdict, ScanError> {
        self.scan_bytes(body_text.as_bytes()).await
    }

    async fn scan_bytes(&self, bytes: &[u8]) -> Result<ScanVerdict, ScanError> {
        if bytes.len() as u64 > self.max_stream_bytes {
            return Err(ScanError::TooLarge {
                limit_bytes: self.max_stream_bytes,
                len_bytes: bytes.len() as u64,
            });
        }
        // ONE deadline over the whole exchange. See `RESPONSE_TIMEOUT`.
        tokio::time::timeout(self.timeout, self.exchange(bytes))
            .await
            .unwrap_or_else(|_| {
                Err(ScanError::Unavailable(format!(
                    "no verdict within {:?}: connect, stream writes and reply share one deadline",
                    self.timeout
                )))
            })
    }
}

impl ClamdScan {
    /// Connect, ship the INSTREAM frame, read the reply. Unbounded on its
    /// own: [`ScanProvider::scan_bytes`] is the only caller and wraps it.
    async fn exchange(&self, bytes: &[u8]) -> Result<ScanVerdict, ScanError> {
        let mut stream = tokio::net::TcpStream::connect(&self.addr)
            .await
            .map_err(|e| ScanError::Unavailable(format!("{}: {e}", self.addr)))?;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"zINSTREAM\0")
            .await
            .map_err(|e| ScanError::Unavailable(format!("write: {e}")))?;
        for chunk in bytes.chunks(CHUNK_SIZE) {
            stream
                .write_all(&(chunk.len() as u32).to_be_bytes())
                .await
                .map_err(|e| ScanError::Unavailable(format!("write: {e}")))?;
            stream
                .write_all(chunk)
                .await
                .map_err(|e| ScanError::Unavailable(format!("write: {e}")))?;
        }
        stream
            .write_all(&0_u32.to_be_bytes())
            .await
            .map_err(|e| ScanError::Unavailable(format!("write: {e}")))?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|e| ScanError::Unavailable(format!("read: {e}")))?;
        let text = String::from_utf8_lossy(&response);
        let text = text.trim_end_matches('\0').trim();
        if text.ends_with("FOUND") {
            Ok(ScanVerdict {
                infected: true,
                skipped: false,
                detail: Some(text.to_string()),
            })
        } else if text.ends_with("OK") {
            Ok(ScanVerdict {
                infected: false,
                skipped: false,
                detail: None,
            })
        } else {
            Err(ScanError::Unavailable(format!(
                "unexpected clamd reply: {text}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_reports_clean_with_none_mode_detail() {
        let verdict = NoopScan::new().scan("anything").await.expect("clean");
        assert!(!verdict.infected);
        // None mode is a SKIP, not a clean bill of health: the record has to
        // be able to say nobody looked.
        assert!(verdict.skipped);
        assert!(verdict.detail.unwrap().contains("none mode"));
    }

    /// Fake clamd asserting INSTREAM framing, replying FOUND.
    #[tokio::test]
    async fn clamd_infected_verdict_from_instream_reply() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 10];
            sock.read_exact(&mut header).await.unwrap();
            assert_eq!(&header, b"zINSTREAM\0");
            // One chunk of 5 bytes + terminator.
            let mut len_buf = [0u8; 4];
            sock.read_exact(&mut len_buf).await.unwrap();
            assert_eq!(u32::from_be_bytes(len_buf), 5);
            let mut payload = [0u8; 5];
            sock.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"hello");
            sock.read_exact(&mut len_buf).await.unwrap();
            assert_eq!(u32::from_be_bytes(len_buf), 0);
            sock.write_all(b"stream: Eicar-Test-Signature FOUND\0")
                .await
                .unwrap();
        });

        let scanner = ClamdScan::new(addr.to_string());
        let verdict = scanner.scan("hello").await.expect("verdict");
        server.await.unwrap();
        assert!(verdict.infected);
        assert!(verdict.detail.unwrap().contains("Eicar"));
    }

    #[tokio::test]
    async fn unreachable_clamd_fails_closed_as_unavailable() {
        let scanner = ClamdScan::new("127.0.0.1:1");
        let err = scanner.scan("x").await.expect_err("unavailable");
        assert!(matches!(err, ScanError::Unavailable(_)));
    }

    /// A daemon that accepts the connection and then stops reading must not
    /// park the caller. The socket buffer fills, `write_all` blocks, and only
    /// the deadline returns control. Before the whole exchange shared one
    /// deadline, this test never finished: the connect and the final
    /// `read_to_end` were bounded and the writes between them were not.
    ///
    /// The timeout is shortened so the test stays fast, and the payload is
    /// larger than any socket buffer so the write genuinely blocks rather
    /// than fitting in the kernel's send queue.
    #[tokio::test]
    async fn stalled_clamd_returns_by_the_deadline_instead_of_parking() {
        use std::time::{Duration, Instant};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 10];
            sock.read_exact(&mut header).await.unwrap();
            // Read the first length prefix, then never read again and never
            // reply: the connection stays open and the peer goes quiet,
            // which is the shape of a clamd that has stopped draining.
            let mut len = [0u8; 4];
            sock.read_exact(&mut len).await.unwrap();
            std::future::pending::<()>().await;
        });

        let scanner = ClamdScan::new(addr.to_string()).with_timeout(Duration::from_millis(250));
        let payload = vec![0u8; 8 * 1024 * 1024];
        let started = Instant::now();
        let err = scanner
            .scan_bytes(&payload)
            .await
            .expect_err("a silent daemon is no verdict");
        let elapsed = started.elapsed();
        server.abort();

        assert!(matches!(err, ScanError::Unavailable(_)), "{err:?}");
        assert!(
            elapsed >= Duration::from_millis(200),
            "the deadline is what returned: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "bounded, not parked: {elapsed:?}"
        );
    }

    /// The adapter refuses a body past its own cap WITHOUT connecting, so the
    /// caller hears "too large to scan" instead of a scanner outage. The
    /// pointer below is a closed port: a connect attempt would surface as
    /// `Unavailable`, which is the answer this test exists to rule out.
    #[tokio::test]
    async fn a_body_past_the_stream_cap_is_too_large_not_unavailable() {
        let scanner = ClamdScan::new("127.0.0.1:9").with_max_stream_bytes(16);
        let err = scanner
            .scan_bytes(&[b'x'; 17])
            .await
            .expect_err("must refuse");
        match err {
            ScanError::TooLarge {
                limit_bytes,
                len_bytes,
            } => {
                assert_eq!(limit_bytes, 16);
                assert_eq!(len_bytes, 17);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    /// A real clamd answers a stream past `StreamMaxLength` with
    /// `INSTREAM size limit exceeded. ERROR` and closes the connection. That
    /// reply ends in neither `OK` nor `FOUND`, so the adapter fails closed —
    /// those bytes are refused, not reported clean. The daemon's exact bytes
    /// are pinned by the live leg (`tests/clamd_live.rs`); this test pins the
    /// branch that has to keep failing closed.
    #[tokio::test]
    async fn stream_limit_error_reply_is_unavailable_never_clean() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 10];
            sock.read_exact(&mut header).await.unwrap();
            // Drain the frame up to the zero-length terminator, then answer
            // the way the daemon does when the limit trips.
            loop {
                let mut len_buf = [0u8; 4];
                sock.read_exact(&mut len_buf).await.unwrap();
                let len = u32::from_be_bytes(len_buf) as usize;
                if len == 0 {
                    break;
                }
                let mut chunk = vec![0u8; len];
                sock.read_exact(&mut chunk).await.unwrap();
            }
            sock.write_all(b"INSTREAM size limit exceeded. ERROR\0")
                .await
                .unwrap();
        });

        let scanner = ClamdScan::new(addr.to_string());
        let verdict = scanner.scan_bytes(b"%PDF-1.7 oversized").await;
        server.await.unwrap();

        let err = verdict.expect_err("a size-limit reply is not a verdict");
        match err {
            ScanError::Unavailable(detail) => {
                assert!(detail.contains("size limit"), "{detail}");
            }
            ScanError::TooLarge { .. } => {
                panic!("the adapter's own cap did not trip; this is the daemon's reply")
            }
        }
    }

    /// `scan_bytes` has to ship the frame clamd expects for a payload bigger
    /// than one chunk: length-prefixed 4096-byte pieces, then a zero-length
    /// terminator. A single-chunk-only framing would pass every small test and
    /// split real attachments wrong.
    #[tokio::test]
    async fn large_payload_is_split_into_length_prefixed_chunks() {
        let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let expected = payload.clone();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 10];
            sock.read_exact(&mut header).await.unwrap();
            assert_eq!(&header, b"zINSTREAM\0");
            let mut assembled = Vec::new();
            let mut chunk_sizes = Vec::new();
            loop {
                let mut len_buf = [0u8; 4];
                sock.read_exact(&mut len_buf).await.unwrap();
                let len = u32::from_be_bytes(len_buf) as usize;
                if len == 0 {
                    break;
                }
                assert!(
                    len <= CHUNK_SIZE,
                    "chunk of {len} bytes exceeds the chunk size"
                );
                let mut chunk = vec![0u8; len];
                sock.read_exact(&mut chunk).await.unwrap();
                chunk_sizes.push(len);
                assembled.extend_from_slice(&chunk);
            }
            assert_eq!(assembled, expected, "every octet arrives, in order");
            assert_eq!(chunk_sizes, vec![4096, 4096, 1808]);
            sock.write_all(b"stream: OK\0").await.unwrap();
        });

        let scanner = ClamdScan::new(addr.to_string());
        let verdict = scanner.scan_bytes(&payload).await.expect("verdict");
        server.await.unwrap();
        assert!(!verdict.infected);
    }

    /// The bytes leg must ship the OCTETS. An attachment is binary, and the
    /// text leg would hand clamd a lossy UTF-8 view of it: every byte that is
    /// not valid UTF-8 becomes U+FFFD, so a signature living in the binary
    /// region is exactly what stops matching. The fake server asserts the
    /// bytes arrive unchanged.
    #[tokio::test]
    async fn clamd_bytes_leg_ships_octets_not_a_lossy_view() {
        // Header that a text conversion would mangle: invalid UTF-8, NULs,
        // and a high byte.
        let payload: Vec<u8> = vec![0x50, 0x4B, 0x03, 0x04, 0x00, 0xFF, 0xFE, 0x00, 0xC0];

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let expected = payload.clone();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 10];
            sock.read_exact(&mut header).await.unwrap();
            assert_eq!(&header, b"zINSTREAM\0");
            let mut len_buf = [0u8; 4];
            sock.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut chunk = vec![0u8; len];
            sock.read_exact(&mut chunk).await.unwrap();
            assert_eq!(chunk, expected, "clamd must receive the raw bytes");
            sock.read_exact(&mut len_buf).await.unwrap();
            assert_eq!(u32::from_be_bytes(len_buf), 0);
            sock.write_all(b"stream: OK\0").await.unwrap();
        });

        let scanner = ClamdScan::new(addr.to_string());
        let verdict = scanner.scan_bytes(&payload).await.expect("verdict");
        server.await.unwrap();
        assert!(!verdict.infected);
        assert!(!verdict.skipped);
    }
}
