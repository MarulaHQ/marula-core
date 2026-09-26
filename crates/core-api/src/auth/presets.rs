//! First-party OIDC provider PRESETS (TASKLIST ID1; Wave 34 lane 6 adds
//! Keycloak): Google, Entra ID/M365, Okta, Keycloak as DECLARATIVE config.
//! The catalog below is data — issuer/discovery templates, scopes, and a
//! per-preset claim mapping — so an admin creates a provider by naming a
//! preset and filling its parameters; no code fork per vendor.
//!
//! Security posture: a preset only FILLS configuration defaults
//! (issuer, discovery URL, attribute mapping). Secrets, client ids,
//! and enable decisions stay admin inputs; verification is identical
//! for every provider (RS256 via the discovered JWKS with the issuer
//! pinned) regardless of preset.
//!
//! Non-preset kinds: a headless IdP that does NOT speak standard OIDC
//! (e.g. a bare Ory Kratos deployment without an OIDC front) is NOT a
//! preset — it would need its own session-introspection provider KIND at
//! the `auth::provider` seam. See docs/reference/auth-provider-contract.md
//! for that typed not-built state; a Kratos deployment fronted by
//! Hydra/Ory Network speaks standard OIDC and works through the generic
//! (preset-less) provider path today.

use crate::auth::oidc_client::IdClaims;
use std::collections::HashMap;

/// One first-party preset. Templates use `{name}` placeholders filled
/// from the admin's `preset_params` map.
pub struct OidcPreset {
    /// Machine name stored on `oidc_providers.preset`.
    pub name: &'static str,
    pub display_name: &'static str,
    /// `{param}`-templated issuer, pinned at verification time.
    pub issuer_template: &'static str,
    /// `{param}`-templated discovery document URL.
    pub discovery_template: &'static str,
    /// Scopes the login redirect requests.
    pub scopes: &'static str,
    /// Parameters the admin MUST provide for this preset.
    pub required_params: &'static [&'static str],
    /// Where the human-facing redirect guidance lives.
    pub redirect_guidance: &'static str,
    /// Normalizes the provider's id_token claims into [`IdClaims`]
    /// shape (attribute mapping per preset).
    pub map_claims: fn(&mut IdClaims),
}

/// Google Workspace. Claims are standard OIDC; `email` arrives with
/// `email_verified` set for Google-hosted accounts.
fn map_google(c: &mut IdClaims) {
    // Google asserts email_verified reliably; nothing to remap.
    let _ = c;
}

/// Entra ID (Azure AD) v2.0 endpoint. `email` is often ABSENT —
/// `preferred_username` carries the UPN. Verified flag is not asserted
/// the way Google does, so conservative linking stays explicit.
fn map_entra(c: &mut IdClaims) {
    if c.email.is_none() {
        if let Some(upn) = &c.preferred_username {
            if upn.contains('@') {
                c.email = Some(upn.clone());
            }
        }
    }
    if c.name.is_none() {
        if let Some(upn) = &c.preferred_username {
            c.name = Some(upn.clone());
        }
    }
}

/// Okta. Standard claims; `preferred_username` is the login attribute.
fn map_okta(c: &mut IdClaims) {
    if c.name.is_none() {
        if let Some(login) = &c.preferred_username {
            c.name = Some(login.clone());
        }
    }
}

/// Keycloak (self-hosted or any standards realm). Standard OIDC claims; a
/// realm can be configured without the `email` scope, in which case
/// `preferred_username` is the e-mail-shaped login and the UPN fallback
/// applies exactly as it does for Entra. `email_verified` stays whatever
/// the realm asserts — never invented here.
fn map_keycloak(c: &mut IdClaims) {
    if c.email.is_none() {
        if let Some(login) = &c.preferred_username {
            if login.contains('@') {
                c.email = Some(login.clone());
            }
        }
    }
    if c.name.is_none() {
        if let Some(login) = &c.preferred_username {
            c.name = Some(login.clone());
        }
    }
}

pub const OIDC_PRESETS: &[OidcPreset] = &[
    OidcPreset {
        name: "google",
        display_name: "Google Workspace",
        issuer_template: "https://accounts.google.com",
        discovery_template: "https://accounts.google.com/.well-known/openid-configuration",
        scopes: "openid email profile",
        required_params: &[],
        redirect_guidance: "Authorized redirect URI: {base}/api/auth/oidc/{name}/callback",
        map_claims: map_google,
    },
    OidcPreset {
        name: "entra",
        display_name: "Microsoft Entra ID (M365)",
        issuer_template: "https://login.microsoftonline.com/{tenant}/v2.0",
        discovery_template:
            "https://login.microsoftonline.com/{tenant}/v2.0/.well-known/openid-configuration",
        scopes: "openid email profile",
        required_params: &["tenant"],
        redirect_guidance:
            "App registration → Web redirect URI: {base}/api/auth/oidc/{name}/callback",
        map_claims: map_entra,
    },
    OidcPreset {
        name: "okta",
        display_name: "Okta",
        issuer_template: "https://{domain}/oauth2/default",
        discovery_template: "https://{domain}/.well-known/openid-configuration",
        scopes: "openid email profile",
        required_params: &["domain"],
        redirect_guidance:
            "Okta app → General → Login redirect URIs: {base}/api/auth/oidc/{name}/callback",
        map_claims: map_okta,
    },
    OidcPreset {
        name: "keycloak",
        display_name: "Keycloak (realm)",
        issuer_template: "https://{host}/realms/{realm}",
        discovery_template: "https://{host}/realms/{realm}/.well-known/openid-configuration",
        scopes: "openid email profile",
        required_params: &["host", "realm"],
        redirect_guidance:
            "Keycloak admin → Clients → {name} → Valid redirect URIs: {base}/api/auth/oidc/{name}/callback",
        map_claims: map_keycloak,
    },
];

pub fn get_preset(name: &str) -> Option<&'static OidcPreset> {
    OIDC_PRESETS.iter().find(|p| p.name == name)
}

/// Fills a `{param}` template. Unknown or missing parameters are an
/// error — a partially-resolved issuer must never be persisted.
pub fn render_template(template: &str, params: &HashMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let end = rest[start..]
            .find('}')
            .ok_or_else(|| format!("template {template:?} has an unterminated placeholder"))?;
        out.push_str(&rest[..start]);
        let key = &rest[start + 1..start + end];
        let value = params
            .get(key)
            .ok_or_else(|| format!("missing preset parameter {key:?}"))?;
        out.push_str(value);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Resolves a preset + params into the concrete (issuer, discovery)
/// pair to persist, applying the preset's claim mapping name.
pub fn resolve_preset(
    preset_name: &str,
    params: &HashMap<String, String>,
) -> Result<(String, String), String> {
    let preset =
        get_preset(preset_name).ok_or_else(|| format!("unknown preset {preset_name:?}"))?;
    for key in preset.required_params {
        if params.get(*key).is_none_or(|v| v.trim().is_empty()) {
            return Err(format!("missing preset parameter {key:?}"));
        }
    }
    let issuer = render_template(preset.issuer_template, params)?;
    let discovery = render_template(preset.discovery_template, params)?;
    Ok((issuer, discovery))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_render_declared_issuer_and_discovery() {
        let mut p = HashMap::new();
        p.insert("tenant".to_string(), "contoso.onmicrosoft.com".to_string());
        let (issuer, discovery) = resolve_preset("entra", &p).unwrap();
        assert_eq!(
            issuer,
            "https://login.microsoftonline.com/contoso.onmicrosoft.com/v2.0"
        );
        assert_eq!(
            discovery,
            "https://login.microsoftonline.com/contoso.onmicrosoft.com/v2.0/.well-known/openid-configuration"
        );

        let mut p = HashMap::new();
        p.insert("domain".to_string(), "example.okta.com".to_string());
        let (issuer, _) = resolve_preset("okta", &p).unwrap();
        assert_eq!(issuer, "https://example.okta.com/oauth2/default");

        let (issuer, discovery) = resolve_preset("google", &HashMap::new()).unwrap();
        assert_eq!(issuer, "https://accounts.google.com");
        assert!(discovery.ends_with(".well-known/openid-configuration"));

        // Keycloak (self-hosted IdP realm): host + realm template the
        // standard realm issuer/discovery pair.
        let mut p = HashMap::new();
        p.insert("host".to_string(), "sso.example.com".to_string());
        p.insert("realm".to_string(), "meridian".to_string());
        let (issuer, discovery) = resolve_preset("keycloak", &p).unwrap();
        assert_eq!(issuer, "https://sso.example.com/realms/meridian");
        assert_eq!(
            discovery,
            "https://sso.example.com/realms/meridian/.well-known/openid-configuration"
        );
    }

    #[test]
    fn presets_fail_closed_on_missing_or_unknown_input() {
        // entra without its tenant parameter: refused, never half-filled.
        assert!(resolve_preset("entra", &HashMap::new()).is_err());
        assert!(resolve_preset("no-such-preset", &HashMap::new()).is_err());
        // A blank parameter is as good as a missing one.
        let mut p = HashMap::new();
        p.insert("tenant".to_string(), "  ".to_string());
        assert!(resolve_preset("entra", &p).is_err());
    }

    #[test]
    fn entra_mapping_falls_back_to_upn_email() {
        let mut claims = IdClaims {
            sub: "s".into(),
            email: None,
            email_verified: None,
            name: None,
            preferred_username: Some("alice@contoso.com".into()),
            nonce: None,
        };
        (get_preset("entra").unwrap().map_claims)(&mut claims);
        assert_eq!(claims.email.as_deref(), Some("alice@contoso.com"));
        assert_eq!(claims.name.as_deref(), Some("alice@contoso.com"));

        // A non-email preferred_username must NOT become an email.
        let mut claims = IdClaims {
            sub: "s".into(),
            email: None,
            email_verified: None,
            name: None,
            preferred_username: Some("alice.smith".into()),
            nonce: None,
        };
        (get_preset("entra").unwrap().map_claims)(&mut claims);
        assert!(claims.email.is_none());
    }

    #[test]
    fn keycloak_mapping_uses_upn_fallback_only_for_email_shaped_logins() {
        let mut claims = IdClaims {
            sub: "s".into(),
            email: None,
            email_verified: Some(true),
            name: None,
            preferred_username: Some("alice@example.com".into()),
            nonce: None,
        };
        (get_preset("keycloak").unwrap().map_claims)(&mut claims);
        assert_eq!(claims.email.as_deref(), Some("alice@example.com"));
        assert_eq!(claims.name.as_deref(), Some("alice@example.com"));
        assert_eq!(claims.email_verified, Some(true));

        let mut bare = IdClaims {
            sub: "s".into(),
            email: None,
            email_verified: None,
            name: None,
            preferred_username: Some("alice".into()),
            nonce: None,
        };
        (get_preset("keycloak").unwrap().map_claims)(&mut bare);
        assert!(bare.email.is_none());
    }
}
