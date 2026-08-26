//! `ScanProvider` — the malware-scan boundary between inbound webhook
//! receipt and persistence (§2 email row). A verdict gates whether an
//! inbound message may create tickets/comments.
//!
//! Non-purpose: no quarantine storage and no raw-file handling — v1 scans
//! the extracted text payload only; full attachment scanning arrives with
//! Stage-3 object storage.

use async_trait::async_trait;

/// Verdict for one scanned message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanVerdict {
    pub infected: bool,
    /// Scanner detail string (e.g. signature name) when available.
    pub detail: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// Scanner unreachable — callers must fail closed (persist as infected
    /// or defer), never treat unknown as clean.
    #[error("scanner unavailable: {0}")]
    Unavailable(String),
}

/// The single seam for malware scanning. Implementations: clamd TCP,
/// noop none-mode double.
#[async_trait]
pub trait ScanProvider: Send + Sync {
    /// Adapter name recorded on the message ledger (`clamd`, `none`, …).
    fn name(&self) -> &'static str;

    async fn scan(&self, body_text: &str) -> Result<ScanVerdict, ScanError>;
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
            detail: Some("scan provider disabled (none mode)".to_string()),
        })
    }
}

/// ClamAV clamd TCP adapter speaking the INSTREAM protocol: `zINSTREAM\0`
/// then length-prefixed chunks terminated by a zero-length chunk; the
/// reply is `stream: OK\0` or `stream: <signature> FOUND\0`.
#[derive(Debug, Clone)]
pub struct ClamdScan {
    pub addr: String,
}

const CHUNK_SIZE: usize = 4096;
const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

impl ClamdScan {
    #[must_use]
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }
}

#[async_trait]
impl ScanProvider for ClamdScan {
    fn name(&self) -> &'static str {
        "clamd"
    }

    async fn scan(&self, body_text: &str) -> Result<ScanVerdict, ScanError> {
        let mut stream =
            tokio::time::timeout(RESPONSE_TIMEOUT, tokio::net::TcpStream::connect(&self.addr))
                .await
                .map_err(|_| ScanError::Unavailable("connect timeout".into()))?
                .map_err(|e| ScanError::Unavailable(format!("{}: {e}", self.addr)))?;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"zINSTREAM\0")
            .await
            .map_err(|e| ScanError::Unavailable(format!("write: {e}")))?;
        let bytes = body_text.as_bytes();
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
        tokio::time::timeout(RESPONSE_TIMEOUT, stream.read_to_end(&mut response))
            .await
            .map_err(|_| ScanError::Unavailable("response timeout".into()))?
            .map_err(|e| ScanError::Unavailable(format!("read: {e}")))?;
        let text = String::from_utf8_lossy(&response);
        let text = text.trim_end_matches('\0').trim();
        if text.ends_with("FOUND") {
            Ok(ScanVerdict {
                infected: true,
                detail: Some(text.to_string()),
            })
        } else if text.ends_with("OK") {
            Ok(ScanVerdict {
                infected: false,
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
}
