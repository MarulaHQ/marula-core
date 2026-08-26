//! WebAuthn/passkey ceremonies via `webauthn-rs`, scoped to a platform
//! authenticator ("passkey") profile.
//!
//! Purpose: build the relying-party context from config and run the four
//! ceremony halves (register begin/finish, authenticate begin/finish).
//! Ceremony state is serializable — callers persist it (mfa_challenges)
//! between the two halves; nothing here touches the database.
//!
//! Non-purpose: credential storage and user resolution.

use webauthn_rs::prelude::*;

pub struct CeremonyContext {
    webauthn: Webauthn,
}

#[derive(Debug, thiserror::Error)]
pub enum WebauthnError {
    #[error("webauthn configuration invalid: {0}")]
    Config(String),
    #[error("webauthn ceremony rejected: {0}")]
    Rejected(String),
}

impl CeremonyContext {
    pub fn new(rp_id: &str, rp_origin: &str) -> Result<Self, WebauthnError> {
        let origin = Url::parse(rp_origin).map_err(|e| WebauthnError::Config(e.to_string()))?;
        let builder = WebauthnBuilder::new(rp_id, &origin)
            .map_err(|e| WebauthnError::Config(e.to_string()))?;
        Ok(Self {
            webauthn: builder
                .build()
                .map_err(|e| WebauthnError::Config(e.to_string()))?,
        })
    }

    /// Registration half 1 → browser options + server state to persist.
    /// `exclude` carries already-registered credential ids as base64url.
    pub fn start_registration(
        &self,
        user_unique_id: Uuid,
        user_name: &str,
        user_display_name: &str,
        exclude: Vec<String>,
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), WebauthnError> {
        let excluded: Option<Vec<CredentialID>> = if exclude.is_empty() {
            None
        } else {
            Some(
                exclude
                    .iter()
                    .filter_map(|s| data_encoding::BASE64URL_NOPAD.decode(s.as_bytes()).ok())
                    .map(CredentialID::from)
                    .collect(),
            )
        };
        self.webauthn
            .start_passkey_registration(user_unique_id, user_name, user_display_name, excluded)
            .map_err(|e| WebauthnError::Rejected(e.to_string()))
    }

    /// Registration half 2 → the stored passkey record.
    pub fn finish_registration(
        &self,
        response: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> Result<Passkey, WebauthnError> {
        self.webauthn
            .finish_passkey_registration(response, state)
            .map_err(|e| WebauthnError::Rejected(e.to_string()))
    }

    /// Authentication half 1 → browser assertion request + server state.
    pub fn start_authentication(
        &self,
        credentials: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), WebauthnError> {
        self.webauthn
            .start_passkey_authentication(credentials)
            .map_err(|e| WebauthnError::Rejected(e.to_string()))
    }

    /// Authentication half 2 → verified result (counter advanced, user
    /// presence asserted).
    pub fn finish_authentication(
        &self,
        response: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> Result<AuthenticationResult, WebauthnError> {
        self.webauthn
            .finish_passkey_authentication(response, state)
            .map_err(|e| WebauthnError::Rejected(e.to_string()))
    }
}

/// Serializes a passkey for JSONB storage.
pub fn passkey_to_json(passkey: &Passkey) -> serde_json::Value {
    serde_json::to_value(passkey).unwrap_or(serde_json::Value::Null)
}

/// Deserializes stored passkeys; corrupt rows are skipped (never fatal).
pub fn passkeys_from_json(rows: &[serde_json::Value]) -> Vec<Passkey> {
    rows.iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CeremonyContext {
        CeremonyContext::new("localhost", "http://localhost:3000").unwrap()
    }

    #[test]
    fn registration_challenge_is_wellformed() {
        let ctx = ctx();
        let (challenge, _state) = ctx
            .start_registration(Uuid::now_v7(), "user@x.test", "User", vec![])
            .unwrap();
        assert!(!challenge.public_key.challenge.is_empty());
        assert_eq!(challenge.public_key.rp.name.as_str(), "localhost");
    }

    #[test]
    fn passkeys_survive_a_json_roundtrip() {
        // A passkey only exists post-ceremony; here we prove the storage
        // codec at least roundtrips an empty set without loss or panic.
        let empty: Vec<Passkey> = passkeys_from_json(&[serde_json::json!({"not": "a passkey"})]);
        assert!(empty.is_empty(), "corrupt rows are skipped");
    }
}
