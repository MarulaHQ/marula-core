// Typed transport for the project-delivery endpoints these surfaces call.
//
// The paths are in the generated schema (packages/sdk); this module still owns
// their transport because every surface here needs one thing the typed client
// does not do — a refusal shown in the API's own words — and the fetch helper
// below does that in one place. The interfaces are this module's reading of
// the responses, narrower than the schema's, and go away with the helper when
// these surfaces move onto `api.api`.

import { api, authReady, loadTokens, resolveApiBaseUrl } from "../../lib/api";

// --- shared shapes ----------------------------------------------------------

export interface GalleryTemplate {
  id: string;
  name: string;
  description: string | null;
  category: string | null;
  typical_days: number | null;
  effort_min_minutes: number | null;
  effort_max_minutes: number | null;
  archived: boolean;
  is_shipped: boolean;
  current_version: number | null;
  forked_from_version_id: string | null;
  phase_count: number;
  task_count: number;
  created_at: string;
  updated_at: string;
}

export interface PreviewTask {
  key: string;
  title: string;
  phase_key: string | null;
  phase_name: string | null;
  role_key: string | null;
  work_type: string | null;
  duration_days: number | null;
  effort_minutes: number | null;
  depends_on: string[];
  optional: boolean;
  is_milestone: boolean;
}

export interface TemplatePreview {
  name: string;
  billing_method: string | null;
  currency: string | null;
  phases: {
    key: string;
    name: string;
    is_milestone: boolean;
    task_count: number;
    effort_minutes: number;
  }[];
  tasks: PreviewTask[];
  roles: { role_key: string; task_count: number; effort_minutes: number }[];
  total_effort_minutes: number;
  dated_tasks: number;
  undated_tasks: number;
}

export interface ValidationIssue {
  code: string;
  severity: "error" | "warning" | "info";
  message: string;
  task_key: string | null;
  phase_key: string | null;
}

export interface ValidationReport {
  publishable: boolean;
  issues: ValidationIssue[];
}

export interface TemplateLesson {
  id: string;
  template_id: string;
  project_id: string | null;
  kind: string;
  message: string;
  evidence: Record<string, unknown>;
  state: "open" | "accepted" | "dismissed";
  dismiss_reason: string | null;
  decided_by: string | null;
  decided_at: string | null;
  created_at: string;
}

export type ChangeOrderStatus =
  | "draft"
  | "submitted"
  | "approved"
  | "rejected"
  | "awaiting_client"
  | "applied"
  | "billing_failed"
  | "billed";

export interface ChangeOrder {
  id: string;
  project_id: string;
  reference: string;
  title: string;
  status: ChangeOrderStatus;
  source: "manual" | "scope_signal" | "client_request";
  reason: string;
  scope_changes: Record<string, unknown>;
  client_summary: string | null;
  evidence: unknown[];
  coverage_note: string | null;
  confidence: "strong" | "fair" | "unsure" | null;
  contract_clause: string | null;
  amount_minor: number | null;
  currency: string | null;
  submitted_at: string | null;
  submitted_by: string | null;
  decided_at: string | null;
  decided_by: string | null;
  decision_note: string | null;
  applied_at: string | null;
  applied_by: string | null;
  applied_baseline_id: string | null;
  billed_at: string | null;
  billing_release_id: string | null;
  billing_error: string | null;
  client_ack_required: boolean;
  client_ack_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface ApplyResult {
  change_order: ChangeOrder;
  tasks_added: number;
  tasks_shifted: number;
  baseline_id: string | null;
}

export interface ClientUpdate {
  id: string;
  project_id: string;
  status: "draft" | "approved" | "sent" | "withheld";
  body: string;
  facts: unknown[];
  period_start: string | null;
  period_end: string | null;
  withheld_reason: string | null;
  approved_by: string | null;
  approved_at: string | null;
  sent_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface PortfolioProject {
  id: string;
  name: string;
  status: string;
  client_id: string | null;
  client_name: string | null;
  start_date: string | null;
  due_date: string | null;
  at_risk: boolean;
  risk_reasons: string[];
  has_dated_phase: boolean;
  phases: {
    id: string;
    name: string;
    status: string;
    start_date: string;
    end_date: string;
  }[];
}

export interface PortfolioGroup {
  key: string;
  label: string;
  project_count: number;
  at_risk_count: number;
  projects: PortfolioProject[];
}

export interface PortfolioView {
  from: string;
  to: string;
  zoom: string;
  group_by: string;
  weeks: { start: string; label: string }[];
  groups: PortfolioGroup[];
  unscheduled: {
    id: string;
    name: string;
    status: string;
    start_date: string | null;
    due_date: string | null;
  }[];
}

export interface MyTask {
  task_id: string;
  project_id: string;
  project_name: string;
  phase_name: string | null;
  title: string;
  status: string;
  blocker_type: string | null;
  blocker_note: string | null;
  blocked_since: string | null;
  due_at: string | null;
  start_at: string | null;
  duration_minutes: number | null;
  effort_remaining_minutes: number | null;
  work_type: string | null;
  is_milestone: boolean;
  ticket_id: string | null;
}

export interface PodSettings {
  surface: string;
  role_key: string;
  pods: string[];
  updated_at: string;
}

export interface PodPrefs {
  surface: string;
  hidden: string[];
  pod_order: string[];
}

export interface PromoteResult {
  task_id: string;
  project_id: string;
  ticket_id: string;
  ticket_number: string;
  created: boolean;
}

// --- transport --------------------------------------------------------------

interface ErrorEnvelope {
  error?: { message?: string };
}

export async function deliveryFetch<T>(path: string, init?: RequestInit): Promise<T> {
  // A full page load restores the token from the session cookie. Wait for
  // that only when there is no token yet; a seeded test session proceeds.
  if (loadTokens() === null) await authReady();
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
    let message = `The server refused this with ${response.status}.`;
    try {
      const body = (await response.json()) as ErrorEnvelope;
      if (body.error?.message !== undefined) message = body.error.message;
    } catch {
      // A non-JSON body is not worth guessing at; the status stands.
    }
    throw new Error(message);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

// --- templates --------------------------------------------------------------

export function fetchTemplateGallery(includeArchived = false): Promise<GalleryTemplate[]> {
  const query = includeArchived ? "?include_archived=true" : "";
  return deliveryFetch<GalleryTemplate[]>(`/api/v1/projects/template-gallery${query}`);
}

export function archiveTemplate(id: string, archived: boolean): Promise<GalleryTemplate> {
  return deliveryFetch<GalleryTemplate>(`/api/v1/project-templates/${id}/archive`, {
    method: "POST",
    body: JSON.stringify({ archived }),
  });
}

export function fetchTemplatePreview(id: string): Promise<TemplatePreview> {
  return deliveryFetch<TemplatePreview>(`/api/v1/project-templates/${id}/preview`);
}

export function fetchTemplateValidation(id: string): Promise<ValidationReport> {
  return deliveryFetch<ValidationReport>(`/api/v1/project-templates/${id}/validate`);
}

export function fetchTemplateLessons(id: string): Promise<TemplateLesson[]> {
  return deliveryFetch<TemplateLesson[]>(`/api/v1/project-templates/${id}/lessons`);
}

export function decideTemplateLesson(
  templateId: string,
  lessonId: string,
  state: "accepted" | "dismissed",
  reason?: string,
): Promise<TemplateLesson> {
  return deliveryFetch<TemplateLesson>(
    `/api/v1/project-templates/${templateId}/lessons/${lessonId}/decide`,
    { method: "POST", body: JSON.stringify({ state, reason: reason ?? null }) },
  );
}

export function setTemplateMeta(
  id: string,
  meta: {
    category: string | null;
    typical_days: number | null;
    effort_min_minutes: number | null;
    effort_max_minutes: number | null;
  },
): Promise<GalleryTemplate> {
  return deliveryFetch<GalleryTemplate>(`/api/v1/project-templates/${id}/meta`, {
    method: "PUT",
    body: JSON.stringify(meta),
  });
}

export interface CreateFromTemplateRequest {
  name: string;
  client_id: string | null;
  start_date: string | null;
  due_date: string | null;
  assignee_map: Record<string, string>;
}

export function createFromTemplate(
  id: string,
  body: CreateFromTemplateRequest,
): Promise<{ id: string; name: string; phase_count: number; task_count: number }> {
  return deliveryFetch(`/api/v1/project-templates/${id}/create`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function forkTemplate(id: string, name: string): Promise<{ id: string; name: string }> {
  return deliveryFetch(`/api/v1/project-templates/${id}/fork`, {
    method: "POST",
    body: JSON.stringify({ name }),
  });
}

// --- change orders ----------------------------------------------------------

export function fetchChangeOrders(projectId: string): Promise<ChangeOrder[]> {
  return deliveryFetch<ChangeOrder[]>(`/api/v1/projects/${projectId}/change-orders`);
}

export interface NewChangeOrder {
  title: string;
  source: ChangeOrder["source"];
  reason: string;
  scope_changes: Record<string, unknown>;
  client_summary: string | null;
  evidence: unknown[];
  coverage_note: string | null;
  confidence: ChangeOrder["confidence"];
  contract_clause: string | null;
  amount_minor: number | null;
  currency: string | null;
  client_ack_required: boolean;
}

export function createChangeOrder(
  projectId: string,
  body: NewChangeOrder,
): Promise<ChangeOrder> {
  return deliveryFetch<ChangeOrder>(`/api/v1/projects/${projectId}/change-orders`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function patchChangeOrder(
  projectId: string,
  id: string,
  patch: Partial<NewChangeOrder>,
): Promise<ChangeOrder> {
  return deliveryFetch<ChangeOrder>(
    `/api/v1/projects/${projectId}/change-orders/${id}`,
    { method: "PATCH", body: JSON.stringify(patch) },
  );
}

export function changeOrderAction(
  projectId: string,
  id: string,
  action: "submit" | "approve" | "reject" | "apply" | "bill",
  note?: string,
): Promise<ChangeOrder | ApplyResult> {
  return deliveryFetch(`/api/v1/projects/${projectId}/change-orders/${id}/${action}`, {
    method: "POST",
    body: action === "approve" || action === "reject" ? JSON.stringify({ note }) : "{}",
  });
}

// --- client updates ---------------------------------------------------------

export function fetchClientUpdates(projectId: string): Promise<ClientUpdate[]> {
  return deliveryFetch<ClientUpdate[]>(`/api/v1/projects/${projectId}/client-updates`);
}

export function draftClientUpdate(
  projectId: string,
  body: { period_start: string | null; period_end: string | null; closing: string | null },
): Promise<ClientUpdate> {
  return deliveryFetch<ClientUpdate>(`/api/v1/projects/${projectId}/client-updates`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function editClientUpdate(
  projectId: string,
  id: string,
  body: string,
): Promise<ClientUpdate> {
  return deliveryFetch<ClientUpdate>(
    `/api/v1/projects/${projectId}/client-updates/${id}`,
    { method: "PATCH", body: JSON.stringify({ body }) },
  );
}

export function clientUpdateAction(
  projectId: string,
  id: string,
  action: "approve" | "send" | "withhold",
  reason?: string,
): Promise<ClientUpdate> {
  return deliveryFetch<ClientUpdate>(
    `/api/v1/projects/${projectId}/client-updates/${id}/${action}`,
    {
      method: "POST",
      body: action === "withhold" ? JSON.stringify({ reason }) : "{}",
    },
  );
}

// --- portfolio --------------------------------------------------------------

export function fetchPortfolio(params: {
  from?: string;
  to?: string;
  group_by?: string;
  at_risk_only?: boolean;
  zoom?: string;
}): Promise<PortfolioView> {
  const query = new URLSearchParams();
  if (params.from) query.set("from", params.from);
  if (params.to) query.set("to", params.to);
  if (params.group_by) query.set("group_by", params.group_by);
  if (params.at_risk_only) query.set("at_risk_only", "true");
  if (params.zoom) query.set("zoom", params.zoom);
  return deliveryFetch<PortfolioView>(`/api/v1/projects/portfolio?${query.toString()}`);
}

export function sharePortfolio(params: {
  from?: string;
  to?: string;
  group_by?: string;
  at_risk_only?: boolean;
  zoom?: string;
}): Promise<{ token: string; path: string }> {
  const query = new URLSearchParams();
  if (params.from) query.set("from", params.from);
  if (params.to) query.set("to", params.to);
  if (params.group_by) query.set("group_by", params.group_by);
  if (params.at_risk_only) query.set("at_risk_only", "true");
  if (params.zoom) query.set("zoom", params.zoom);
  return deliveryFetch(`/api/v1/projects/portfolio/share?${query.toString()}`, {
    method: "POST",
    body: "{}",
  });
}

export function fetchSharedPortfolio(token: string): Promise<PortfolioView> {
  // The token arrives from a link, so it is attacker-controllable: encode it
  // before it becomes a path segment, or `..%2F..` would decode into one.
  return deliveryFetch<PortfolioView>(
    `/api/v1/projects/portfolio/shared/${encodeURIComponent(token)}`,
  );
}

// --- pods, work and promotion ----------------------------------------------

export function fetchMyWork(): Promise<MyTask[]> {
  return deliveryFetch<MyTask[]>("/api/v1/projects/my-work");
}

export function setTaskBlocker(
  projectId: string,
  taskId: string,
  body: { kind: string; note: string | null; owner_id: string | null },
): Promise<{ blocked: boolean }> {
  return deliveryFetch(`/api/v1/projects/${projectId}/tasks/${taskId}/blocker`, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export function clearTaskBlocker(
  projectId: string,
  taskId: string,
): Promise<{ cleared: boolean }> {
  return deliveryFetch(`/api/v1/projects/${projectId}/tasks/${taskId}/blocker`, {
    method: "DELETE",
  });
}

export function promoteTask(
  projectId: string,
  taskId: string,
  body: { board_id: string | null; priority: string | null },
): Promise<PromoteResult> {
  return deliveryFetch<PromoteResult>(
    `/api/v1/projects/${projectId}/tasks/${taskId}/promote`,
    { method: "POST", body: JSON.stringify(body) },
  );
}

export function fetchPodSettings(surface = "project_task"): Promise<PodSettings[]> {
  return deliveryFetch<PodSettings[]>(`/api/v1/project-pods/settings?surface=${surface}`);
}

export function putPodSettings(body: {
  surface: string;
  role_key: string;
  pods: string[];
}): Promise<PodSettings> {
  return deliveryFetch<PodSettings>("/api/v1/project-pods/settings", {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export function fetchPodPrefs(surface = "project_task"): Promise<PodPrefs> {
  return deliveryFetch<PodPrefs>(`/api/v1/project-pods/prefs?surface=${surface}`);
}

export function putPodPrefs(body: PodPrefs): Promise<PodPrefs> {
  return deliveryFetch<PodPrefs>("/api/v1/project-pods/prefs", {
    method: "PUT",
    body: JSON.stringify(body),
  });
}
