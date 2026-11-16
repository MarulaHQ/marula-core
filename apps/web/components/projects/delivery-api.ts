// Typed transport for the project-delivery endpoints these surfaces call.
//
// Every endpoint function below rides the generated client (`api.api`): the
// path, its query and body, and the response shape all come from
// `@marula-core/sdk`, so a spec rename fails `tsc` here instead of 404ing in
// the browser. The exported interfaces are the narrow reading these surfaces
// need, derived from the schema types (`Solid`, `Pick`, `Omit`) rather than
// hand-copied, so a shape cannot drift silently from the contract.
//
// One thing still uses the raw `deliveryFetch` helper: `template-editor.tsx`
// reads a gallery card through it with a runtime-built path. The export stays
// for that caller; it is not part of this module's own endpoint set.
//
// `deliveryFetch` also keeps the auth wait that a full page load needs. The
// generated client only sends the bearer once the restore has stored it, and
// every surface importing this module already gates its first call on
// `auth.status === "authed"`, so the typed functions do not repeat that wait.
//
// The shared-portfolio read is NOT unauthenticated: the route requires a
// signed-in principal with `projects:view` and scopes the token to that org
// (crates/core-api/src/routes/project_delivery/portfolio.rs), so it rides the
// same authenticated client as everything else and keeps sending the staff
// bearer. Do not move it onto an anonymous transport.

import type { components } from "@marula-core/sdk";
import { api, authReady, loadTokens, resolveApiBaseUrl } from "../../lib/api";

type Schemas = components["schemas"];
type Wire<K extends keyof Schemas> = Schemas[K];

/** A wire shape read as present: the API emits every declared key (a nullable
 * field is a value, not a missing one), which is the claim the surfaces make. */
type Solid<T> = { [K in keyof T]-?: Exclude<T[K], undefined> };

// --- shared shapes ----------------------------------------------------------

export type GalleryTemplate = Solid<Wire<"GalleryTemplateView">>;

export type PreviewTask = Solid<Wire<"PreviewTask">>;

export type TemplatePreview = Omit<
  Solid<Wire<"TemplatePreviewView">>,
  "phases" | "tasks" | "roles"
> & {
  phases: Solid<Wire<"PreviewPhase">>[];
  tasks: PreviewTask[];
  roles: Solid<Wire<"PreviewRole">>[];
};

export type ValidationIssue = Omit<Solid<Wire<"ValidationIssue">>, "severity"> & {
  severity: "error" | "warning" | "info";
};

export type ValidationReport = Omit<Solid<Wire<"ValidationReport">>, "issues"> & {
  issues: ValidationIssue[];
};

export type TemplateLesson = Omit<
  Solid<Wire<"LessonView">>,
  "state" | "evidence"
> & {
  state: "open" | "accepted" | "dismissed";
  evidence: Record<string, unknown>;
};

export type ChangeOrderStatus =
  | "draft"
  | "submitted"
  | "approved"
  | "rejected"
  | "awaiting_client"
  | "applied"
  | "billing_failed"
  | "billed";

export type ChangeOrder = Omit<
  Solid<Wire<"ChangeOrderView">>,
  "created_by" | "status" | "source" | "confidence" | "scope_changes" | "evidence"
> & {
  status: ChangeOrderStatus;
  source: "manual" | "scope_signal" | "client_request";
  confidence: "strong" | "fair" | "unsure" | null;
  scope_changes: Record<string, unknown>;
  evidence: unknown[];
};

export type ApplyResult = Omit<Solid<Wire<"ApplyResponse">>, "change_order"> & {
  change_order: ChangeOrder;
};

export type ClientUpdate = Omit<
  Solid<Wire<"ClientUpdateView">>,
  "created_by" | "status" | "facts"
> & {
  status: "draft" | "approved" | "sent" | "withheld";
  facts: unknown[];
};

export type PortfolioProject = Omit<
  Solid<Wire<"PortfolioProject">>,
  "phases"
> & {
  phases: Solid<Wire<"PortfolioPhase">>[];
};

export type PortfolioGroup = Omit<
  Solid<Wire<"PortfolioGroup">>,
  "projects"
> & {
  projects: PortfolioProject[];
};

export type PortfolioView = Omit<
  Solid<Wire<"PortfolioView">>,
  "weeks" | "groups" | "unscheduled"
> & {
  weeks: Solid<Wire<"PortfolioWeek">>[];
  groups: PortfolioGroup[];
  unscheduled: Solid<Wire<"UnscheduledProject">>[];
};

export type MyTask = Solid<Wire<"MyTaskView">>;

export type PodSettings = Solid<Wire<"PodSettingsView">>;

export type PodPrefs = Solid<Wire<"PodPrefsView">>;

export type PromoteResult = Solid<Wire<"PromoteResponse">>;

// --- transport --------------------------------------------------------------

interface ErrorEnvelope {
  error?: { message?: string };
}

/** The generated client's result, without its per-path generics. */
interface ApiResponse {
  data?: unknown;
  error?: unknown;
  response: Response;
}

/**
 * A full page load restores the token from the session cookie. Wait for that
 * only when there is no token yet; a seeded test session proceeds.
 *
 * Kept for `deliveryFetch`: the typed helpers below are reached only after a
 * surface has seen `auth.status === "authed"`, so the restore has already set
 * the client's bearer by then.
 */
async function awaitAuth(): Promise<void> {
  if (loadTokens() === null) await authReady();
}

/** Throws the API's own refusal text, exactly as this module always has. */
function refusalMessage(status: number, body: unknown): string {
  const message = (body as ErrorEnvelope | null | undefined)?.error?.message;
  return typeof message === "string" ? message : `The server refused this with ${status}.`;
}

/**
 * Unwraps one generated-client result. A refusal becomes the same `Error` the
 * raw helper produced: the API's message when the envelope carries one, and
 * the status sentence when it does not.
 */
function unwrap<T>(result: ApiResponse): T {
  if (!result.response.ok) {
    throw new Error(refusalMessage(result.response.status, result.error));
  }
  return result.data as T;
}

/** Reads a wire object as the present shape its surface expects. */
function solid<T extends object>(wire: T): Solid<T> {
  return wire as Solid<T>;
}

/**
 * Generic escape hatch for the one caller named in the header. Everything in
 * this module's own endpoint set uses `api.api` through `unwrap`.
 */
export async function deliveryFetch<T>(path: string, init?: RequestInit): Promise<T> {
  await awaitAuth();
  const token = api.tokens.accessToken;
  const response = await fetch(`${resolveApiBaseUrl()}${path}`, {
    ...init,
    credentials: "include",
    headers: {
      "content-type": "application/json",
      ...(token === null ? {} : { authorization: `Bearer ${token}` }),
      ...init?.headers,
    },
  });
  if (!response.ok) {
    let body: ErrorEnvelope | null = null;
    try {
      body = (await response.json()) as ErrorEnvelope;
    } catch {
      // A non-JSON body is not worth guessing at; the status stands.
    }
    throw new Error(refusalMessage(response.status, body));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

// --- view mappers -----------------------------------------------------------

function toChangeOrder(wire: Wire<"ChangeOrderView">): ChangeOrder {
  return {
    ...solid(wire),
    status: wire.status as ChangeOrderStatus,
    source: wire.source as ChangeOrder["source"],
    confidence: wire.confidence as ChangeOrder["confidence"],
    scope_changes: wire.scope_changes as Record<string, unknown>,
    evidence: wire.evidence as unknown[],
  };
}

function toApplyResult(wire: Solid<Wire<"ApplyResponse">>): ApplyResult {
  return {
    change_order: toChangeOrder(wire.change_order),
    tasks_added: wire.tasks_added,
    tasks_shifted: wire.tasks_shifted,
    baseline_id: wire.baseline_id,
  };
}

function toClientUpdate(wire: Wire<"ClientUpdateView">): ClientUpdate {
  return {
    ...solid(wire),
    status: wire.status as ClientUpdate["status"],
    facts: wire.facts as unknown[],
  };
}

function toTemplateLesson(wire: Wire<"LessonView">): TemplateLesson {
  return {
    ...solid(wire),
    state: wire.state as TemplateLesson["state"],
    evidence: wire.evidence as Record<string, unknown>,
  };
}

function toValidationReport(wire: Solid<Wire<"ValidationReport">>): ValidationReport {
  return {
    publishable: wire.publishable,
    issues: wire.issues.map((issue) => ({
      ...solid(issue),
      severity: issue.severity as ValidationIssue["severity"],
    })),
  };
}

function toTemplatePreview(wire: Wire<"TemplatePreviewView">): TemplatePreview {
  return {
    ...solid(wire),
    phases: wire.phases.map(solid),
    tasks: wire.tasks.map(solid),
    roles: wire.roles.map(solid),
  };
}

function toPortfolioView(wire: Wire<"PortfolioView">): PortfolioView {
  return {
    ...solid(wire),
    weeks: wire.weeks.map(solid),
    groups: wire.groups.map((group) => ({
      ...solid(group),
      projects: group.projects.map((project) => ({
        ...solid(project),
        phases: project.phases.map(solid),
      })),
    })),
    unscheduled: wire.unscheduled.map(solid),
  };
}

// --- templates --------------------------------------------------------------

export async function fetchTemplateGallery(includeArchived = false): Promise<GalleryTemplate[]> {
  const result = await api.api.GET("/api/v1/projects/template-gallery", {
    params: { query: includeArchived ? { include_archived: true } : {} },
  });
  return unwrap<Wire<"GalleryTemplateView">[]>(result).map(solid);
}

export async function archiveTemplate(id: string, archived: boolean): Promise<GalleryTemplate> {
  const result = await api.api.POST("/api/v1/project-templates/{id}/archive", {
    params: { path: { id } },
    body: { archived },
  });
  return solid(unwrap<Wire<"GalleryTemplateView">>(result));
}

export async function fetchTemplatePreview(id: string): Promise<TemplatePreview> {
  const result = await api.api.GET("/api/v1/project-templates/{id}/preview", {
    params: { path: { id } },
  });
  return toTemplatePreview(unwrap<Wire<"TemplatePreviewView">>(result));
}

export async function fetchTemplateValidation(id: string): Promise<ValidationReport> {
  const result = await api.api.GET("/api/v1/project-templates/{id}/validate", {
    params: { path: { id } },
  });
  return toValidationReport(unwrap<Wire<"ValidationReport">>(result));
}

export async function fetchTemplateLessons(id: string): Promise<TemplateLesson[]> {
  const result = await api.api.GET("/api/v1/project-templates/{id}/lessons", {
    params: { path: { id } },
  });
  return unwrap<Wire<"LessonView">[]>(result).map(toTemplateLesson);
}

export async function decideTemplateLesson(
  templateId: string,
  lessonId: string,
  state: "accepted" | "dismissed",
  reason?: string,
): Promise<TemplateLesson> {
  const result = await api.api.POST(
    "/api/v1/project-templates/{id}/lessons/{lesson_id}/decide",
    {
      params: { path: { id: templateId, lesson_id: lessonId } },
      body: { state, reason: reason ?? null },
    },
  );
  return toTemplateLesson(unwrap<Wire<"LessonView">>(result));
}

export async function setTemplateMeta(
  id: string,
  meta: {
    category: string | null;
    typical_days: number | null;
    effort_min_minutes: number | null;
    effort_max_minutes: number | null;
  },
): Promise<GalleryTemplate> {
  const result = await api.api.PUT("/api/v1/project-templates/{id}/meta", {
    params: { path: { id } },
    body: meta,
  });
  return solid(unwrap<Wire<"GalleryTemplateView">>(result));
}

export type CreateFromTemplateRequest = Solid<
  Pick<
    Wire<"CreateFromTemplateRequest">,
    "name" | "client_id" | "start_date" | "due_date" | "assignee_map"
  >
>;

export async function createFromTemplate(
  id: string,
  body: CreateFromTemplateRequest,
): Promise<{ id: string; name: string; phase_count: number; task_count: number }> {
  const result = await api.api.POST("/api/v1/project-templates/{id}/create", {
    params: { path: { id } },
    body,
  });
  return solid(unwrap<Wire<"ProjectInstantiateResponse">>(result));
}

export async function forkTemplate(
  id: string,
  name: string,
): Promise<{ id: string; name: string }> {
  const result = await api.api.POST("/api/v1/project-templates/{id}/fork", {
    params: { path: { id } },
    body: { name },
  });
  return solid(unwrap<Wire<"ProjectTemplate">>(result));
}

// --- change orders ----------------------------------------------------------

export async function fetchChangeOrders(projectId: string): Promise<ChangeOrder[]> {
  const result = await api.api.GET("/api/v1/projects/{id}/change-orders", {
    params: { path: { id: projectId } },
  });
  return unwrap<Wire<"ChangeOrderView">[]>(result).map(toChangeOrder);
}

type NewChangeOrderWire = Solid<
  Pick<
    Wire<"CreateProjectChangeOrderRequest">,
    | "title"
    | "source"
    | "reason"
    | "scope_changes"
    | "client_summary"
    | "evidence"
    | "coverage_note"
    | "confidence"
    | "contract_clause"
    | "amount_minor"
    | "currency"
    | "client_ack_required"
  >
>;

export type NewChangeOrder = Omit<
  NewChangeOrderWire,
  "source" | "confidence" | "scope_changes" | "evidence"
> & {
  source: ChangeOrder["source"];
  confidence: ChangeOrder["confidence"];
  scope_changes: Record<string, unknown>;
  evidence: unknown[];
};

export async function createChangeOrder(
  projectId: string,
  body: NewChangeOrder,
): Promise<ChangeOrder> {
  const result = await api.api.POST("/api/v1/projects/{id}/change-orders", {
    params: { path: { id: projectId } },
    body,
  });
  return toChangeOrder(unwrap<Wire<"ChangeOrderView">>(result));
}

export async function patchChangeOrder(
  projectId: string,
  id: string,
  patch: Partial<NewChangeOrder>,
): Promise<ChangeOrder> {
  const result = await api.api.PATCH(
    "/api/v1/projects/{id}/change-orders/{change_order_id}",
    {
      params: { path: { id: projectId, change_order_id: id } },
      body: patch,
    },
  );
  return toChangeOrder(unwrap<Wire<"ChangeOrderView">>(result));
}

export async function changeOrderAction(
  projectId: string,
  id: string,
  action: "submit" | "approve" | "reject" | "apply" | "bill",
  note?: string,
): Promise<ChangeOrder | ApplyResult> {
  const params = { path: { id: projectId, change_order_id: id } };
  switch (action) {
    case "submit": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/change-orders/{change_order_id}/submit",
        { params },
      );
      return toChangeOrder(unwrap<Wire<"ChangeOrderView">>(result));
    }
    case "approve": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/change-orders/{change_order_id}/approve",
        { params, body: { note: note ?? null } },
      );
      return toChangeOrder(unwrap<Wire<"ChangeOrderView">>(result));
    }
    case "reject": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/change-orders/{change_order_id}/reject",
        { params, body: { note: note ?? null } },
      );
      return toChangeOrder(unwrap<Wire<"ChangeOrderView">>(result));
    }
    case "apply": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/change-orders/{change_order_id}/apply",
        { params },
      );
      return toApplyResult(solid(unwrap<Wire<"ApplyResponse">>(result)));
    }
    case "bill": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/change-orders/{change_order_id}/bill",
        { params },
      );
      return toChangeOrder(unwrap<Wire<"ChangeOrderView">>(result));
    }
  }
}

// --- client updates ---------------------------------------------------------

export async function fetchClientUpdates(projectId: string): Promise<ClientUpdate[]> {
  const result = await api.api.GET("/api/v1/projects/{id}/client-updates", {
    params: { path: { id: projectId } },
  });
  return unwrap<Wire<"ClientUpdateView">[]>(result).map(toClientUpdate);
}

export async function draftClientUpdate(
  projectId: string,
  body: { period_start: string | null; period_end: string | null; closing: string | null },
): Promise<ClientUpdate> {
  const result = await api.api.POST("/api/v1/projects/{id}/client-updates", {
    params: { path: { id: projectId } },
    body,
  });
  return toClientUpdate(unwrap<Wire<"ClientUpdateView">>(result));
}

export async function editClientUpdate(
  projectId: string,
  id: string,
  body: string,
): Promise<ClientUpdate> {
  const result = await api.api.PATCH(
    "/api/v1/projects/{id}/client-updates/{update_id}",
    {
      params: { path: { id: projectId, update_id: id } },
      body: { body },
    },
  );
  return toClientUpdate(unwrap<Wire<"ClientUpdateView">>(result));
}

export async function clientUpdateAction(
  projectId: string,
  id: string,
  action: "approve" | "send" | "withhold",
  reason?: string,
): Promise<ClientUpdate> {
  const params = { path: { id: projectId, update_id: id } };
  switch (action) {
    case "approve": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/client-updates/{update_id}/approve",
        { params },
      );
      return toClientUpdate(unwrap<Wire<"ClientUpdateView">>(result));
    }
    case "send": {
      const result = await api.api.POST(
        "/api/v1/projects/{id}/client-updates/{update_id}/send",
        { params },
      );
      return toClientUpdate(unwrap<Wire<"ClientUpdateView">>(result));
    }
    case "withhold": {
      // A missing reason is sent as absent, so the API refuses with the same
      // validation text it produced for the JSON `{}` this used to send.
      const result = await api.api.POST(
        "/api/v1/projects/{id}/client-updates/{update_id}/withhold",
        {
          params,
          body: { reason } as Wire<"WithholdUpdateRequest">,
        },
      );
      return toClientUpdate(unwrap<Wire<"ClientUpdateView">>(result));
    }
  }
}

// --- portfolio --------------------------------------------------------------

interface PortfolioQuery {
  from?: string;
  to?: string;
  group_by?: string;
  at_risk_only?: boolean;
  zoom?: string;
}

function portfolioQuery(params: PortfolioQuery): PortfolioQuery {
  const query: PortfolioQuery = {};
  if (params.from) query.from = params.from;
  if (params.to) query.to = params.to;
  if (params.group_by) query.group_by = params.group_by;
  if (params.at_risk_only) query.at_risk_only = true;
  if (params.zoom) query.zoom = params.zoom;
  return query;
}

export async function fetchPortfolio(params: {
  from?: string;
  to?: string;
  group_by?: string;
  at_risk_only?: boolean;
  zoom?: string;
}): Promise<PortfolioView> {
  const result = await api.api.GET("/api/v1/projects/portfolio", {
    params: { query: portfolioQuery(params) },
  });
  return toPortfolioView(unwrap<Wire<"PortfolioView">>(result));
}

export async function sharePortfolio(params: {
  from?: string;
  to?: string;
  group_by?: string;
  at_risk_only?: boolean;
  zoom?: string;
}): Promise<{ token: string; path: string }> {
  const result = await api.api.POST("/api/v1/projects/portfolio/share", {
    params: { query: portfolioQuery(params) },
  });
  return solid(unwrap<Wire<"ShareView">>(result));
}

export async function fetchSharedPortfolio(token: string): Promise<PortfolioView> {
  // The generated client encodes the path segment itself, so the token cannot
  // turn `..%2F..` into a path. The read is authenticated staff-side (see the
  // header), so it stays on `api.api` and keeps the bearer.
  const result = await api.api.GET("/api/v1/projects/portfolio/shared/{token}", {
    params: { path: { token } },
  });
  return toPortfolioView(unwrap<Wire<"PortfolioView">>(result));
}

// --- pods, work and promotion ----------------------------------------------

export async function fetchMyWork(): Promise<MyTask[]> {
  const result = await api.api.GET("/api/v1/projects/my-work");
  return unwrap<Wire<"MyTaskView">[]>(result).map(solid);
}

export async function setTaskBlocker(
  projectId: string,
  taskId: string,
  body: { kind: string; note: string | null; owner_id: string | null },
): Promise<{ blocked: boolean }> {
  const result = await api.api.PUT("/api/v1/projects/{id}/tasks/{task_id}/blocker", {
    params: { path: { id: projectId, task_id: taskId } },
    body,
  });
  return unwrap<{ blocked: boolean }>(result);
}

export async function clearTaskBlocker(
  projectId: string,
  taskId: string,
): Promise<{ cleared: boolean }> {
  const result = await api.api.DELETE("/api/v1/projects/{id}/tasks/{task_id}/blocker", {
    params: { path: { id: projectId, task_id: taskId } },
  });
  return unwrap<{ cleared: boolean }>(result);
}

export async function promoteTask(
  projectId: string,
  taskId: string,
  body: { board_id: string | null; priority: string | null },
): Promise<PromoteResult> {
  const result = await api.api.POST("/api/v1/projects/{id}/tasks/{task_id}/promote", {
    params: { path: { id: projectId, task_id: taskId } },
    body,
  });
  return solid(unwrap<Wire<"PromoteResponse">>(result));
}

export async function fetchPodSettings(surface = "project_task"): Promise<PodSettings[]> {
  const result = await api.api.GET("/api/v1/project-pods/settings", {
    params: { query: { surface } },
  });
  return unwrap<Wire<"PodSettingsView">[]>(result).map(solid);
}

export async function putPodSettings(body: {
  surface: string;
  role_key: string;
  pods: string[];
}): Promise<PodSettings> {
  const result = await api.api.PUT("/api/v1/project-pods/settings", { body });
  return solid(unwrap<Wire<"PodSettingsView">>(result));
}

export async function fetchPodPrefs(surface = "project_task"): Promise<PodPrefs> {
  const result = await api.api.GET("/api/v1/project-pods/prefs", {
    params: { query: { surface } },
  });
  return solid(unwrap<Wire<"PodPrefsView">>(result));
}

export async function putPodPrefs(body: PodPrefs): Promise<PodPrefs> {
  const result = await api.api.PUT("/api/v1/project-pods/prefs", { body });
  return solid(unwrap<Wire<"PodPrefsView">>(result));
}
