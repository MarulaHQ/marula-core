// Connector write requests, one place (§2–§3, ADR 0032).
//
// Purpose: the connector write routes are newer than the pinned OpenAPI
// document, so `@marula-core/sdk` does not carry them. This module is the
// single hand-written client for the five paths the surface needs, and it
// returns a typed outcome instead of throwing: a 409 from a connection delete
// names the destinations and vendor identities that still point at it, and
// that copy has to reach the screen rather than become a generic toast.
//
// Non-purpose: no roster read (that lives in `./types` and the page), no
// caching, no retry. A save is not done until the server answers, so every
// function resolves only after the response has been read.
//
// Error shape: `crates/core-api/src/error.rs` emits
// `{ error: { code, reason, message } }`. `message` is hand-written for the
// connector routes (`label must be 1..=120 characters`, `connection is still
// referenced: …`), so it is safe and useful to render verbatim. A response
// that carries no envelope degrades to a status sentence.

import { apiBaseUrl, authHeaders } from "../api";

export interface BindingView {
  id: string;
  connection_id: string;
  /** `slack_channel` | `teams_conversation` | `rewst_workflow` | `voip_route`. */
  binding_kind: string;
  label: string;
  /** The vendor's own id. Never editable: changing it is a new destination. */
  external_id: string;
  external_parent: string | null;
  vendor_ids: Record<string, unknown>;
  enabled: boolean;
}

export interface CreateConnectionBody {
  label: string;
  /** Absent or null is the org-wide connection. */
  scope_client_id?: string | null;
  settings: Record<string, unknown>;
  /** Credential values only. Never sent in `settings`. */
  secrets?: Record<string, string>;
  /** `roles.id` values. Empty means every caller the module permission allows. */
  audience_roles?: string[];
}

export interface UpdateConnectionBody {
  label?: string;
  /** `null` moves the connection back to org-wide. */
  scope_client_id?: string | null;
  /** Merged per key: absent untouched, `null` clears. */
  settings?: Record<string, unknown>;
  /** Merged per key: absent leaves the stored credential alone. */
  secrets?: Record<string, string>;
  audience_roles?: string[];
}

export interface CreateBindingBody {
  binding_kind: string;
  label: string;
  external_id: string;
  external_parent?: string | null;
  vendor_ids?: Record<string, unknown>;
  enabled?: boolean;
}

export interface UpdateBindingBody {
  label?: string;
  enabled?: boolean;
  external_parent?: string | null;
  vendor_ids?: Record<string, unknown>;
}

/** One typed refusal, ready for the surface that asked. */
export interface ConnectorFailure {
  status: number;
  reason: string;
  /** Copy to render. The server's own words when it wrote any. */
  message: string;
  /**
   * The settings field a validation message names, when it names one. The
   * schema form attaches the message to that field as well as to the banner.
   */
  field?: string;
}

export type ConnectorResult<T> =
  | { ok: true; data: T }
  | { ok: false; failure: ConnectorFailure };

const FALLBACK: Readonly<Record<number, string>> = {
  400: "The request was rejected. Check the values and try again.",
  401: "Your session expired. Sign in again to continue.",
  403: "You don't have permission to change connectors — that needs an owner or admin.",
  404: "That connection no longer exists, or it isn't visible to you.",
  409: "This conflicts with something that still points at it.",
  422: "Some values do not satisfy this connector's schema.",
  429: "Too many requests in a short window — wait a moment, then try again.",
  500: "Something went wrong on our side — nothing was saved. Try again.",
  503: "This service is temporarily unavailable. Try again in a moment.",
};

/** `settings key "network.port" is …` names a field; the form uses it. */
export function settingsFieldFromMessage(message: string): string | undefined {
  const settingsKey = /settings key "([^"]+)"/.exec(message);
  if (settingsKey?.[1] !== undefined) return settingsKey[1].split(".")[0];
  const secretKey = /secret "([^"]+)"/.exec(message);
  if (secretKey?.[1] !== undefined) return secretKey[1];
  if (message.startsWith("label ")) return "label";
  if (message.startsWith("external_id ")) return "external_id";
  if (message.startsWith("audience_roles ")) return "audience_roles";
  return undefined;
}

function failureFrom(status: number, body: unknown): ConnectorFailure {
  let reason = "internal_error";
  let message: string | undefined;
  if (typeof body === "object" && body !== null && "error" in body) {
    const envelope = (body as { error?: unknown }).error;
    if (typeof envelope === "object" && envelope !== null) {
      const shaped = envelope as { reason?: unknown; message?: unknown };
      if (typeof shaped.reason === "string") reason = shaped.reason;
      if (typeof shaped.message === "string" && shaped.message.trim() !== "") {
        message = shaped.message;
      }
    }
  }
  const text = message ?? FALLBACK[status] ?? `The request failed (${String(status)}).`;
  const failure: ConnectorFailure = { status, reason, message: text };
  // A field hint only helps a 422; a 409 naming a destination is not a field.
  if (status === 422) {
    const field = settingsFieldFromMessage(text);
    if (field !== undefined) failure.field = field;
  }
  return failure;
}

function url(path: string): string {
  return `${apiBaseUrl()}/api/v1${path}`;
}

async function request<T>(
  method: "GET" | "POST" | "PATCH" | "DELETE",
  path: string,
  body?: unknown,
): Promise<ConnectorResult<T>> {
  try {
    const response = await fetch(url(path), {
      method,
      headers: {
        ...authHeaders(),
        ...(body === undefined ? {} : { "content-type": "application/json" }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      cache: "no-store",
    });
    if (response.status === 204) {
      return { ok: true, data: undefined as T };
    }
    const text = await response.text();
    const parsed: unknown = text === "" ? undefined : safeJson(text);
    if (!response.ok) {
      return { ok: false, failure: failureFrom(response.status, parsed) };
    }
    return { ok: true, data: parsed as T };
  } catch (error) {
    // A network failure is not a refusal: nothing reached the server, so the
    // same submit can be retried unchanged.
    return {
      ok: false,
      failure: {
        status: 0,
        reason: "network_error",
        message:
          error instanceof Error
            ? `The request did not reach the server (${error.message}). Nothing was saved — try again.`
            : "The request did not reach the server. Nothing was saved — try again.",
      },
    };
  }
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return { error: { message: text } };
  }
}

/** `POST /connector-installations/{id}/connections` — configure one use. */
export function createConnection(
  installationId: string,
  body: CreateConnectionBody,
): Promise<ConnectorResult<unknown>> {
  return request<unknown>(
    "POST",
    `/connector-installations/${encodeURIComponent(installationId)}/connections`,
    body,
  );
}

/** `PATCH /connector-connections/{id}` — merged; absent leaves a field alone. */
export function updateConnection(
  id: string,
  body: UpdateConnectionBody,
): Promise<ConnectorResult<unknown>> {
  return request<unknown>("PATCH", `/connector-connections/${encodeURIComponent(id)}`, body);
}

/**
 * `DELETE /connector-connections/{id}`.
 *
 * A 409 names what still points at the connection (destinations, external
 * references, vendor identities). It is a refusal, not a failure: the caller
 * renders `failure.message` in place and offers no retry, because retrying the
 * same delete cannot change the answer.
 */
export function deleteConnection(id: string): Promise<ConnectorResult<void>> {
  return request<void>("DELETE", `/connector-connections/${encodeURIComponent(id)}`);
}

/** `GET /connector-connections/{id}/bindings` — enabled and disabled rows. */
export function listBindings(connectionId: string): Promise<ConnectorResult<BindingView[]>> {
  return request<{ bindings?: BindingView[] }>(
    "GET",
    `/connector-connections/${encodeURIComponent(connectionId)}/bindings`,
  ).then((result) => {
    if (!result.ok) return result;
    const rows = result.data.bindings;
    if (!Array.isArray(rows)) {
      return {
        ok: false as const,
        failure: {
          status: 200,
          reason: "invalid_response",
          message: "The destination list came back without a bindings array.",
        },
      };
    }
    return { ok: true as const, data: rows };
  });
}

/** `POST /connector-connections/{id}/bindings` — add a destination. */
export function createBinding(
  connectionId: string,
  body: CreateBindingBody,
): Promise<ConnectorResult<BindingView>> {
  return request<BindingView>(
    "POST",
    `/connector-connections/${encodeURIComponent(connectionId)}/bindings`,
    body,
  );
}

/** `PATCH /connector-bindings/{id}` — rename, disable, re-point metadata. */
export function updateBinding(
  id: string,
  body: UpdateBindingBody,
): Promise<ConnectorResult<BindingView>> {
  return request<BindingView>("PATCH", `/connector-bindings/${encodeURIComponent(id)}`, body);
}

/** `DELETE /connector-bindings/{id}` — remove a destination. */
export function deleteBinding(id: string): Promise<ConnectorResult<void>> {
  return request<void>("DELETE", `/connector-bindings/${encodeURIComponent(id)}`);
}

/** `GET /plugins/installed/{id}/settings` — the pinned manifest's schema. */
export function fetchPinnedSettingsSchema(
  installationId: string,
): Promise<ConnectorResult<unknown>> {
  return request<{ settings_schema?: unknown }>(
    "GET",
    `/plugins/installed/${encodeURIComponent(installationId)}/settings`,
  ).then((result) => {
    if (!result.ok) return result;
    return { ok: true as const, data: result.data.settings_schema ?? null };
  });
}
