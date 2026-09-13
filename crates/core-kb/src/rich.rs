//! core-kb rich-content rules (KB1/KB2, ADR 0004) — the pure half of the
//! oEmbed allowlist and the linked-external-document classification.
//!
//! Purpose: the ONE home for KB rich-content policy so core-api and any
//! later surface enforce identical rules:
//!
//! * [`validate_url`] — https-only absolute URLs (oEmbed fetches and
//!   external documents are never fetched over plaintext);
//! * [`match_provider`] — an embed URL is sanctioned only by a REGISTERED
//!   provider (enabled, host matches exactly, URL matches the pinned
//!   regex); raw arbitrary iframes are never allowed;
//! * [`oembed_url`] — substitutes `{url}` percent-encoded, exactly once;
//! * [`classify_url_kind`] — the closed SharePoint/OneDrive/Drive/other
//!   vocabulary.
//!
//! Non-purpose: no SQL (core-db), no HTTP (core-api owns the fetches), no
//! persistence.

use core_domain::entities::{ExternalDocUrlKind, KbEmbedProvider};
use regex::Regex;
use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum RichError {
    #[error("url must be an absolute https url")]
    BadUrl,
    #[error("url does not match any allowlisted embed provider")]
    ProviderNotAllowlisted,
    #[error("provider is disabled")]
    ProviderDisabled,
    #[error("endpoint template must contain exactly one {{url}} placeholder")]
    BadEndpointTemplate,
    #[error("host_patterns must contain at least one host")]
    EmptyHostPatterns,
    #[error("url_regex does not compile")]
    BadUrlRegex,
    #[error("title is required")]
    EmptyTitle,
}

/// Parses and validates an absolute https URL (trimmed). Anchors a URL
/// against a provider: parse the host (IDNA-decoded, lowercase), require
/// scheme https.
pub fn validate_url(raw: &str) -> Result<Url, RichError> {
    let url = Url::parse(raw.trim()).map_err(|_| RichError::BadUrl)?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(RichError::BadUrl);
    }
    Ok(url)
}

/// Validates a provider row's shape before persistence: non-empty title,
/// at least one host, a compilable single-line regex, and an endpoint
/// template with exactly one `{url}` substitution.
pub fn validate_provider(
    name: &str,
    host_patterns: &[String],
    url_regex: &str,
    oembed_endpoint: &str,
) -> Result<(), RichError> {
    if name.trim().is_empty() {
        return Err(RichError::EmptyTitle);
    }
    if host_patterns.iter().all(|h| h.trim().is_empty()) {
        return Err(RichError::EmptyHostPatterns);
    }
    let re = Regex::new(url_regex).map_err(|_e| RichError::BadUrlRegex)?;
    if re.as_str().contains('\n') || url_regex.contains("(?m)") {
        return Err(RichError::BadUrlRegex);
    }
    if oembed_endpoint.matches("{url}").count() != 1 {
        return Err(RichError::BadEndpointTemplate);
    }
    Ok(())
}

/// The allowlist decision: does this URL belong to this provider row?
/// Hosts match exactly (no suffix tricks — `evil-youtube.com` is never
/// `youtube.com`) and the full URL must match the pinned regex.
pub fn matches_provider(provider: &KbEmbedProvider, url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if !provider
        .host_patterns
        .iter()
        .any(|pattern| pattern.eq_ignore_ascii_case(host))
    {
        return false;
    }
    match Regex::new(&provider.url_regex) {
        Ok(re) => re.is_match(url.as_str()),
        Err(_) => false,
    }
}

/// Resolves an embed URL against the allowlist (system + org rows, enabled
/// only). First match wins; registry order is the caller's responsibility.
pub fn match_provider<'a>(
    providers: &'a [KbEmbedProvider],
    raw_url: &str,
) -> Result<&'a KbEmbedProvider, RichError> {
    let url = validate_url(raw_url)?;
    let provider = providers
        .iter()
        .filter(|p| p.enabled)
        .find(|p| matches_provider(p, &url))
        .ok_or(RichError::ProviderNotAllowlisted)?;
    if !provider.enabled {
        return Err(RichError::ProviderDisabled);
    }
    Ok(provider)
}

/// Substitutes `{url}` exactly once with the percent-encoded absolute URL.
/// The endpoint stays https (the registry write path validates it).
pub fn oembed_url(endpoint: &str, url: &Url) -> Result<String, RichError> {
    if endpoint.matches("{url}").count() != 1 {
        return Err(RichError::BadEndpointTemplate);
    }
    if endpoint.split("://").next().unwrap_or("") != "https" {
        return Err(RichError::BadEndpointTemplate);
    }
    let encoded = url.as_str();
    // url.as_str() keeps reserved characters; oEmbed endpoints expect the
    // URL query-encoded, so encode it once more for safe substitution.
    let encoded = encoded
        .replace('%', "%25")
        .replace('?', "%3F")
        .replace('&', "%26")
        .replace('=', "%3D");
    Ok(endpoint.replace("{url}", &encoded))
}

/// Classifies an external-document URL into the closed vocabulary. The
/// mapping is over the HOST only (path shape differs per tenant), so it
/// is stable across provider URL churn:
/// sharepoint.com / *.sharepoint.com → sharepoint; onedrive.live.com,
/// *.files.1drv.com, 1drv.ms → onedrive; drive.google.com, docs.google.com
/// → google_drive; everything else → other.
pub fn classify_url_kind(raw_url: &str) -> ExternalDocUrlKind {
    let host = validate_url(raw_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let is = |suffix: &str| host == suffix || host.ends_with(&format!(".{suffix}"));
    if is("sharepoint.com") {
        return ExternalDocUrlKind::SharePoint;
    }
    if is("onedrive.live.com") || is("1drv.com") || is("1drv.ms") {
        return ExternalDocUrlKind::OneDrive;
    }
    if is("drive.google.com") || is("docs.google.com") {
        return ExternalDocUrlKind::GoogleDrive;
    }
    ExternalDocUrlKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn provider(slug: &str, hosts: &[&str], regex: &str, endpoint: &str) -> KbEmbedProvider {
        KbEmbedProvider {
            id: uuid::Uuid::nil(),
            org_id: None,
            name: slug.to_string(),
            slug: slug.to_string(),
            host_patterns: hosts.iter().map(|h| h.to_string()).collect(),
            url_regex: regex.to_string(),
            oembed_endpoint: endpoint.to_string(),
            enabled: true,
            is_system: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn youtube() -> KbEmbedProvider {
        provider(
            "youtube",
            &["youtube.com", "www.youtube.com", "youtu.be"],
            "^(https://)?(www\\.)?(youtube\\.com/(watch\\?v=[A-Za-z0-9_-]+|shorts/[A-Za-z0-9_-]+)|youtu\\.be/[A-Za-z0-9_-]+)([?&].*)?$",
            "https://www.youtube.com/oembed?url={url}&format=json",
        )
    }

    #[test]
    fn plain_http_urls_are_rejected() {
        assert_eq!(
            validate_url("http://youtube.com/watch?v=x"),
            Err(RichError::BadUrl)
        );
        assert_eq!(validate_url("not a url"), Err(RichError::BadUrl));
        assert_eq!(validate_url("https://"), Err(RichError::BadUrl));
        assert!(validate_url("https://youtube.com/watch?v=x").is_ok());
    }

    #[test]
    fn allowlist_requires_exact_host_and_regex_match() {
        let p = youtube();
        assert_eq!(
            match_provider(
                std::slice::from_ref(&p),
                "https://evil-youtube.com/watch?v=x"
            ),
            Err(RichError::ProviderNotAllowlisted)
        );
        // Host allowlisted but path shape wrong.
        assert_eq!(
            match_provider(
                std::slice::from_ref(&p),
                "https://youtube.com/playlist?list=x"
            ),
            Err(RichError::ProviderNotAllowlisted)
        );
        // javascript: never parses as https, so never matches.
        assert!(validate_url("javascript:alert(1)").is_err());
        let providers = [p, youtube()];
        assert_eq!(
            match_provider(&providers, "https://youtu.be/dQw4w9WgXcQ").map(|p| p.slug.as_str()),
            Ok("youtube")
        );
        assert!(
            match_provider(&providers, "https://www.youtube.com/shorts/abc123")
                .map(|p| p.slug == "youtube")
                .unwrap_or(false)
        );
    }

    #[test]
    fn disabled_providers_never_match() {
        let mut p = youtube();
        p.enabled = false;
        assert_eq!(
            match_provider(&[p], "https://youtu.be/dQw4w9WgXcQ"),
            Err(RichError::ProviderNotAllowlisted)
        );
    }

    #[test]
    fn endpoint_substitution_encodes_the_url() {
        let p = youtube();
        let url = validate_url("https://youtu.be/dQw4w9WgXcQ?t=1").unwrap();
        let out = oembed_url(&p.oembed_endpoint, &url).unwrap();
        assert!(out.starts_with("https://www.youtube.com/oembed?url="));
        assert!(out.contains("dQw4w9WgXcQ%3Ft%3D1"), "encoded: {out}");
        assert!(out.ends_with("&format=json"));
        assert_eq!(
            oembed_url("https://e.example/no-param", &url),
            Err(RichError::BadEndpointTemplate)
        );
        assert_eq!(
            oembed_url("http://e.example/?u={url}", &url),
            Err(RichError::BadEndpointTemplate)
        );
    }

    #[test]
    fn provider_validation_rejects_bad_shapes() {
        assert_eq!(
            validate_provider("", &["h.com".to_string()], ".", "https://e/{url}"),
            Err(RichError::EmptyTitle)
        );
        assert_eq!(
            validate_provider("n", &["".to_string()], ".", "https://e/{url}"),
            Err(RichError::EmptyHostPatterns)
        );
        assert_eq!(
            validate_provider("n", &["h.com".to_string()], "(unclosed", "https://e/{url}"),
            Err(RichError::BadUrlRegex)
        );
        assert_eq!(
            validate_provider("n", &["h.com".to_string()], ".", "https://e"),
            Err(RichError::BadEndpointTemplate)
        );
        assert!(validate_provider("n", &["h.com".to_string()], ".", "https://e/?u={url}").is_ok());
    }

    #[test]
    fn url_kind_classification_is_host_based() {
        assert_eq!(
            classify_url_kind("https://acme.sharepoint.com/:w:/r/sites/IT/Doc.docx"),
            ExternalDocUrlKind::SharePoint
        );
        assert_eq!(
            classify_url_kind("https://onedrive.live.com/?id=root"),
            ExternalDocUrlKind::OneDrive
        );
        assert_eq!(
            classify_url_kind("https://drive.google.com/file/d/123/view"),
            ExternalDocUrlKind::GoogleDrive
        );
        assert_eq!(
            classify_url_kind("https://confluence.internal/spaces/IT/pages/1"),
            ExternalDocUrlKind::Other
        );
    }

    #[test]
    fn host_match_is_case_insensitive() {
        let p = youtube();
        assert!(matches_provider(
            &p,
            &validate_url("https://WWW.YOUTUBE.COM/watch?v=x").unwrap()
        ));
    }
}
