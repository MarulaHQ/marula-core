// Local wire types for the template registry (GET/POST/PATCH /api/v1/templates…).
//
// Why local mirrors instead of `components["schemas"]`: the pinned OpenAPI
// schema carries two utoipa name collisions — `TemplateMutationResponse`
// resolves to the PROJECT template's schema (id/name/spec, no subject, body,
// slug or variables) and `UpdateTemplateRequest` resolves to a merged shape
// (entity_kind/mapping) that is not what the templates route accepts. The
// server's true shapes are `crates/core-api/src/routes/templates.rs`. So the
// editor mirrors the route structs here and casts at the generated-client
// call boundary — the same approach the email settings page records in its
// header comment. When the SDK regenerates collision-free, these can be
// replaced by the generated types one for one.

/** One declared template variable (mirrors `TemplateVariableView`). */
export interface TemplateVariable {
  key: string;
  label: string;
}

/** One template row (mirrors `TemplateView` in routes/templates.rs). */
export interface TemplateRow {
  id: string;
  org_id: string;
  kind: string;
  slug: string;
  name: string;
  description: string;
  subject: string | null;
  body: string;
  variables: TemplateVariable[];
  /** Shipped preset tag, null for org-created rows. */
  builtin: string | null;
  /** Shipped defaults are immutable: fork, never edit in place. */
  is_system: boolean;
  /** For a forked copy: the origin's builtin tag (or its slug). */
  forked_from: string | null;
  created_at: string;
  updated_at: string;
}

/** The GET /api/v1/templates response (mirrors `TemplatesListResponse`). */
export interface TemplatesList {
  templates: TemplateRow[];
  /** The closed per-kind variable vocabulary: kind → placeholder keys. */
  vocabulary: { kind: string; variables: string[] }[];
}

/** `mutation` metadata on every instant-first mutation response (IF2). */
export interface MutationMeta {
  seq: number;
  /** Echo in the NEXT write's `version` field (If-Match style). */
  version: number;
  updated_at: string;
}

/** POST/PATCH/DELETE response: the row at the top level plus `mutation`. */
export interface TemplateMutationResult {
  mutation: MutationMeta;
}

/** PATCH /api/v1/templates/{id} body (mirrors `UpdateTemplateRequest`). */
export interface UpdateTemplateBody {
  idempotency_key?: string;
  version?: number;
  name?: string;
  description?: string;
  subject?: string | null;
  body?: string;
  variables?: TemplateVariable[];
}

/** POST /api/v1/templates/{id}/fork body (mirrors `ForkTemplateRequest`). */
export interface ForkTemplateBody {
  idempotency_key?: string;
  slug: string;
  name: string;
}

/** POST /api/v1/templates/{id}/restore body (mirrors `RestoreTemplateRequest`). */
export interface RestoreTemplateBody {
  idempotency_key?: string;
  version?: number;
}

/** What a fork returns, once the collision cast is through. */
export interface ForkedTemplate extends TemplateRow {
  mutation: MutationMeta;
}
