//! The Marula REST sink: implements [`DmsSink`] against the PUBLIC
//! v1 API only — resource REST over a scoped machine API key
//! (`x-api-key`), the same seam the NinjaOne/Hudu reference connectors
//! use. No Meridian internals are imported; the wire shapes below are
//! the published API contracts (`/api/documents*`, `/api/assets*`,
//! `/api/passwords`, `/api/clients`).
//!
//! Instant-first inheritance (the ADR-0003 / IF7 posture rides the
//! envelope, not the connector):
//!
//! * every create carries a DETERMINISTIC idempotency key derived from
//!   the record's identity (`dms-{source}-{kind}-{handle}`) — a retried
//!   or replayed create returns the SAME stored result, never a twin;
//! * every update carries one too, and its `change_note` rides the
//!   change summary so the audit trail reads "synced from <source>";
//! * the server emits the registered `asset.*` / `document.*` /
//!   `password.*` / `link.*` contract events on these writes — this
//!   crate introduces no new event families;
//! * IF7 undo, provenance, and the audited vault path are server-side
//!   properties of the same routes.
//!
//! Vault boundary: `create_password` is the ONLY credential-carrying
//! call. The secret crosses once, in the request body, and is sealed
//! AES-256-GCM server-side (the 0053 vault path). The sink never logs,
//! caches, or serializes credentials, and `CredentialCreate`'s Debug
//! output is defined by the caller side (`model::Credential` redacts).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    AssetCreate, AssetRecord, AssetUpdate, ClientRecord, CredentialCreate, CredentialReceipt,
    DmsSink, DocumentCreate, DocumentRecord, DocumentUpdate, LayoutRecord, LinkCreate, LinkRecord,
    NewLayout, SinkError,
};
use crate::model::FieldKind;
use crate::model::LinkKind;

/// Credentials + endpoint configuration for [`MarulaRestSink`].
/// Values come from the operator's secret store just-in-time — the
/// crate never persists them. `Debug` redacts the key material.
#[derive(Clone)]
pub struct MarulaConfig {
    /// Meridian core API base URL (e.g. `https://meridian.example`).
    pub base_url: String,
    /// Scoped machine API key (raw `mrd_…` material). Never logged.
    pub api_key: String,
}

impl std::fmt::Debug for MarulaConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarulaConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

/// The public-API sink implementation.
pub struct MarulaRestSink {
    http: reqwest::Client,
    config: MarulaConfig,
}

/// The server's document layout (mirror of the published
/// `DocumentLayout` shape — fields carry `type` per the wire contract).
#[derive(Debug, Deserialize)]
struct WireLayout {
    id: Uuid,
    slug: String,
    name: String,
    #[serde(default)]
    fields: Vec<WireLayoutField>,
    #[serde(default)]
    is_system: bool,
}

#[derive(Debug, Deserialize)]
struct WireLayoutField {
    key: String,
    label: String,
    #[serde(rename = "type", default)]
    field_type: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    position: i32,
    #[serde(default)]
    options: Vec<String>,
}

impl From<WireLayoutField> for crate::model::SyncField {
    fn from(f: WireLayoutField) -> Self {
        Self {
            key: f.key,
            label: f.label,
            kind: FieldKind::parse(&f.field_type).unwrap_or(FieldKind::Text),
            required: f.required,
            position: f.position,
            options: f.options,
        }
    }
}

impl From<WireLayout> for LayoutRecord {
    fn from(l: WireLayout) -> Self {
        Self {
            id: l.id,
            slug: l.slug,
            name: l.name,
            is_system: l.is_system,
            fields: l.fields.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireClient {
    id: Uuid,
    name: String,
}

#[derive(Debug, Deserialize)]
struct WireAsset {
    id: Uuid,
    client_id: Uuid,
    #[serde(default)]
    tag: String,
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    layout_id: Option<Uuid>,
    #[serde(default)]
    fields: Value,
}

impl From<WireAsset> for AssetRecord {
    fn from(a: WireAsset) -> Self {
        Self {
            id: a.id,
            client_id: a.client_id,
            tag: a.tag,
            name: a.name,
            status: a.status,
            layout_id: a.layout_id,
            fields: a.fields,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireDocument {
    id: Uuid,
    #[serde(default)]
    client_id: Option<Uuid>,
    #[serde(default)]
    layout_id: Option<Uuid>,
    #[serde(default)]
    fields: Value,
    title: String,
}

/// GET /api/documents/{id} shape: the document flattened with the
/// current version's body.
#[derive(Debug, Deserialize)]
struct WireDocumentView {
    #[serde(flatten)]
    document: WireDocument,
    #[serde(default)]
    body: String,
}

impl From<WireDocument> for DocumentRecord {
    fn from(d: WireDocument) -> Self {
        Self {
            id: d.id,
            client_id: d.client_id,
            title: d.title,
            layout_id: d.layout_id,
            fields: d.fields,
            body: String::new(),
        }
    }
}

impl From<WireDocumentView> for DocumentRecord {
    fn from(v: WireDocumentView) -> Self {
        let mut record: DocumentRecord = v.document.into();
        record.body = v.body;
        record
    }
}

/// Mutation responses flatten the entity and add the instant-first
/// `mutation` metadata; only the entity side is read here.
#[derive(Debug, Deserialize)]
struct WireEntityEnvelope<T> {
    #[serde(flatten)]
    entity: T,
}

#[derive(Debug, Deserialize)]
struct WireLink {
    id: Uuid,
    from_kind: String,
    from_id: Uuid,
    to_kind: String,
    to_id: Uuid,
}

impl TryFrom<WireLink> for LinkRecord {
    type Error = SinkError;

    fn try_from(l: WireLink) -> Result<Self, Self::Error> {
        Ok(Self {
            id: l.id,
            from_kind: parse_kind(&l.from_kind)?,
            from_id: l.from_id,
            to_kind: parse_kind(&l.to_kind)?,
            to_id: l.to_id,
        })
    }
}

fn parse_kind(s: &str) -> Result<LinkKind, SinkError> {
    Ok(match s {
        "document" => LinkKind::Document,
        "asset" => LinkKind::Asset,
        "password" => LinkKind::Password,
        "client" => LinkKind::Client,
        "ticket" => LinkKind::Ticket,
        "kb_article" => LinkKind::KbArticle,
        other => {
            return Err(SinkError::InvalidRequest(format!(
                "unknown link kind {other:?}"
            )))
        }
    })
}

fn kind_str(kind: LinkKind) -> &'static str {
    kind.as_str()
}

impl MarulaRestSink {
    pub fn new(config: MarulaConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    /// Test/observability hook: bring your own HTTP client.
    pub fn with_http(config: MarulaConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/api/{}",
            self.config.base_url.trim_end_matches('/'),
            path
        )
    }

    /// One REST round trip: auth header, typed error mapping, JSON
    /// decode.
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, SinkError> {
        let mut request = self
            .http
            .request(method, self.url(path))
            .header("x-api-key", &self.config.api_key)
            .header("Accept", "application/json");
        if let Some(body) = &body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| SinkError::Network(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| SinkError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(map_error(status.as_u16(), &text));
        }
        serde_json::from_str(&text)
            .map_err(|e| SinkError::Network(format!("unparseable Meridian response: {e}")))
    }

    async fn list<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, SinkError> {
        let value = self.call(reqwest::Method::GET, path, None).await?;
        serde_json::from_value(value).map_err(|e| SinkError::Network(format!("list {path}: {e}")))
    }
}

fn map_error(status: u16, body: &str) -> SinkError {
    let detail = body.chars().take(500).collect::<String>();
    match status {
        401 | 403 => SinkError::Unauthorized(format!("Meridian returned {status}: {detail}")),
        404 => SinkError::NotFound(detail),
        409 => SinkError::Conflict(detail),
        429 => SinkError::RateLimited(detail),
        s => SinkError::Network(format!("Meridian returned {s}: {detail}")),
    }
}

#[async_trait]
impl DmsSink for MarulaRestSink {
    fn name(&self) -> &str {
        "meridian"
    }

    async fn list_layouts(&self) -> Result<Vec<LayoutRecord>, SinkError> {
        self.list::<WireLayout>("documents/layouts")
            .await
            .map(|ls| ls.into_iter().map(Into::into).collect())
    }

    async fn create_layout(&self, layout: &NewLayout) -> Result<LayoutRecord, SinkError> {
        let fields: Vec<Value> = layout
            .fields
            .iter()
            .map(|f| {
                json!({
                    "key": f.key, "label": f.label, "type": f.kind.as_str(),
                    "required": f.required, "position": f.position,
                    "options": f.options,
                })
            })
            .collect();
        let body = json!({
            "slug": layout.slug,
            "name": layout.name,
            "description": layout.description,
            "icon": layout.icon,
            "fields": fields,
            "idempotency_key": format!("dms-layout-{}", layout.slug),
        });
        let response: WireEntityEnvelope<WireLayout> = serde_json::from_value(
            self.call(reqwest::Method::POST, "documents/layouts", Some(body))
                .await?,
        )
        .map_err(|e| SinkError::Network(format!("layout create decode: {e}")))?;
        Ok(response.entity.into())
    }

    async fn list_clients(&self) -> Result<Vec<ClientRecord>, SinkError> {
        self.list::<WireClient>("clients").await.map(|cs| {
            cs.into_iter()
                .map(|c| ClientRecord {
                    id: c.id,
                    name: c.name,
                })
                .collect()
        })
    }

    async fn list_assets(&self) -> Result<Vec<AssetRecord>, SinkError> {
        self.list::<WireAsset>("assets")
            .await
            .map(|as_| as_.into_iter().map(Into::into).collect())
    }

    async fn create_asset(&self, plan: &AssetCreate) -> Result<AssetRecord, SinkError> {
        let mut body = json!({
            "client_id": plan.client_id,
            "tag": plan.tag,
            "name": plan.name,
            "layout_id": plan.layout_id,
            "fields": plan.fields,
            "change_summary": plan.change_note,
        });
        merge_instant(&mut body, &plan.idempotency_key);
        let response: WireEntityEnvelope<WireAsset> = serde_json::from_value(
            self.call(reqwest::Method::POST, "assets", Some(body))
                .await?,
        )
        .map_err(|e| SinkError::Network(format!("asset create decode: {e}")))?;
        Ok(response.entity.into())
    }

    async fn update_asset(&self, id: Uuid, plan: &AssetUpdate) -> Result<AssetRecord, SinkError> {
        let mut body = json!({});
        if let Some(name) = &plan.name {
            body["name"] = json!(name);
        }
        if let Some(status) = &plan.status {
            body["status"] = json!(status);
        }
        if let Some(layout_id) = plan.layout_id {
            body["layout_id"] = json!(layout_id);
        }
        if let Some(fields) = &plan.fields {
            body["fields"] = fields.clone();
        }
        if let Some(note) = &plan.change_note {
            body["change_summary"] = json!(note);
        }
        merge_instant(&mut body, &plan.idempotency_key);
        let response: WireEntityEnvelope<WireAsset> = serde_json::from_value(
            self.call(reqwest::Method::PATCH, &format!("assets/{id}"), Some(body))
                .await?,
        )
        .map_err(|e| SinkError::Network(format!("asset update decode: {e}")))?;
        Ok(response.entity.into())
    }

    async fn list_documents(&self) -> Result<Vec<DocumentRecord>, SinkError> {
        // List responses carry document metadata only (no body); the
        // engine pulls bodies lazily via get_document.
        self.list::<WireDocument>("documents")
            .await
            .map(|ds| ds.into_iter().map(Into::into).collect())
    }

    async fn get_document(&self, id: Uuid) -> Result<DocumentRecord, SinkError> {
        let response: WireEntityEnvelope<WireDocumentView> = serde_json::from_value(
            self.call(reqwest::Method::GET, &format!("documents/{id}"), None)
                .await?,
        )
        .map_err(|e| SinkError::Network(format!("document get decode: {e}")))?;
        Ok(response.entity.into())
    }

    async fn create_document(&self, plan: &DocumentCreate) -> Result<DocumentRecord, SinkError> {
        let mut body = json!({
            "title": plan.title,
            "client_id": plan.client_id,
            "layout_id": plan.layout_id,
            "fields": plan.fields,
            "body": plan.body,
            "change_summary": plan.change_note,
        });
        merge_instant(&mut body, &plan.idempotency_key);
        let response: WireEntityEnvelope<WireDocument> = serde_json::from_value(
            self.call(reqwest::Method::POST, "documents", Some(body))
                .await?,
        )
        .map_err(|e| SinkError::Network(format!("document create decode: {e}")))?;
        let mut record: DocumentRecord = response.entity.into();
        record.body = plan.body.clone();
        Ok(record)
    }

    async fn update_document(
        &self,
        id: Uuid,
        plan: &DocumentUpdate,
    ) -> Result<DocumentRecord, SinkError> {
        let mut body = json!({});
        if let Some(title) = &plan.title {
            body["title"] = json!(title);
        }
        if let Some(layout_id) = plan.layout_id {
            body["layout_id"] = json!(layout_id);
        }
        if let Some(fields) = &plan.fields {
            body["fields"] = fields.clone();
        }
        if let Some(body_text) = &plan.body {
            body["body"] = json!(body_text);
        }
        if let Some(note) = &plan.change_note {
            body["change_summary"] = json!(note);
        }
        merge_instant(&mut body, &plan.idempotency_key);
        let response: WireEntityEnvelope<WireDocument> = serde_json::from_value(
            self.call(
                reqwest::Method::PATCH,
                &format!("documents/{id}"),
                Some(body),
            )
            .await?,
        )
        .map_err(|e| SinkError::Network(format!("document update decode: {e}")))?;
        Ok(response.entity.into())
    }

    async fn list_links(
        &self,
        from_kind: LinkKind,
        from_id: Uuid,
    ) -> Result<Vec<LinkRecord>, SinkError> {
        let path = link_path(from_kind, from_id, "links");
        let links = self.list::<WireLink>(&path).await?;
        links.into_iter().map(TryInto::try_into).collect()
    }

    async fn create_link(&self, plan: &LinkCreate) -> Result<LinkRecord, SinkError> {
        let mut body = json!({
            "to_kind": kind_str(plan.to_kind),
            "to_id": plan.to_id,
        });
        merge_instant(&mut body, &plan.idempotency_key);
        let path = link_path(plan.from_kind, plan.from_id, "links");
        let response: WireEntityEnvelope<WireLink> =
            serde_json::from_value(self.call(reqwest::Method::POST, &path, Some(body)).await?)
                .map_err(|e| SinkError::Network(format!("link create decode: {e}")))?;
        response.entity.try_into()
    }

    async fn create_password(
        &self,
        plan: &CredentialCreate,
    ) -> Result<CredentialReceipt, SinkError> {
        // The audited vault path: the secret crosses ONCE here and is
        // sealed server-side. No logging, no caching, no echo.
        let mut body = json!({
            "client_id": plan.client_id,
            "title": plan.title,
            "username": plan.username,
            "url": plan.url,
            "secret": plan.secret,
            "notes": plan.notes,
            "expires_at": plan.expires_at,
        });
        merge_instant(&mut body, &plan.idempotency_key);
        let value = self
            .call(reqwest::Method::POST, "passwords", Some(body))
            .await?;
        let password_id = value
            .get("id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| SinkError::Network("password create response carried no id".into()))?;
        Ok(CredentialReceipt { password_id })
    }
}

fn merge_instant(body: &mut Value, key: &str) {
    if let Some(map) = body.as_object_mut() {
        map.insert("idempotency_key".into(), json!(key));
    }
}

fn link_path(from_kind: LinkKind, from_id: Uuid, suffix: &str) -> String {
    match from_kind {
        LinkKind::Asset => format!("assets/{from_id}/{suffix}"),
        _ => format!("documents/{from_id}/{suffix}"),
    }
}
