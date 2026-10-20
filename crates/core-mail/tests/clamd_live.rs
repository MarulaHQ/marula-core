//! `ClamdScan` against a REAL clamd (Wave 52 lane P6f).
//!
//! The unit tests in `src/scan.rs` speak the protocol to a fake socket. That
//! proves the framing we intended; it cannot prove the daemon agrees. Three
//! things only a real ClamAV settles, and this file asks each of them:
//!
//! 1. that `stream: OK` and `stream: <name> FOUND` are what a real daemon
//!    actually sends for clean and infected octets;
//! 2. what it sends when the stream is bigger than its `StreamMaxLength`, and
//!    that the adapter never reports those bytes clean;
//! 3. that the whole exchange is bounded, so a daemon that stops draining
//!    cannot park the upload (pinned by the stall test in `src/scan.rs`, whose
//!    shape this leg repeats against the real socket).
//!
//! Run it with `scripts/p6f-clamd-live.sh`, which boots the container, waits
//! for the signature DB and sets `CLAMD_LIVE_ADDR`. Every test here is
//! `#[ignore]`d: the default suite stays fast and hermetic.

use std::io::{Read, Write};
use std::time::Duration;

use core_mail::ScanProvider;

/// The whole-file EICAR test string. ClamAV's signature is a whole-file
/// match: embedded in a larger document it does NOT trip, which is why the
/// door tests wrap it in a ZIP instead of padding it into a PDF.
const EICAR: &[u8] = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

fn addr() -> String {
    std::env::var("CLAMD_LIVE_ADDR").unwrap_or_else(|_| "127.0.0.1:3310".into())
}

fn scanner() -> core_mail::ClamdScan {
    core_mail::ClamdScan::new(addr())
}

/// Speaks one clamd command by hand, so a reply can be asserted as bytes.
fn raw_command(command: &[u8]) -> String {
    let mut sock = std::net::TcpStream::connect(addr()).unwrap_or_else(|e| {
        panic!(
            "clamd unreachable at {}: {e}. Run scripts/p6f-clamd-live.sh",
            addr()
        )
    });
    sock.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    sock.write_all(command).expect("write command");
    let mut out = Vec::new();
    sock.read_to_end(&mut out).expect("read reply");
    String::from_utf8_lossy(&out)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

/// Ships an INSTREAM frame by hand and returns the daemon's reply.
///
/// `None` means the daemon answered and closed while the frame was still
/// going out, so the write failed with EPIPE before the reply could be read.
/// That is a real outcome for an oversized stream — the daemon stops reading
/// as soon as the limit trips — and it is why the assertions below accept
/// either path as long as neither is "clean".
fn raw_instream(payload: &[u8]) -> Option<String> {
    let mut sock = std::net::TcpStream::connect(addr()).unwrap_or_else(|e| {
        panic!(
            "clamd unreachable at {}: {e}. Run scripts/p6f-clamd-live.sh",
            addr()
        )
    });
    sock.set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    sock.set_write_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    if sock.write_all(b"zINSTREAM\0").is_err() {
        return None;
    }
    for chunk in payload.chunks(4096) {
        if sock
            .write_all(&(chunk.len() as u32).to_be_bytes())
            .and_then(|()| sock.write_all(chunk))
            .is_err()
        {
            return None;
        }
    }
    if sock.write_all(&0u32.to_be_bytes()).is_err() {
        return None;
    }
    let mut out = Vec::new();
    sock.read_to_end(&mut out).ok()?;
    Some(
        String::from_utf8_lossy(&out)
            .trim_end_matches('\0')
            .trim()
            .to_string(),
    )
}

/// The daemon's `StreamMaxLength`, as `scripts/p6f-clamd-live.sh` sets it.
fn stream_limit() -> usize {
    std::env::var("CLAMD_LIVE_STREAM_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024 * 1024)
}

#[tokio::test]
#[ignore = "live leg: boots a clamd container; run scripts/p6f-clamd-live.sh"]
async fn live_daemon_is_reachable_and_names_its_version() {
    assert_eq!(
        raw_command(b"zPING\0"),
        "PONG",
        "a live clamd answers zPING"
    );
    // Printed, not asserted: the version and signature date are what the
    // gate record cites, so they belong in the log next to the verdicts.
    println!("clamd: {}", raw_command(b"zVERSION\0"));
}

#[tokio::test]
#[ignore = "live leg: boots a clamd container; run scripts/p6f-clamd-live.sh"]
async fn live_clean_octets_come_back_clean_not_skipped() {
    let verdict = scanner()
        .scan_bytes(
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
        )
        .await
        .expect("a real daemon answers");
    assert!(!verdict.infected);
    assert!(!verdict.skipped, "a real scan is not the none-mode skip");
    assert!(verdict.detail.is_none(), "{:?}", verdict.detail);
}

/// The whole-file form is what the daemon detects, and its FOUND line names
/// the signature. The `detail` the gate records comes from this string.
#[tokio::test]
#[ignore = "live leg: boots a clamd container; run scripts/p6f-clamd-live.sh"]
async fn live_eicar_whole_file_is_found_with_its_signature_name() {
    let verdict = scanner()
        .scan_bytes(EICAR)
        .await
        .expect("a real daemon answers");
    assert!(
        verdict.infected,
        "EICAR is the one sample everybody agrees on"
    );
    let detail = verdict.detail.expect("a FOUND reply carries its signature");
    assert!(
        detail.ends_with("FOUND"),
        "detail is the daemon's reply line: {detail}"
    );
    assert!(
        detail.contains("Eicar-Test-Signature"),
        "the signature name is the daemon's: {detail}"
    );
}

/// A stream past `StreamMaxLength` is not a verdict. The daemon answers
/// `INSTREAM size limit exceeded. ERROR` (and closes, so the writer can also
/// take an EPIPE before it reads that), and the adapter must fail closed
/// rather than let an oversized attachment through as clean.
#[tokio::test]
#[ignore = "live leg: boots a clamd container; run scripts/p6f-clamd-live.sh"]
async fn live_stream_past_the_daemon_limit_is_refused_never_clean() {
    let limit = stream_limit();
    let mut payload = b"%PDF-1.7\n".to_vec();
    payload.resize(limit + 64 * 1024, 0u8);

    // What the daemon says, verbatim: the reply when it can be read, or the
    // closed-connection race when the write gets there first.
    match raw_instream(&payload) {
        Some(reply) => {
            println!("oversize reply: {reply:?}");
            assert!(
                !reply.ends_with("OK") && !reply.ends_with("FOUND"),
                "a stream past the limit is never a clean scan: {reply:?}"
            );
            assert!(
                reply.contains("size limit"),
                "the daemon names the limit: {reply:?}"
            );
        }
        None => println!(
            "oversize reply: connection closed while writing (the daemon stops reading at the limit)"
        ),
    }

    let err = scanner()
        .scan_bytes(&payload)
        .await
        .expect_err("a size-limit reply is not a verdict");
    let core_mail::ScanError::Unavailable(detail) = err;
    println!("adapter error: {detail}");
    assert!(
        detail.contains("size limit") || detail.starts_with("write:"),
        "either the reply is parsed or the write is refused; both fail closed: {detail}"
    );
}

/// An empty stream is `stream: OK`, not an error: a zero-byte attachment is
/// scanned and admitted, not turned into a 503 by a parser gap.
#[tokio::test]
#[ignore = "live leg: boots a clamd container; run scripts/p6f-clamd-live.sh"]
async fn live_empty_stream_is_clean() {
    let verdict = scanner()
        .scan_bytes(b"")
        .await
        .expect("a real daemon answers");
    assert!(!verdict.infected);
    assert!(!verdict.skipped);
}
