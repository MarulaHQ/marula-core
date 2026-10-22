//! Per-install credential bindings: which vault record an install's egress
//! profile may wear, and the sealed bytes a resolve returns (Wave 53 lane 12,
//! migration 0157).
//!
//! # What this module decides, and what it must never decide
//!
//! ADR 0023 defers `secret_get` because an install holding both a secret read
//! and `http_request` can take an org's vault and post it anywhere in one
//! call. This module is the safe half of that pair: the guest names a profile,
//! core-api resolves the binding, decrypts, and adds a header — the value
//! never enters guest memory, so the composition the ADR refused cannot be
//! rebuilt from this side.
//!
//! Three rules hold the shape:
//!
//! 1. **No value, ever.** The binding is a pointer at a `secret_records` row.
//!    The only bytes this module returns live in [`ResolvedCredential::sealed`]
//!    and they are ciphertext. There is no function here that takes a key,
//!    holds a key, or returns a `String` read out of `secret_fields`.
//! 2. **The vault's own grant is the authority.** Both [`bind`] and [`resolve`]
//!    call [`crate::secret_vault::folder_access`], the recursive role x folder
//!    check the audited reveal endpoint runs. No plugin-specific grant
//!    vocabulary exists, because a second authority is how the two drift.
//!    [`bind`] checks the consenting operator's roles now; [`resolve`] re-runs
//!    the check against the roles recorded on the row, so a grant that was
//!    withdrawn after consent refuses at the next call instead of trusting a
//!    decision that has since stopped being true.
//! 3. **Org scope is a predicate, not a hope.** [`bind`] selects the record
//!    through the caller's `org_id` inside the INSERT and inserts nothing when
//!    it is not there, so another tenant's record is a `404`, not a row. A
//!    client-scope record is refused by the same predicate on purpose: an
//!    org-wide install may not wear a client's credential.
//!
//! # Query style
//!
//! Plain `sqlx::query` / `query_as` with an explicit `FromRow`, like
//! [`crate::plugin_health`], rather than the compile-time `query!` macro: this
//! module has no offline-cache entry, and a module that needs a `.sqlx` blob
//! before it can build is a module that cannot be reviewed on its own.
//!
//! Every function takes a connection that already carries `app.org_id` (see
//! [`crate::set_org`]); the row's `org_id` is what the RLS policy isolates.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use crate::{secret_vault, DbError, DbResult};

/// The vault's canonical material field. `crates/core-api/src/routes/secrets.rs`
/// keeps a closed field vocabulary (`secret`, `notes`, `username`, `url`) and
/// `secret` is the one that holds the credential; the other three are
/// metadata a connector has no business putting in a header.
pub const CREDENTIAL_FIELD_KEY: &str = "secret";

/// One binding row, with the record's title joined for the consent view.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct CredentialBinding {
    pub id: Uuid,
    pub org_id: Uuid,
    pub install_id: Uuid,
    /// The profile name from the pinned manifest's `outbound_scopes`.
    pub profile: String,
    pub secret_record_id: Uuid,
    /// The header the value is injected as, lowercased.
    pub header_name: String,
    /// Who consented. `None` when that user has since been deleted; the
    /// binding survives the user because the install outlives the operator.
    pub bound_by: Option<Uuid>,
    /// The roles the consenting operator held. Read by [`resolve`], never by a
    /// screen: a stale uuid here is a fail-closed refusal, not a display bug.
    pub bound_role_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The vault record's title — the only thing about the record the consent
    /// surface may name. Joined here so no caller has to remember that.
    pub record_title: String,
}

/// The values a caller may set on a binding. Deliberately not the whole row:
/// `id`, `org_id`, timestamps and the record's title are the database's.
#[derive(Debug, Clone)]
pub struct NewBinding<'a> {
    pub profile: &'a str,
    pub secret_record_id: Uuid,
    /// Lowercased by the caller; the column's CHECK is the backstop.
    pub header_name: &'a str,
    pub bound_by: Option<Uuid>,
    /// The roles the consenting operator holds right now — the authority
    /// [`bind`] checks and [`resolve`] re-checks.
    pub bound_role_ids: Vec<Uuid>,
}

/// What [`bind`] did, typed so the route can answer `404` and `403` without
/// parsing prose.
#[derive(Debug)]
pub enum BindOutcome {
    Bound(Box<CredentialBinding>),
    /// No such record in this org — which is also the answer for another
    /// tenant's record and for a client-scope record. One refusal for three
    /// causes, so a probe cannot tell which.
    NoSuchRecord,
    /// The consenting roles do not hold read on the record's category.
    CategoryDenied,
}

/// What [`resolve`] found. Every variant is a decision phase 2 maps to a
/// refusal code; none of them carries plaintext.
#[derive(Debug)]
pub enum ResolveOutcome {
    /// The binding exists, its category grant still holds, and the ciphertext
    /// is in [`ResolvedCredential::sealed`].
    Granted(Box<ResolvedCredential>),
    /// This install has no binding for that profile.
    NotBound,
    /// The binding exists but the roles recorded on it no longer hold read on
    /// the record's category. Refused rather than trusted: consent is a
    /// standing decision, and this is where it stops standing.
    GrantRevoked,
    /// The record is there but has no `secret` field — an empty record, or one
    /// whose material was cleared after consent.
    FieldMissing,
}

/// The sealed credential a credentialed request needs, and nothing else.
///
/// # Who decrypts
///
/// **Not this crate.** `core-db` has no key and must not grow one: it is the
/// module every query path links, and a key here would be a key in the
/// process that also serves paths nobody has audited for it. The caller that
/// is about to make the request decrypts — in phase 1's design that is
/// core-api, which owns `AppState::config.secret_key` and
/// `crate::crypto::open_with_aad`, and which performs the request parent-side
/// because the worker that runs untrusted wasm must never hold the key.
///
/// The fields `org_id`, `record_id` and `field_key` are the AAD coordinates:
/// vault ciphertext is sealed against
/// `secret_field:{org_id}:{record_id}:{field_key}`
/// (`crates/core-api/src/routes/secrets.rs::field_aad`), so the caller rebuilds
/// the AAD from them rather than this module re-deriving a format that lives
/// in another crate. Importer-forwarded rows predate that AAD and open through
/// the legacy `crypto::open`; that fallback is the caller's, not this
/// module's.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedCredential {
    pub install_id: Uuid,
    pub org_id: Uuid,
    pub profile: String,
    /// The header the transport applies after its own guard has passed.
    pub header_name: String,
    pub record_id: Uuid,
    /// Always [`CREDENTIAL_FIELD_KEY`] today; carried so the caller builds the
    /// AAD from the row it actually read instead of a constant it hopes agrees.
    pub field_key: String,
    /// Ciphertext. Never plaintext, and never a `String`.
    pub sealed: Vec<u8>,
}

/// Hand-written so a `tracing::debug!(?credential)` prints a length and not
/// the ciphertext bytes. Ciphertext is not a disclosure on its own, but a log
/// line that grows a kilobyte of base64 per request is a problem an operator
/// should opt into, not inherit.
impl std::fmt::Debug for ResolvedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedCredential")
            .field("install_id", &self.install_id)
            .field("org_id", &self.org_id)
            .field("profile", &self.profile)
            .field("header_name", &self.header_name)
            .field("record_id", &self.record_id)
            .field("field_key", &self.field_key)
            .field(
                "sealed",
                &format_args!("<{} sealed bytes>", self.sealed.len()),
            )
            .finish()
    }
}

const BINDING_COLUMNS: &str = "b.id, b.org_id, b.install_id, b.profile, b.secret_record_id, \
     b.header_name, b.bound_by, b.bound_role_ids, b.created_at, b.updated_at, \
     sr.title AS record_title";

/// One install's bindings, ordered by profile so the consent screen and the
/// test that pins its shape see a stable list.
pub async fn list(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<Vec<CredentialBinding>> {
    let sql = format!(
        "SELECT {BINDING_COLUMNS}
           FROM plugin_credential_bindings b
           JOIN secret_records sr ON sr.id = b.secret_record_id
          WHERE b.org_id = $1 AND b.install_id = $2
          ORDER BY b.profile"
    );
    let rows = sqlx::query_as::<_, CredentialBinding>(&sql)
        .bind(org_id)
        .bind(install_id)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows)
}

/// Every binding for a page of installs, in one statement.
///
/// The install roster builds one view per install, so a per-install read here
/// would be the N+1 this function exists to avoid: the roster already costs
/// two reads per install and a third would be a visible regression on a page
/// an admin opens.
pub async fn list_for_installs(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_ids: &[Uuid],
) -> DbResult<Vec<CredentialBinding>> {
    if install_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {BINDING_COLUMNS}
           FROM plugin_credential_bindings b
           JOIN secret_records sr ON sr.id = b.secret_record_id
          WHERE b.org_id = $1 AND b.install_id = ANY($2)
          ORDER BY b.install_id, b.profile"
    );
    let rows = sqlx::query_as::<_, CredentialBinding>(&sql)
        .bind(org_id)
        .bind(install_ids)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows)
}

/// Binds (or re-binds) one profile to one vault record.
///
/// Order matters and is the security argument:
///
/// 1. The record is selected through the caller's org. A record from another
///    tenant, or a client-scope record, yields [`BindOutcome::NoSuchRecord`]
///    before anything is written — the check is the SELECT that feeds the
///    INSERT, not a Rust comparison over a row someone else already trusted.
/// 2. [`secret_vault::folder_access`] answers with the ROLES BEING BOUND. An
///    operator who cannot read the category cannot hand it to a plugin; this
///    is the same check the reveal endpoint runs, with the same arguments.
/// 3. The row is upserted on `(install_id, profile)`. Re-binding is how a
///    rotation lands, and it re-records `bound_by` and `bound_role_ids`: a
///    credential re-bound by a different operator is consented by them, not by
///    their predecessor.
pub async fn bind(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    binding: &NewBinding<'_>,
) -> DbResult<BindOutcome> {
    // Org scope only, and deliberately not `secret_vault::in_org_scope`'s
    // client arm: an install is org-wide, so a client's record is out of
    // reach here even though the vault itself can see it.
    let folder_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT sr.folder_id FROM secret_records sr WHERE sr.id = $2 AND sr.org_id = $1",
    )
    .bind(org_id)
    .bind(binding.secret_record_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(folder_id) = folder_id else {
        return Ok(BindOutcome::NoSuchRecord);
    };
    if !secret_vault::folder_access(conn, org_id, &binding.bound_role_ids, folder_id, false).await?
    {
        return Ok(BindOutcome::CategoryDenied);
    }
    // The INSERT selects the record again rather than trusting the value read
    // above: one statement that can only produce a row when (org, record,
    // install) agree is simpler to audit than two that must be read together.
    let sql = "INSERT INTO plugin_credential_bindings
           (id, org_id, install_id, profile, secret_record_id, header_name, bound_by, bound_role_ids)
         SELECT $2, $1, $3, $4, sr.id, $5, $6, $7
           FROM secret_records sr
          WHERE sr.id = $8 AND sr.org_id = $1
         ON CONFLICT (install_id, profile) DO UPDATE
           SET secret_record_id = EXCLUDED.secret_record_id,
               header_name = EXCLUDED.header_name,
               bound_by = EXCLUDED.bound_by,
               bound_role_ids = EXCLUDED.bound_role_ids,
               updated_at = now()
         RETURNING id";
    let id: Option<Uuid> = sqlx::query_scalar(sql)
        .bind(org_id)
        .bind(Uuid::now_v7())
        .bind(install_id)
        .bind(binding.profile)
        .bind(binding.header_name)
        .bind(binding.bound_by)
        .bind(binding.bound_role_ids.clone())
        .bind(binding.secret_record_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(id) = id else {
        // The record was deleted between the two statements. Same answer as a
        // record that was never ours.
        return Ok(BindOutcome::NoSuchRecord);
    };
    let row = load(conn, org_id, install_id, id).await?;
    Ok(BindOutcome::Bound(Box::new(row)))
}

/// Removes one profile's binding. Absent is `NotFound`, so a caller cannot
/// claim it removed a consent that was never there.
pub async fn unbind(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    profile: &str,
) -> DbResult<()> {
    let done = sqlx::query(
        "DELETE FROM plugin_credential_bindings
          WHERE org_id = $1 AND install_id = $2 AND profile = $3",
    )
    .bind(org_id)
    .bind(install_id)
    .bind(profile)
    .execute(&mut *conn)
    .await?;
    if done.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Resolves one profile to sealed bytes, re-checking the grant.
///
/// Cost, stated because phase 2 will call this on the request path: two
/// statements — one indexed read of the binding joined to its record and the
/// `secret` field (the unique `(install_id, profile)` index leads), and one
/// [`secret_vault::folder_access`], which walks the record's folder chain and
/// looks for a grant row. Both are primary-key lookups over tables the size of
/// one org's vault; neither is a scan. It is reached at most once per
/// `http_request` call and only on the credentialed path — a request that
/// names no bound profile never arrives here. A single invocation that makes
/// several calls may cache the outcome for its own lifetime; that is the
/// bridge's decision, and nothing here forbids it.
///
/// The grant check runs on the roles RECORDED AT CONSENT, not on the install
/// and not on the current caller. That is the point: the install is not a
/// principal, so it cannot hold a grant; what can be re-verified later is the
/// authority that consented, and a role whose read was withdrawn stops
/// resolving here.
pub async fn resolve(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    profile: &str,
) -> DbResult<ResolveOutcome> {
    let row = sqlx::query_as::<_, ResolveRow>(
        "SELECT b.header_name, b.secret_record_id, b.bound_role_ids,
                sr.folder_id, f.field_key, f.value_ciphertext
           FROM plugin_credential_bindings b
           JOIN secret_records sr ON sr.id = b.secret_record_id AND sr.org_id = b.org_id
           LEFT JOIN secret_fields f
             ON f.record_id = sr.id AND f.field_key = $4
          WHERE b.org_id = $1 AND b.install_id = $2 AND b.profile = $3",
    )
    .bind(org_id)
    .bind(install_id)
    .bind(profile)
    .bind(CREDENTIAL_FIELD_KEY)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(ResolveOutcome::NotBound);
    };
    // Both columns come from the same LEFT JOIN, so they arrive together; the
    // pair of `let … else` keeps a partially-matched row out of the granted arm
    // instead of substituting a field name for a field that is not there.
    let (Some(field_key), Some(sealed)) = (row.field_key, row.value_ciphertext) else {
        return Ok(ResolveOutcome::FieldMissing);
    };
    if !secret_vault::folder_access(conn, org_id, &row.bound_role_ids, row.folder_id, false).await?
    {
        return Ok(ResolveOutcome::GrantRevoked);
    }
    Ok(ResolveOutcome::Granted(Box::new(ResolvedCredential {
        install_id,
        org_id,
        profile: profile.to_string(),
        header_name: row.header_name,
        record_id: row.secret_record_id,
        field_key,
        sealed,
    })))
}

#[derive(FromRow)]
struct ResolveRow {
    header_name: String,
    secret_record_id: Uuid,
    bound_role_ids: Vec<Uuid>,
    folder_id: Uuid,
    field_key: Option<String>,
    value_ciphertext: Option<Vec<u8>>,
}

async fn load(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    id: Uuid,
) -> DbResult<CredentialBinding> {
    let sql = format!(
        "SELECT {BINDING_COLUMNS}
           FROM plugin_credential_bindings b
           JOIN secret_records sr ON sr.id = b.secret_record_id
          WHERE b.org_id = $1 AND b.install_id = $2 AND b.id = $3"
    );
    sqlx::query_as::<_, CredentialBinding>(&sql)
        .bind(org_id)
        .bind(install_id)
        .bind(id)
        .fetch_one(&mut *conn)
        .await
        .map_err(DbError::from)
}
