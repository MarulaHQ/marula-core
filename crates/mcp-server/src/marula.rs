//! The MCP service: the catalog, the capability filter, and the proxy.
//!
//! Built on `rmcp`'s `ServerHandler`, so framing, version negotiation and
//! transport semantics belong to the SDK. What stays here is Marula's:
//! which tools, resources and prompts exist (derived from the committed
//! OpenAPI snapshot, the resource families and the prompt table, all filtered
//! by the org's grant registry) and who may use them (the caller's own
//! session, forwarded to core-api, which authorizes every read).
//!
//! Three surfaces, one authorization rule:
//!
//! - **tools** — the derived read catalog;
//! - **resources** — `marula://tickets/{id}` and its two siblings, each
//!   resolving through the *same* catalog tool as the equivalent tool call, so
//!   the proxied path, the capability and the core-api check are identical;
//! - **prompts** — in-code workflow templates, listed only when the caller can
//!   perform the first read the workflow needs.
//!
//! Tasks are deliberately not advertised. The 2026-07-28 revision defines them
//! as an extension with its own server-side lifecycle and cancellation, and no
//! caller of this surface needs to poll one yet. `tasks/*` therefore answers
//! with the SDK's method-not-found, which is the truthful answer.

use crate::api::CoreApi;
use crate::catalog::{self, Tool as CatalogTool};
use crate::prompts::{self, Prompt as PromptDefinition};
use crate::resources::{self, Family};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ContentBlock, GetPromptRequestParams,
        GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult,
        ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
        Prompt as McpPrompt, PromptArgument, PromptMessage, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, Resource as McpResource, ResourceContents,
        ResourceTemplate, Role, ServerCapabilities, ServerConfig, Tool as McpTool,
    },
    service::RequestContext,
    ErrorData as McpError, RoleServer, ServerHandler,
};
use std::collections::HashSet;
use std::sync::Arc;

/// One published resource family paired with the catalog tool that reads it.
#[derive(Clone)]
struct ResourceEntry {
    family: &'static Family,
    tool: CatalogTool,
}

/// Requests carry the caller's bearer either in the HTTP request parts or, for
/// stdio, in the environment the adapter was started with.
#[derive(Clone)]
pub struct MarulaServer {
    api: CoreApi,
    tools: Arc<Vec<CatalogTool>>,
    resources: Arc<Vec<ResourceEntry>>,
    stdio_token: Option<String>,
}

impl MarulaServer {
    pub fn new(
        base_url: &str,
        spec: &serde_json::Value,
        catalog_spec: &serde_json::Value,
        stdio_token: Option<String>,
    ) -> Self {
        let tools = catalog::build_catalog(spec, catalog_spec);
        // A family is published only when its catalog tool exists, so the
        // resource surface can never outlive the operation behind it.
        let resources = resources::FAMILIES
            .iter()
            .filter_map(|family| {
                tools
                    .iter()
                    .find(|tool| tool.name == family.tool)
                    .map(|tool| ResourceEntry {
                        family,
                        tool: tool.clone(),
                    })
            })
            .collect();
        Self {
            api: CoreApi::new(base_url),
            tools: Arc::new(tools),
            resources: Arc::new(resources),
            stdio_token,
        }
    }

    /// The full derived catalog, for tests and diagnostics.
    pub fn catalog(&self) -> &[CatalogTool] {
        &self.tools
    }

    /// The resource families this build publishes, for tests and diagnostics.
    pub fn resource_families(&self) -> Vec<&'static str> {
        self.resources
            .iter()
            .map(|entry| entry.family.family)
            .collect()
    }

    fn bearer(&self, context: &RequestContext<RoleServer>) -> Option<String> {
        if let Some(parts) = context.extensions.get::<http::request::Parts>() {
            if let Some(value) = parts
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
            {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    let token = token.trim();
                    if !token.is_empty() {
                        return Some(token.to_string());
                    }
                }
            }
        }
        self.stdio_token.clone()
    }

    /// Granted capability namespaces for this session. An unreadable context is
    /// an empty set, so deny-by-default holds on every failure mode.
    async fn granted(&self, token: Option<&str>) -> HashSet<String> {
        let Some(token) = token else {
            return HashSet::new();
        };
        let Some(session) = self.api.session(token).await else {
            return HashSet::new();
        };
        session
            .mcp_capabilities
            .into_iter()
            .filter_map(|capability| {
                capability
                    .strip_suffix(":read")
                    .map(|namespace| namespace.to_string())
            })
            .collect()
    }

    fn visible(&self, granted: &HashSet<String>) -> Vec<&CatalogTool> {
        self.tools
            .iter()
            .filter(|tool| granted.contains(namespace_of(&tool.capability)))
            .collect()
    }

    fn visible_resources(&self, granted: &HashSet<String>) -> Vec<&ResourceEntry> {
        self.resources
            .iter()
            .filter(|entry| granted.contains(namespace_of(&entry.tool.capability)))
            .collect()
    }

    /// Resolves a concrete URI to its family and record id. The id is a plain
    /// identifier, never an encoded path: see [`resources::parse`].
    fn lookup_uri(&self, uri: &str) -> Result<(&ResourceEntry, String), String> {
        let reference = resources::parse(uri)?;
        let entry = self
            .resources
            .iter()
            .find(|entry| entry.family.family == reference.family.family)
            .ok_or_else(|| {
                format!(
                    "resource family {} is not served by this build",
                    reference.family.family
                )
            })?;
        Ok((entry, reference.id))
    }

    fn as_mcp_tool(tool: &CatalogTool) -> McpTool {
        let schema = tool.input_schema.as_object().cloned().unwrap_or_else(|| {
            let mut fallback = serde_json::Map::new();
            fallback.insert("type".into(), serde_json::json!("object"));
            fallback.insert("properties".into(), serde_json::json!({}));
            fallback
        });
        let mut mcp = McpTool::new(
            tool.name.clone(),
            tool.description.clone(),
            Arc::new(schema),
        );
        mcp.annotations = None;
        mcp
    }

    fn as_mcp_template(entry: &ResourceEntry) -> ResourceTemplate {
        ResourceTemplate::new(
            resources::uri_template(entry.family),
            entry.family.family.to_string(),
        )
        .with_title(entry.family.title)
        .with_description(entry.family.description)
        .with_mime_type("application/json")
    }

    fn as_mcp_prompt(prompt: &PromptDefinition) -> McpPrompt {
        let argument = PromptArgument::new(prompt.argument.name)
            .with_title(prompt.argument.title)
            .with_description(prompt.argument.description)
            .with_required(true);
        McpPrompt::new(prompt.name, Some(prompt.description), Some(vec![argument]))
            .with_title(prompt.title)
    }

    /// The resource link a prompt carries, so the client can attach the record
    /// the workflow starts from without a second round trip.
    fn prompt_link(&self, prompt: &PromptDefinition, id: &str) -> Option<McpResource> {
        let entry = self
            .resources
            .iter()
            .find(|entry| entry.family.family == prompt.resource)?;
        Some(
            McpResource::new(
                resources::uri_for(entry.family, id),
                format!("{} {id}", entry.family.title),
            )
            .with_mime_type("application/json"),
        )
    }
}

/// The namespace side of a capability slug (`tickets:read` → `tickets`).
fn namespace_of(capability: &str) -> &str {
    capability.strip_suffix(":read").unwrap_or(capability)
}

/// The one refusal message every surface gives for an ungranted capability, so
/// a tool result and a resource error read the same.
fn not_granted_message(capability: &str) -> String {
    format!(
        "capability {capability} is not granted for this organization (deny-by-default registry)"
    )
}

fn not_granted(capability: &str) -> McpError {
    McpError::invalid_request(not_granted_message(capability), None)
}

const NO_SESSION: &str =
    "this request carries no session token; the adapter holds no credential of its own";

fn no_session() -> McpError {
    McpError::invalid_request(NO_SESSION, None)
}

impl ServerHandler for MarulaServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("marula-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Read tools, resources and prompts derived from the Marula Core API. A \
             surface exists only when this organization has granted its capability, and \
             every read runs as the caller's own session. Resources are addressed as \
             marula://tickets/{id}, marula://clients/{id} and marula://documents/{id}; \
             prompts are workflows that start from one of those records. Writes are not \
             exposed here: they enter through the agent proposal pipeline for human \
             approval.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let token = self.bearer(&context);
        let granted = self.granted(token.as_deref()).await;
        let tools = self
            .visible(&granted)
            .into_iter()
            .map(Self::as_mcp_tool)
            .collect();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, McpError> {
        let Some(token) = self.bearer(&context) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(NO_SESSION)]).into());
        };
        let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.name == request.name.as_ref())
        else {
            return Err(McpError::invalid_params(
                format!("unknown tool: {}", request.name),
                None,
            ));
        };
        let granted = self.granted(Some(&token)).await;
        if !granted.contains(namespace_of(&tool.capability)) {
            return Ok(
                CallToolResult::error(vec![ContentBlock::text(not_granted_message(
                    &tool.capability,
                ))])
                .into(),
            );
        }
        let arguments = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or_else(|| serde_json::json!({}));
        let path = match catalog::resolve_path(tool, &arguments) {
            Ok(path) => path,
            Err(message) => return Err(McpError::invalid_params(message, None)),
        };
        let query = catalog::query_string(tool, &arguments);
        match self.api.get(&path, &query, &token).await {
            Ok(value) => {
                let text = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
            }
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "core-api answered {}: {}",
                error.status, error.body
            ))])
            .into()),
        }
    }

    /// Empty on purpose. A resource is an instance, and this adapter does not
    /// enumerate an organization's records: that is a paginated, RLS-scoped
    /// query the tool catalog already answers. The templates below are the
    /// discovery surface.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::default())
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        let token = self.bearer(&context);
        let granted = self.granted(token.as_deref()).await;
        let resource_templates = self
            .visible_resources(&granted)
            .into_iter()
            .map(Self::as_mcp_template)
            .collect();
        Ok(ListResourceTemplatesResult {
            resource_templates,
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let Some(token) = self.bearer(&context) else {
            return Err(no_session());
        };
        let (entry, id) = self
            .lookup_uri(&request.uri)
            .map_err(|message| McpError::invalid_params(message, None))?;
        let granted = self.granted(Some(&token)).await;
        if !granted.contains(namespace_of(&entry.tool.capability)) {
            return Err(not_granted(&entry.tool.capability));
        }
        let path = catalog::resolve_path(&entry.tool, &serde_json::json!({ "id": id }))
            .map_err(|message| McpError::internal_error(message, None))?;
        match self.api.get(&path, "", &token).await {
            Ok(value) => {
                let text =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                Ok(
                    ReadResourceResult::new(vec![ResourceContents::text(
                        text,
                        request.uri.clone(),
                    )
                    .with_mime_type("application/json")])
                    .into(),
                )
            }
            Err(error) => Err(McpError::internal_error(
                format!("core-api answered {}: {}", error.status, error.body),
                None,
            )),
        }
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let token = self.bearer(&context);
        let granted = self.granted(token.as_deref()).await;
        let prompts = prompts::PROMPTS
            .iter()
            .filter(|prompt| granted.contains(namespace_of(prompt.requires)))
            .map(Self::as_mcp_prompt)
            .collect();
        Ok(ListPromptsResult {
            prompts,
            ..Default::default()
        })
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let Some(token) = self.bearer(&context) else {
            return Err(no_session());
        };
        let prompt = prompts::by_name(&request.name).ok_or_else(|| {
            McpError::invalid_params(format!("unknown prompt: {}", request.name), None)
        })?;
        let granted = self.granted(Some(&token)).await;
        if !granted.contains(namespace_of(prompt.requires)) {
            return Err(not_granted(prompt.requires));
        }
        let id = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get(prompt.argument.name))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "prompt {} requires the {} argument",
                        prompt.name, prompt.argument.name
                    ),
                    None,
                )
            })?;
        if !resources::is_record_id(id) {
            return Err(McpError::invalid_params(
                "record id must be 1..=64 ASCII alphanumerics, '-' or '_'",
                None,
            ));
        }
        let mut messages = vec![PromptMessage::new(
            Role::User,
            ContentBlock::text(prompts::body_for(prompt, id)),
        )];
        if let Some(link) = self.prompt_link(prompt, id) {
            messages.push(PromptMessage::new(
                Role::User,
                ContentBlock::ResourceLink(link),
            ));
        }
        Ok(GetPromptResult::new(messages)
            .with_description(prompt.description)
            .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> MarulaServer {
        let spec = serde_json::json!({ "paths": {
            "/api/clients": { "get": {
                "operationId": "clients_list",
                "summary": "List clients",
                "responses": { "200": { "description": "ok" } }
            }},
            "/api/tickets": { "get": {
                "operationId": "tickets_list",
                "summary": "List tickets",
                "responses": { "200": { "description": "ok" } }
            }}
        }});
        let catalog = serde_json::json!({ "version": 1, "families": [
            { "namespace": "clients", "capability": "clients:read", "tools": [
                { "name": "clients_list", "path": "/api/clients", "capability": "clients:read" }
            ]},
            { "namespace": "tickets", "capability": "tickets:read", "tools": [
                { "name": "tickets_list", "path": "/api/tickets", "capability": "tickets:read" }
            ]}
        ]});
        MarulaServer::new("http://127.0.0.1:9", &spec, &catalog, None)
    }

    fn real_server() -> MarulaServer {
        MarulaServer::new(
            "http://127.0.0.1:9",
            &crate::openapi_spec(),
            &crate::mcp_catalog_spec(),
            None,
        )
    }

    fn granted(namespaces: &[&str]) -> HashSet<String> {
        namespaces.iter().map(|ns| (*ns).to_string()).collect()
    }

    #[test]
    fn the_catalog_is_derived_only_from_granted_families() {
        let server = server();
        assert_eq!(server.catalog().len(), 2);
        let visible: Vec<&str> = server
            .visible(&granted(&["clients"]))
            .into_iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(visible, vec!["clients_list"]);
    }

    #[test]
    fn no_grants_means_no_tools() {
        let server = server();
        assert!(server.visible(&HashSet::new()).is_empty());
    }

    #[test]
    fn the_advertised_protocol_is_the_sdk_default_not_an_echo() {
        let info = server().get_info();
        assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
        assert!(info.capabilities.tools.is_some());
    }

    /// The surface the revision defines beyond tools is advertised, and the
    /// tasks extension is not: a capability that answers method-not-found is
    /// worse than one that was never offered.
    #[test]
    fn resources_and_prompts_are_advertised_and_tasks_are_not() {
        let info = server().get_info();
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.prompts.is_some());
        assert!(info.capabilities.experimental.is_none());
    }

    /// Every published family resolves through a tool the real catalog
    /// contains, and every prompt's first read exists as a capability. A
    /// rename in the artifact fails here instead of shipping a dead URI.
    #[test]
    fn the_published_surface_matches_the_real_catalog() {
        let server = real_server();
        assert_eq!(
            server.resource_families(),
            vec!["tickets", "clients", "documents"]
        );
        for entry in server.resources.iter() {
            assert_eq!(
                entry.tool.capability,
                format!("{}:read", entry.family.family)
            );
        }
        for prompt in prompts::PROMPTS {
            let family = resources::FAMILIES
                .iter()
                .find(|family| family.family == prompt.resource)
                .expect("prompt names a published family");
            let tool = server
                .tools
                .iter()
                .find(|tool| tool.name == family.tool)
                .expect("family tool is in the catalog");
            assert_eq!(tool.capability, prompt.requires);
        }
    }

    #[test]
    fn resource_templates_follow_the_grant() {
        let server = real_server();
        let visible: Vec<&'static str> = server
            .visible_resources(&granted(&["tickets"]))
            .into_iter()
            .map(|entry| entry.family.family)
            .collect();
        assert_eq!(visible, vec!["tickets"]);
        assert!(server.visible_resources(&HashSet::new()).is_empty());
    }

    #[test]
    fn a_uri_resolves_to_its_family_and_id() {
        let server = real_server();
        let (entry, id) = server
            .lookup_uri("marula://clients/018f0a5e-0000-7000-8000-000000000002")
            .unwrap();
        assert_eq!(entry.family.family, "clients");
        assert_eq!(entry.tool.path, "/api/clients/{id}");
        assert_eq!(id, "018f0a5e-0000-7000-8000-000000000002");
        assert!(server.lookup_uri("marula://invoices/abc").is_err());
    }

    #[test]
    fn prompt_visibility_follows_the_first_read() {
        let visible: Vec<&str> = prompts::PROMPTS
            .iter()
            .filter(|prompt| granted(&["documents"]).contains(namespace_of(prompt.requires)))
            .map(|prompt| prompt.name)
            .collect();
        assert_eq!(visible, vec!["document_review"]);
    }
}
