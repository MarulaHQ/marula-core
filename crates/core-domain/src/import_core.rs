//! Import framework core (TASKLIST D2): the deterministic, storage-free
//! half of data migration.
//!
//! Purpose: turn raw CSV/JSON rows into validated per-entity specs, decide
//! what each row WOULD do through `ConflictPolicy` as a pure function over
//! caller-supplied lookups, and emit a stable dry-run report
//! (`row_index → errors[] | ok | would_skip | would_update`). This module is
//! the single source of truth for field catalogs, format validation, and
//! conflict semantics; core-db/core-api only persist its decisions and emit
//! events. Idempotency contract: a row whose canonical projection equals the
//! existing row is always a no-op skip under every policy, so re-pushing a
//! batch neither duplicates rows nor spams events.
//!
//! Non-purpose: no I/O whatsoever (no SQL, HTTP, clock, filesystem — all
//! external state arrives as closures); not a migration wizard — there is no
//! field-mapping DSL, headers must already be Meridian's canonical names;
//! not async giant-batch orchestration (worker lane owns that); device assets
//! and further entity families ride in later waves and extend the catalogs
//! additively.

use crate::entities::{Client, Contact, Site};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Hard cap on rows accepted per API call (dry-run body or one chunk post).
/// Giant batches belong to the deferred worker-based lane.
pub const MAX_ROWS_PER_CHUNK: usize = 5_000;
/// Hard cap on request payload size for import bodies, enforced before
/// parsing (mirrors the axum `DefaultBodyLimit` override on the routes).
pub const MAX_CHUNK_BYTES: usize = 1_048_576;

/// Fatal (whole-request) parse/limit failures. Row-level problems do NOT use
/// this type — they flow through [`RowReport`] so one bad row cannot hide
/// the fate of the other 4,999.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("row limit exceeded: {0}")]
    TooManyRows(usize),
    #[error("payload limit exceeded: {0} bytes")]
    TooManyBytes(usize),
    #[error("{0}")]
    Malformed(String),
}

/// Which entity family a batch targets. One batch = exactly one kind; the
/// parent-before-child ordering across batches (clients → sites → contacts)
/// keeps foreign references resolvable without cross-batch transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Client,
    Site,
    Contact,
}

impl std::str::FromStr for EntityKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "client" => Ok(EntityKind::Client),
            "site" => Ok(EntityKind::Site),
            "contact" => Ok(EntityKind::Contact),
            other => Err(format!("unknown entity_kind {other:?}")),
        }
    }
}

impl EntityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityKind::Client => "client",
            EntityKind::Site => "site",
            EntityKind::Contact => "contact",
        }
    }
}

/// Per-entity conflict semantics when an external key already maps to a
/// live row whose content differs from the incoming spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Keep the existing row; report the incoming row as skipped.
    Skip,
    /// Overwrite the existing row with incoming canonical fields.
    Update,
    /// Reject the whole chunk transactionally on any differing conflict.
    Fail,
}

impl std::str::FromStr for ConflictPolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skip" => Ok(ConflictPolicy::Skip),
            "update" => Ok(ConflictPolicy::Update),
            "fail" => Ok(ConflictPolicy::Fail),
            other => Err(format!("unknown conflict_policy {other:?}")),
        }
    }
}

impl ConflictPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            ConflictPolicy::Skip => "skip",
            ConflictPolicy::Update => "update",
            ConflictPolicy::Fail => "fail",
        }
    }
}

// --- raw rows ----------------------------------------------------------------

/// One source row after decoding: generic string-valued field bag keyed by
/// canonical column name, plus its original position for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRow {
    pub index: usize,
    pub fields: BTreeMap<String, String>,
}

/// Wire format selector for an import payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RowFormat {
    Json,
    Csv,
}

/// Parses a JSON array of flat objects. Scalars coerce to strings (`null`
/// behaves exactly like an absent key); arrays/nested objects are rejected
/// because a mapper that silently flattens invents data.
pub fn parse_json_chunk(payload: &serde_json::Value) -> Result<Vec<RawRow>, ImportError> {
    let items = payload.as_array().ok_or_else(|| {
        ImportError::Malformed("payload must be a JSON array of row objects".into())
    })?;
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ImportError::Malformed(format!(
                "row {i}: expected an object, got {}",
                type_name(item)
            ))
        })?;
        let mut fields = BTreeMap::new();
        for (key, value) in obj {
            match value {
                serde_json::Value::Null => {}
                serde_json::Value::String(s) => {
                    fields.insert(key.clone(), s.clone());
                }
                serde_json::Value::Number(n) => {
                    fields.insert(key.clone(), n.to_string());
                }
                serde_json::Value::Bool(b) => {
                    fields.insert(key.clone(), b.to_string());
                }
                other => {
                    return Err(ImportError::Malformed(format!(
                        "row {i}, field {key:?}: unsupported nested {} value",
                        type_name(other)
                    )))
                }
            }
        }
        out.push(RawRow { index: i, fields });
    }
    Ok(out)
}

fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Parses CSV text (RFC 4180 dialect with deliberate leniencies documented
/// below) into raw rows. First non-blank record is the header; every data
/// cell becomes the string under its header name.
///
/// Encoding/format quirks handled deliberately:
/// - leading UTF-8 BOM stripped;
/// - CR, LF, and CRLF all terminate records outside quoted spans;
/// - double-quoted cells may contain commas, quotes (doubled), and newlines;
/// - blank lines anywhere are skipped;
/// - short rows are padded with empty strings (an empty cell and an absent
///   optional column behave identically; required-column checks still fire);
/// - long rows are a fatal error (no silent truncation of invented columns).
///
/// Non-purpose: character-set transcoding — input must already be UTF-8;
/// delimiter/tab dialects and Excel exports are upstream concerns.
pub fn parse_csv_chunk(content: &str) -> Result<Vec<RawRow>, ImportError> {
    let body = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut cell = String::new();
    let mut record: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        cell.push('"');
                    } else {
                        in_quotes = false;
                    }
                }
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => in_quotes = true,
            ',' => {
                record.push(std::mem::take(&mut cell));
            }
            '\n' | '\r' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut cell));
                records.push(std::mem::take(&mut record));
            }
            _ => cell.push(c),
        }
    }
    // Tolerate a final record with no trailing newline; a stray unterminated
    // quote reaches the last cell verbatim and fails downstream validation.
    if !cell.is_empty() || !record.is_empty() || body.ends_with(',') {
        record.push(cell);
        records.push(record);
    }

    let records: Vec<Vec<String>> = records
        .into_iter()
        .filter(|r| !(r.len() == 1 && r[0].is_empty()))
        .collect();

    let mut iter = records.into_iter();
    let header_raw = iter
        .next()
        .ok_or_else(|| ImportError::Malformed("csv payload has no header row".into()))?;
    let header: Vec<String> = header_raw.iter().map(|h| h.trim().to_string()).collect();
    if header.iter().any(String::is_empty) {
        return Err(ImportError::Malformed(
            "csv header contains an empty column name".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for name in &header {
        if !seen.insert(name.clone()) {
            return Err(ImportError::Malformed(format!(
                "csv header repeats column {name:?}"
            )));
        }
    }
    let mut out = Vec::new();
    for (i, rec) in iter.enumerate() {
        if rec.len() > header.len() {
            return Err(ImportError::Malformed(format!(
                "row {}: expected {} columns, found {} (extra data)",
                i,
                header.len(),
                rec.len()
            )));
        }
        let mut fields = BTreeMap::new();
        for (col, name) in header.iter().enumerate() {
            let value = rec.get(col).cloned().unwrap_or_default();
            fields.insert(name.clone(), value);
        }
        out.push(RawRow { index: i, fields });
    }
    Ok(out)
}

/// Enforces the chunk limits. `byte_len` is the encoded payload size the
/// transport observed; rows come from whichever parser ran.
pub fn enforce_limits(rows: usize, byte_len: usize) -> Result<(), ImportError> {
    if rows > MAX_ROWS_PER_CHUNK {
        return Err(ImportError::TooManyRows(rows));
    }
    if byte_len > MAX_CHUNK_BYTES {
        return Err(ImportError::TooManyBytes(byte_len));
    }
    Ok(())
}

// --- specs -------------------------------------------------------------------

/// A validated client company row.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientSpec {
    pub external_key: String,
    pub name: String,
    pub notes: Option<String>,
}

/// A validated site row; `client_key` resolves through the external-id map.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteSpec {
    pub external_key: String,
    pub client_key: String,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
}

/// A validated contact row; `site_key` optionally narrows to a site.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactSpec {
    pub external_key: String,
    pub client_key: String,
    pub site_key: Option<String>,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub title: Option<String>,
    pub is_primary: bool,
}

/// A validated row of whichever kind the batch targets.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSpec {
    Client(ClientSpec),
    Site(SiteSpec),
    Contact(ContactSpec),
}

impl ImportSpec {
    pub fn external_key(&self) -> &str {
        match self {
            ImportSpec::Client(s) => &s.external_key,
            ImportSpec::Site(s) => &s.external_key,
            ImportSpec::Contact(s) => &s.external_key,
        }
    }

    /// Canonical scalar projection used for equality against the stored row.
    /// Deliberately excludes ids and timestamps: two rows are "identical"
    /// when their operator-meaningful fields agree, which is what makes
    /// re-pushes no-op instead of churning updated_at/events.
    pub fn projection(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        match self {
            ImportSpec::Client(s) => {
                m.insert("name".into(), s.name.clone());
                m.insert("notes".into(), s.notes.clone().unwrap_or_default());
            }
            ImportSpec::Site(s) => {
                m.insert("name".into(), s.name.clone());
                for (k, v) in [
                    ("address", &s.address),
                    ("city", &s.city),
                    ("region", &s.region),
                    ("postal_code", &s.postal_code),
                    ("country", &s.country),
                    ("timezone", &s.timezone),
                ] {
                    m.insert(k.into(), v.clone().unwrap_or_default());
                }
                m.insert(
                    "latitude".into(),
                    s.latitude.map(|v| v.to_string()).unwrap_or_default(),
                );
                m.insert(
                    "longitude".into(),
                    s.longitude.map(|v| v.to_string()).unwrap_or_default(),
                );
            }
            ImportSpec::Contact(s) => {
                m.insert("name".into(), s.name.clone());
                for (k, v) in [
                    ("email", &s.email),
                    ("phone", &s.phone),
                    ("title", &s.title),
                ] {
                    m.insert(k.into(), v.clone().unwrap_or_default());
                }
                m.insert("is_primary".into(), s.is_primary.to_string());
            }
        }
        m
    }
}

/// Canonical projection of the STORED row, built from the domain entity the
/// caller loaded inside its org-scoped transaction. Keys mirror
/// [`ImportSpec::projection`] exactly.
pub fn client_projection(c: &Client) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("name".into(), c.name.clone());
    m.insert("notes".into(), c.notes.clone().unwrap_or_default());
    m
}

pub fn site_projection(s: &Site) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("name".into(), s.name.clone());
    for (k, v) in [
        ("address", &s.address),
        ("city", &s.city),
        ("region", &s.region),
        ("postal_code", &s.postal_code),
        ("country", &s.country),
        ("timezone", &s.timezone),
    ] {
        m.insert(k.into(), v.clone().unwrap_or_default());
    }
    m.insert(
        "latitude".into(),
        s.latitude.map(|v| v.to_string()).unwrap_or_default(),
    );
    m.insert(
        "longitude".into(),
        s.longitude.map(|v| v.to_string()).unwrap_or_default(),
    );
    m
}

pub fn contact_projection(c: &Contact) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("name".into(), c.name.clone());
    for (k, v) in [
        ("email", &c.email),
        ("phone", &c.phone),
        ("title", &c.title),
    ] {
        m.insert(k.into(), v.clone().unwrap_or_default());
    }
    m.insert("is_primary".into(), c.is_primary.to_string());
    m
}

// --- validation ---------------------------------------------------------------

const CLIENT_COLUMNS: &[&str] = &["external_key", "name", "notes"];
const SITE_COLUMNS: &[&str] = &[
    "external_key",
    "client_key",
    "name",
    "address",
    "city",
    "region",
    "postal_code",
    "country",
    "latitude",
    "longitude",
    "timezone",
];
const CONTACT_COLUMNS: &[&str] = &[
    "external_key",
    "client_key",
    "site_key",
    "name",
    "email",
    "phone",
    "title",
    "is_primary",
];

/// One validated-or-rejected row, pre-conflict-resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecResult {
    pub index: usize,
    pub external_key: Option<String>,
    pub spec: Result<ImportSpec, Vec<String>>,
}

/// Validates raw rows against the canonical field catalog for `kind`.
/// Unknown columns, missing required fields, malformed scalars, and format
/// violations collect into per-row error lists; they never abort the rest.
pub fn build_specs(kind: EntityKind, rows: &[RawRow]) -> Vec<SpecResult> {
    rows.iter()
        .map(|raw| SpecResult {
            index: raw.index,
            external_key: raw
                .fields
                .get("external_key")
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty()),
            spec: validate_row(kind, raw),
        })
        .collect()
}

fn validate_row(kind: EntityKind, raw: &RawRow) -> Result<ImportSpec, Vec<String>> {
    let catalog = match kind {
        EntityKind::Client => CLIENT_COLUMNS,
        EntityKind::Site => SITE_COLUMNS,
        EntityKind::Contact => CONTACT_COLUMNS,
    };
    let mut errors: Vec<String> = Vec::new();
    for col in raw.fields.keys() {
        if !catalog.contains(&col.as_str()) {
            errors.push(format!("unknown column {col:?}"));
        }
    }
    let get = |name: &str| {
        raw.fields
            .get(name)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };

    let external_key = match get("external_key") {
        Some(k) => match validate_external_key(k) {
            Ok(()) => k.to_string(),
            Err(e) => {
                errors.push(format!("external_key: {e}"));
                String::new()
            }
        },
        None => {
            errors.push("external_key: required".into());
            String::new()
        }
    };
    let name = match get("name") {
        Some(n) => match validate_name(n) {
            Ok(()) => n.to_string(),
            Err(e) => {
                errors.push(format!("name: {e}"));
                String::new()
            }
        },
        None => {
            errors.push("name: required".into());
            String::new()
        }
    };

    macro_rules! opt_text {
        ($col:expr, $max:expr) => {
            match get($col) {
                Some(v) => {
                    if v.chars().count() > $max {
                        errors.push(format!("{}: longer than {} characters", $col, $max));
                    }
                    Some(v.to_string())
                }
                None => None,
            }
        };
    }

    let spec = match kind {
        EntityKind::Client => {
            let notes = opt_text!("notes", 10_000);
            ImportSpec::Client(ClientSpec {
                external_key,
                name,
                notes,
            })
        }
        EntityKind::Site => {
            let client_key = match get("client_key") {
                Some(k) => k.to_string(),
                None => {
                    errors.push("client_key: required".into());
                    String::new()
                }
            };
            let latitude = match get("latitude") {
                Some(v) => match v.parse::<f64>() {
                    Ok(lat) if (-90.0..=90.0).contains(&lat) => Some(lat),
                    _ => {
                        errors.push(format!(
                            "latitude: {v:?} is not a number between -90 and 90"
                        ));
                        None
                    }
                },
                None => None,
            };
            let longitude = match get("longitude") {
                Some(v) => match v.parse::<f64>() {
                    Ok(lon) if (-180.0..=180.0).contains(&lon) => Some(lon),
                    _ => {
                        errors.push(format!(
                            "longitude: {v:?} is not a number between -180 and 180"
                        ));
                        None
                    }
                },
                None => None,
            };
            let timezone = match get("timezone") {
                Some(tz) => match validate_timezone(tz) {
                    Ok(()) => Some(tz.to_string()),
                    Err(e) => {
                        errors.push(format!("timezone: {e}"));
                        None
                    }
                },
                None => None,
            };
            ImportSpec::Site(SiteSpec {
                external_key,
                client_key,
                name,
                address: opt_text!("address", 500),
                city: opt_text!("city", 100),
                region: opt_text!("region", 100),
                postal_code: opt_text!("postal_code", 20),
                country: opt_text!("country", 100),
                latitude,
                longitude,
                timezone,
            })
        }
        EntityKind::Contact => {
            let client_key = match get("client_key") {
                Some(k) => k.to_string(),
                None => {
                    errors.push("client_key: required".into());
                    String::new()
                }
            };
            let email = match get("email") {
                Some(e) => match validate_email(e) {
                    Ok(()) => Some(e.to_string()),
                    Err(msg) => {
                        errors.push(format!("email: {msg}"));
                        None
                    }
                },
                None => None,
            };
            let is_primary = match raw.fields.get("is_primary").map(|v| v.trim()) {
                None | Some("") => false,
                Some(v) => match v.to_ascii_lowercase().as_str() {
                    "true" => true,
                    "false" => false,
                    other => {
                        errors.push(format!("is_primary: {other:?} is not true/false"));
                        false
                    }
                },
            };
            ImportSpec::Contact(ContactSpec {
                external_key,
                client_key,
                site_key: opt_text!("site_key", 256),
                name,
                email,
                phone: opt_text!("phone", 40),
                title: opt_text!("title", 120),
                is_primary,
            })
        }
    };
    if errors.is_empty() {
        Ok(spec)
    } else {
        Err(errors)
    }
}

fn validate_external_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("required".into());
    }
    if key.chars().count() > 256 {
        return Err("longer than 256 characters".into());
    }
    if key.chars().any(char::is_control) {
        return Err("control characters are not allowed".into());
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.chars().count() > 200 {
        return Err("longer than 200 characters".into());
    }
    Ok(())
}

fn validate_timezone(tz: &str) -> Result<(), String> {
    if tz.chars().count() > 64 {
        return Err("longer than 64 characters".into());
    }
    let ok = tz
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '.' | '/'));
    if ok {
        Ok(())
    } else {
        Err("unexpected characters".into())
    }
}

/// Same shape rule the auth handlers apply (`x@y.t`, no spaces, ≥5 chars) so
/// imports never admit addresses the product itself would reject.
pub fn validate_email(email: &str) -> Result<(), String> {
    let ok = email.contains('@') && email.contains('.') && !email.contains(' ') && email.len() >= 5;
    if ok {
        Ok(())
    } else {
        Err(format!("{email:?} is not a valid address"))
    }
}

/// Generic URL sanity for future entity families that grow link fields:
/// absolute http(s) URL with a non-empty remainder. Kept beside the email
/// rule so import validation stays one-stop.
pub fn validate_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    match rest {
        Some(rest) if !rest.trim().is_empty() && !rest.contains(' ') && !rest.contains('\t') => {
            Ok(())
        }
        _ => Err(format!("{url:?} is not an http(s) URL")),
    }
}

// --- planning ------------------------------------------------------------------

/// Resolves parent external keys to internal ids (e.g. a contact's
/// `client_key`). Pure input; the persistence layer backs it with the
/// external_ids mapping table plus any earlier rows of the running chunk.
pub type ParentLookup<'a> = &'a dyn Fn(EntityKind, &str) -> Option<Uuid>;

/// Reports whether `external_key` already maps to an internal id.
pub type KeyMappingLookup<'a> = &'a dyn Fn(&str) -> Option<Uuid>;

/// Loads the canonical projection of the STORED row behind an internal id.
/// Pure input; callers implement it with org-scoped getters so RLS applies.
pub type ProjectionLookup<'a> = &'a dyn Fn(Uuid) -> Option<BTreeMap<String, String>>;

/// What the engine decided one validated row should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// External key unseen — create a fresh row.
    Create,
    /// Mapped, content differs, policy = update — overwrite by internal id.
    Update { internal_id: Uuid },
    /// No change needed: identical canonical projection under any policy.
    SkipNoChange,
    /// Content differs but policy = skip — leave the stored row alone.
    SkipPolicy,
    /// Policy = fail and content differs — the chunk rolls back.
    FailConflict { internal_id: Uuid },
}

/// THE conflict function: total over (policy × presence × difference), and
/// deliberately symmetric with the idempotency doctrine — identical content
/// short-circuits BEFORE policy, making re-pushes eventless no-ops.
pub fn resolve_conflict(
    policy: ConflictPolicy,
    incoming: &ImportSpec,
    existing: Option<(Uuid, BTreeMap<String, String>)>,
) -> Decision {
    match existing {
        None => Decision::Create,
        Some((internal_id, projection)) => {
            if projection == incoming.projection() {
                Decision::SkipNoChange
            } else {
                match policy {
                    ConflictPolicy::Skip => Decision::SkipPolicy,
                    ConflictPolicy::Update => Decision::Update { internal_id },
                    ConflictPolicy::Fail => Decision::FailConflict { internal_id },
                }
            }
        }
    }
}

/// Per-row dry-run/commit report entry. The wire shape contract from D2:
/// every row maps to exactly one of `errors[]`, `ok`, `would_skip`,
/// `would_update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RowReport {
    #[cfg_attr(feature = "openapi", schema(value_type = i64))]
    pub row_index: usize,
    pub external_key: Option<String>,
    pub outcome: RowOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RowOutcome {
    /// Validated clean and unmapped — committing would INSERT.
    Ok,
    /// Mapped, differs, update policy — committing would UPDATE.
    WouldUpdate,
    /// Nothing would happen; the reason distinguishes identical-vs-policy.
    WouldSkip { reason: String },
    /// Validation failed or a fail-policy conflict fired; `errors` carries
    /// human-readable items for operators.
    Failed { errors: Vec<String> },
}

/// Aggregated counters for a chunk (and cumulatively, a batch).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChunkSummary {
    pub total: u32,
    pub ok: u32,
    pub would_update: u32,
    pub would_skip: u32,
    pub failed: u32,
}

impl ChunkSummary {
    pub fn of(reports: &[RowReport]) -> Self {
        let mut s = Self::default();
        for r in reports {
            s.total += 1;
            match &r.outcome {
                RowOutcome::Ok => s.ok += 1,
                RowOutcome::WouldUpdate => s.would_update += 1,
                RowOutcome::WouldSkip { .. } => s.would_skip += 1,
                RowOutcome::Failed { .. } => s.failed += 1,
            }
        }
        s
    }
}

/// An executable row: validated spec plus everything the commit layer needs.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedAction {
    pub index: usize,
    pub action: CommitAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommitAction {
    Insert(ImportSpec),
    Update { internal_id: Uuid, spec: ImportSpec },
}

/// Full chunk plan: the wire report, executable actions, and counters.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkPlan {
    pub reports: Vec<RowReport>,
    pub actions: Vec<PlannedAction>,
    pub summary: ChunkSummary,
}

/// Validates a parsed chunk against `kind`, resolves parents, then applies
/// the conflict policy through [`resolve_conflict`].
///
/// `fail_fast_conflicts`: when the batch was opened with policy=fail the
/// COMMIT path passes true — any `FailConflict` taints the whole chunk for a
/// transactional rollback (atomicity requirement), while dry-run always
/// keeps going so operators see every problem in one pass.
pub fn plan_chunk(
    specs: &[SpecResult],
    policy: ConflictPolicy,
    parent_lookup: ParentLookup<'_>,
    mapping_lookup: KeyMappingLookup<'_>,
    projection_lookup: ProjectionLookup<'_>,
    fail_fast_conflicts: bool,
) -> ChunkPlan {
    let mut reports = Vec::with_capacity(specs.len());
    let mut actions = Vec::new();
    let mut claimed_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in specs {
        let report_index = entry.index;
        let external_key = entry.external_key.clone();
        let outcome = match (&entry.spec, &external_key) {
            (Err(errors), _) => RowOutcome::Failed {
                errors: errors.clone(),
            },
            (_, None) => RowOutcome::Failed {
                errors: vec!["external_key: required".into()],
            },
            (Ok(spec), Some(key)) => {
                if !claimed_keys.insert(key.clone()) {
                    RowOutcome::Failed {
                        errors: vec![format!("duplicate external_key {key:?} within this chunk")],
                    }
                } else if let Err(missing) = check_parents(spec, parent_lookup) {
                    RowOutcome::Failed { errors: missing }
                } else {
                    let existing = mapping_lookup(key)
                        .and_then(|id| projection_lookup(id).map(|projection| (id, projection)));
                    match resolve_conflict(policy, spec, existing) {
                        Decision::Create => {
                            actions.push(PlannedAction {
                                index: report_index,
                                action: CommitAction::Insert(spec.clone()),
                            });
                            RowOutcome::Ok
                        }
                        Decision::Update { internal_id } => {
                            actions.push(PlannedAction {
                                index: report_index,
                                action: CommitAction::Update {
                                    internal_id,
                                    spec: spec.clone(),
                                },
                            });
                            RowOutcome::WouldUpdate
                        }
                        Decision::SkipNoChange => RowOutcome::WouldSkip {
                            reason: "identical to existing row".into(),
                        },
                        Decision::SkipPolicy => RowOutcome::WouldSkip {
                            reason: format!(
                                "existing row differs and conflict_policy={}",
                                policy.as_str()
                            ),
                        },
                        Decision::FailConflict { .. } => {
                            if fail_fast_conflicts {
                                RowOutcome::Failed {
                                    errors: vec![format!(
                                        "conflict: external key {key:?} already imported \
                                         with different content and conflict_policy=fail"
                                    )],
                                }
                            } else {
                                RowOutcome::Failed {
                                    errors: vec![format!(
                                        "conflict: external key {key:?} already imported \
                                         with different content"
                                    )],
                                }
                            }
                        }
                    }
                }
            }
        };
        reports.push(RowReport {
            row_index: report_index,
            external_key,
            outcome,
        });
    }

    let summary = ChunkSummary::of(&reports);
    ChunkPlan {
        reports,
        actions,
        summary,
    }
}

/// A fail-policy chunk may execute only when EVERY row is clean — one
/// conflict taints the whole transaction. Dry-run reporting ignores this.
pub fn chunk_is_executable(plan: &ChunkPlan) -> bool {
    plan.summary.failed == 0
}

fn check_parents(spec: &ImportSpec, lookup: ParentLookup<'_>) -> Result<(), Vec<String>> {
    let mut missing = Vec::new();
    match spec {
        ImportSpec::Client(_) => {}
        ImportSpec::Site(s) => {
            if lookup(EntityKind::Client, &s.client_key).is_none() {
                missing.push(format!(
                    "client_key {:?}: no imported client with this external key",
                    s.client_key
                ));
            }
        }
        ImportSpec::Contact(c) => {
            if lookup(EntityKind::Client, &c.client_key).is_none() {
                missing.push(format!(
                    "client_key {:?}: no imported client with this external key",
                    c.client_key
                ));
            }
            if let Some(site_key) = &c.site_key {
                if lookup(EntityKind::Site, site_key).is_none() {
                    missing.push(format!(
                        "site_key {site_key:?}: no imported site with this external key"
                    ));
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests;
