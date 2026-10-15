// The canvas's reads and writes over the workflow surface.
//
// Purpose: one place that talks to core-api for the canvas — the node registry
// (the palette + the properties panel's schemas), the workflow document
// (load/save), and the rule dry-run. It reuses the app's singleton client so
// bearer auth rides every call.
//
// The document endpoints are newer than the committed OpenAPI pin, so the
// generated client does not yet type them; the calls below go through the same
// transport (`api.api`) with a local path cast until the next pin regen, at
// which point this file's cast can be deleted and the generated types used
// directly. Nothing else about the transport changes.
//
// Non-purpose: no business logic and no caching. The canvas owns its view
// state; this owns request/response shapes.

import { api } from "../../lib/api";
import type { WorkflowDocumentDoc } from "./document";

/** Minimal surface of the generated client, for paths not yet in the pin. */
interface UntypedClient {
  GET(path: string, opts?: unknown): Promise<{ data?: unknown; error?: unknown }>;
  POST(path: string, opts?: unknown): Promise<{ data?: unknown; error?: unknown }>;
  PATCH(path: string, opts?: unknown): Promise<{ data?: unknown; error?: unknown }>;
}

const client = api.api as unknown as UntypedClient;

/** One first-party registry node, as `GET /api/workflows/nodes` returns it. */
export interface RegistryNodeView {
  id: string;
  label: string;
  agent_description: string;
  family: "trigger" | "decision" | "condition" | "action" | "control" | "value";
  tier: "A" | "B" | "C";
  idempotent: boolean;
  audience: "human" | "ai" | "both";
  executor: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  emits: string[];
}

export interface NodeCatalogueView {
  version: string;
  digest: string;
  node_count: number;
  nodes: RegistryNodeView[];
}

/** One workflow document row, as the CRUD endpoints return it. */
export interface WorkflowDocumentView {
  id: string;
  name: string;
  enabled: boolean;
  trigger_event: string;
  revision: number;
  /** `"ticket"` or `"board"` — the subject tier the document runs against. */
  subject_kind: string;
  created_by: string | null;
  definition: WorkflowDocumentDoc;
}

export interface WorkflowDocumentListView {
  documents: WorkflowDocumentView[];
}

/** The rule-builder's dry-run response, reused so the canvas never invents a
 * second way to test (spec F). */
export interface AutomationTestRunResponse {
  rule_id: string;
  sampled_count: number;
  matched_count: number;
  matches: { ticket_id: string; number: string; title: string; assignee_id: string | null }[];
  actions_preview: unknown;
}

export class CanvasApiError extends Error {
  readonly status?: number;
  readonly reason?: string;
  constructor(message: string, status?: number, reason?: string) {
    super(message);
    this.name = "CanvasApiError";
    this.status = status;
    this.reason = reason;
  }
}

function reasonOf(error: unknown): string | undefined {
  const body = error as { error?: { reason?: unknown } } | undefined;
  const reason = body?.error?.reason;
  return typeof reason === "string" ? reason : undefined;
}

function messageOf(error: unknown, fallback: string): string {
  const body = error as { error?: { message?: unknown; detail?: unknown } } | undefined;
  const message = body?.error?.message ?? body?.error?.detail;
  return typeof message === "string" && message.length > 0 ? message : fallback;
}

/** The palette + panel schemas: the registry, human audience, every tier. */
export async function fetchCatalogue(): Promise<NodeCatalogueView> {
  const { data, error } = await client.GET("/api/v1/workflows/nodes", {
    params: { query: { audience: "human" } },
  });
  if (error !== undefined || data === undefined) {
    throw new CanvasApiError(messageOf(error, "Could not load the step catalogue."), 0, reasonOf(error));
  }
  return data as NodeCatalogueView;
}

export async function listWorkflowDocuments(): Promise<WorkflowDocumentView[]> {
  const { data, error } = await client.GET("/api/v1/workflows/documents", {});
  if (error !== undefined || data === undefined) {
    throw new CanvasApiError(messageOf(error, "Could not load workflows."), 0, reasonOf(error));
  }
  return (data as WorkflowDocumentListView).documents;
}

export async function getWorkflowDocument(id: string): Promise<WorkflowDocumentView> {
  const { data, error } = await client.GET("/api/v1/workflows/documents/{id}", {
    params: { path: { id } },
  });
  if (error !== undefined || data === undefined) {
    throw new CanvasApiError(messageOf(error, "Could not open the workflow."), 0, reasonOf(error));
  }
  return data as WorkflowDocumentView;
}

export async function createWorkflowDocument(input: {
  name: string;
  trigger_event: string;
  subject_kind?: string;
  definition: WorkflowDocumentDoc;
}): Promise<WorkflowDocumentView> {
  const { data, error } = await client.POST("/api/v1/workflows/documents", { body: input });
  if (error !== undefined || data === undefined) {
    throw new CanvasApiError(messageOf(error, "Could not save the workflow."), 0, reasonOf(error));
  }
  return data as WorkflowDocumentView;
}

export async function updateWorkflowDocument(
  id: string,
  input: {
    name?: string;
    trigger_event?: string;
    subject_kind?: string;
    definition?: WorkflowDocumentDoc;
    enabled?: boolean;
  },
): Promise<WorkflowDocumentView> {
  const { data, error } = await client.PATCH("/api/v1/workflows/documents/{id}", {
    params: { path: { id } },
    body: input,
  });
  if (error !== undefined || data === undefined) {
    throw new CanvasApiError(messageOf(error, "Could not save the workflow."), 0, reasonOf(error));
  }
  return data as WorkflowDocumentView;
}

/** The simple-tier dry-run, the only test path the canvas reuses. */
export async function testRunRule(ruleId: string): Promise<AutomationTestRunResponse> {
  const { data, error } = await client.POST("/api/v1/automation-rules/{id}/test-run", {
    params: { path: { id: ruleId } },
  });
  if (error !== undefined || data === undefined) {
    throw new CanvasApiError(messageOf(error, "The test run failed."), 0, reasonOf(error));
  }
  return data as AutomationTestRunResponse;
}
