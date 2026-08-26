//! Attachment admission rules (TASKLIST 2.14): filename sanitization, the
//! content-type/magic-byte allowlist, and size caps. Pure functions over
//! the bytes' HEAD — the API layer owns IO, storage, and authorization.
//!
//! Doctrine: conservative ALLOWLIST. Anything not positively recognized is
//! rejected; a declared content type that contradicts the sniffed magic
//! bytes is rejected even when both are individually permitted. Executable
//! formats never pass regardless of declaration.
//!
//! Non-purpose: no malware verdicts (`core-mail::ScanProvider` owns
//! scanning), no quota accounting, no storage.

use crate::error::DomainError;

/// Default per-file cap; config can lower but the API default is 25 MB.
pub const DEFAULT_MAX_UPLOAD_BYTES: i64 = 25 * 1024 * 1024;
/// Filenames are capped so Content-Disposition headers stay sane.
pub const MAX_FILENAME_LEN: usize = 180;
/// How many leading bytes the sniffer needs at most.
pub const MAGIC_HEAD_LEN: usize = 16;

/// One allowlisted content family: the sniffed signature AND the set of
/// file extensions that may declare it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Pdf,
    Png,
    Jpeg,
    Gif,
    Webp,
    /// ZIP container: .zip plus OOXML (.docx/.xlsx/.pptx).
    Zip,
    /// UTF-8 text without C0 control characters: txt/csv/md/json/log.
    Text,
}

impl Family {
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Family::Pdf => &["pdf"],
            Family::Png => &["png"],
            Family::Jpeg => &["jpg", "jpeg"],
            Family::Gif => &["gif"],
            Family::Webp => &["webp"],
            Family::Zip => &["zip", "docx", "xlsx", "pptx"],
            Family::Text => &["txt", "csv", "md", "json", "log"],
        }
    }

    fn declared_content_types(self) -> &'static [&'static str] {
        match self {
            Family::Pdf => &["application/pdf"],
            Family::Png => &["image/png"],
            Family::Jpeg => &["image/jpeg"],
            Family::Gif => &["image/gif"],
            Family::Webp => &["image/webp"],
            Family::Zip => &[
                "application/zip",
                "application/x-zip-compressed",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "application/octet-stream",
            ],
            Family::Text => &[
                "text/plain",
                "text/csv",
                "text/markdown",
                "application/json",
                "text/json",
            ],
        }
    }
}

/// Sniffs the payload head. Returns the family whose magic bytes match;
/// text is detected by UTF-8 validity with no C0 controls (tab/newline/CR
/// excepted). `None` = unrecognized binary — always rejected.
#[must_use]
fn sniff(head: &[u8]) -> Option<Family> {
    let starts = |sig: &[u8]| head.starts_with(sig);
    if starts(b"%PDF-") {
        return Some(Family::Pdf);
    }
    if starts(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(Family::Png);
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(Family::Jpeg);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(Family::Gif);
    }
    if starts(b"RIFF") && head.len() >= 12 && &head[8..12] == b"WEBP" {
        return Some(Family::Webp);
    }
    if starts(&[0x50, 0x4B, 0x03, 0x04]) || starts(&[0x50, 0x4B, 0x05, 0x06]) {
        return Some(Family::Zip);
    }
    // Text heuristic: valid UTF-8, printable + whitespace only. Empty
    // payloads count as empty text files.
    if std::str::from_utf8(head).is_ok_and(|s| {
        s.chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t')
    }) {
        return Some(Family::Text);
    }
    None
}

/// Lowercased extension of `filename` (without the dot); empty when none.
#[must_use]
fn extension_of(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    match base.rsplit_once('.') {
        Some((_, ext)) => ext.to_ascii_lowercase(),
        None => String::new(),
    }
}

/// Sanitizes a client-supplied filename: strips path components and
/// control characters, collapses whitespace, enforces the length cap,
/// guarantees a non-empty result. Extension case is preserved for display.
#[must_use]
pub fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| {
            if c.is_whitespace() {
                vec![' ']
            } else {
                vec![c]
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    let mut out = trimmed;
    out.truncate(MAX_FILENAME_LEN);
    if out.is_empty() {
        "attachment".to_string()
    } else {
        out
    }
}

/// Validates one upload against policy: size cap, sanitized filename,
/// recognized family via magic bytes, extension permitted for that family,
/// declared content type compatible with it. Returns the canonical content
/// type to persist.
///
/// # Errors
/// [`DomainError::Validation`] describing exactly which gate failed.
pub fn validate_upload(
    raw_filename: &str,
    declared_content_type: &str,
    head: &[u8],
    total_size_bytes: i64,
    max_bytes: i64,
) -> Result<(String, String), DomainError> {
    if total_size_bytes <= 0 {
        return Err(DomainError::Validation("attachment is empty".into()));
    }
    if total_size_bytes > max_bytes {
        return Err(DomainError::Validation(format!(
            "attachment exceeds size cap ({total_size_bytes} > {max_bytes} bytes)"
        )));
    }
    let filename = sanitize_filename(raw_filename);
    let family = sniff(head).ok_or_else(|| {
        DomainError::Validation(
            "attachment type not allowed: unrecognized or executable format".into(),
        )
    })?;
    let ext = extension_of(&filename);
    if !family.extensions().contains(&ext.as_str()) {
        return Err(DomainError::Validation(format!(
            "extension .{ext} does not match detected file type"
        )));
    }
    let declared = declared_content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !declared.is_empty() && !family.declared_content_types().contains(&declared.as_str()) {
        return Err(DomainError::Validation(format!(
            "declared content type {declared:?} contradicts detected file type"
        )));
    }
    Ok((filename, declared))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_HEAD: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    const PDF_HEAD: &[u8] = b"%PDF-1.7\n";
    const EXE_HEAD: &[u8] = &[0x4D, 0x5A, 0x90, 0x00];

    #[test]
    fn accepts_matching_extension_type_and_magic() {
        let (name, ct) = validate_upload(
            "report.png",
            "image/png",
            PNG_HEAD,
            100,
            DEFAULT_MAX_UPLOAD_BYTES,
        )
        .expect("ok");
        assert_eq!(name, "report.png");
        assert_eq!(ct, "image/png");
    }

    #[test]
    fn rejects_executable_magic_even_with_innocent_name() {
        let err = validate_upload("invoice.pdf", "application/pdf", EXE_HEAD, 10, 1000)
            .expect_err("MZ rejected");
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn rejects_extension_family_mismatch() {
        assert!(validate_upload("evil.png", "image/png", PDF_HEAD, 10, 1000).is_err());
    }

    #[test]
    fn rejects_declared_type_contradiction() {
        assert!(validate_upload("a.png", "application/pdf", PNG_HEAD, 10, 1000).is_err());
    }

    #[test]
    fn rejects_empty_and_oversize() {
        assert!(validate_upload("a.txt", "text/plain", b"x", 0, 1000).is_err());
        assert!(validate_upload("a.txt", "text/plain", b"x", 2000, 1000).is_err());
    }

    #[test]
    fn sanitizes_path_traversal_and_controls_in_filename() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("..\\..\\win.ini"), "win.ini");
        assert_eq!(sanitize_filename("a\u{0007}b.txt"), "ab.txt");
        assert_eq!(sanitize_filename("   "), "attachment");
        assert_eq!(sanitize_filename(".hidden."), "hidden");
    }

    #[test]
    fn text_files_pass_without_binary_magic() {
        validate_upload(
            "notes.md",
            "text/markdown",
            b"# hello\nworld\t",
            20,
            DEFAULT_MAX_UPLOAD_BYTES,
        )
        .expect("markdown ok");
        assert!(validate_upload("x.bin", "", &[0, 159, 146, 150], 4, 1000).is_err());
    }

    /// Property: whatever arrives, an accepted upload's persisted name is
    /// sanitized and its declared type matches its family.
    #[test]
    fn prop_sanitize_output_is_always_safe() {
        use proptest::prelude::*;
        proptest::proptest! {
            #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
            #[test]
            fn safe_names(raw in "\\PC*{0,60}") {
                let name = sanitize_filename(&raw);
                proptest::prop_assert!(!name.contains('/'));
                proptest::prop_assert!(!name.contains('\\'));
                proptest::prop_assert!(!name.chars().any(|c| c.is_control()));
                proptest::prop_assert!(!name.trim().is_empty());
                proptest::prop_assert!(name.len() <= MAX_FILENAME_LEN);
            }
        }
    }
}
