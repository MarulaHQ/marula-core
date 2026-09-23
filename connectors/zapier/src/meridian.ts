//! Meridian side of the Zapier integration: a typed `MeridianClient` from
//! @meridian/sdk, authenticated with an x-api-key machine credential.
//!
//! Purpose: prove the extension pattern — this file talks to Meridian ONLY
//! through the published SDK surface (public REST + generated types). No
//! Meridian internals are imported anywhere in this connector.

import { MeridianClient } from "@meridian/sdk";
import type { Config } from "./config.ts";
import type { MeridianEnvelope } from "./events.ts";

export function connectMeridian(
  config: Config,
  apiKey: string = config.meridianApiKey ?? "",
): MeridianClient {
  const client = new MeridianClient({ baseUrl: config.meridianBaseUrl });
  // Machine auth: the scoped API key rides x-api-key on every typed call.
  client.api.use({
    async onRequest({ request }) {
      request.headers.set("x-api-key", apiKey);
      return request;
    },
  });
  return client;
}

/** One live typed read proving key validity + connectivity. */
export async function verifyAccess(client: MeridianClient): Promise<string> {
  const { data, error } = await client.api.GET("/api/v1/tickets", {
    params: { query: { limit: 1 } },
  });
  if (error) {
    throw new Error(`Meridian rejected the key: ${JSON.stringify(error)}`);
  }
  return `key valid — ${data?.length ?? 0} ticket(s) visible in scope`;
}

/**
 * The action side of the Zapier app. Each action is a typed SDK call with
 * an idempotency key (IF1): Zapier retries its perform() calls, and the
 * same key with the same payload replays the SAME stored result —
 * never a duplicate ticket, client, or device record.
 */

export interface CreateTicketInput {
  board_id: string;
  title: string;
  body?: string;
  /** low | medium | high | urgent (Meridian Priority). */
  priority?: string;
  client_id?: string;
}

export async function createTicket(
  client: MeridianClient,
  input: CreateTicketInput,
  idempotencyKey: string,
): Promise<Record<string, unknown>> {
  const { data, error } = await client.api.POST("/api/v1/tickets", {
    body: {
      // The Zapier field map owns real values; extra keys are the caller's
      // problem — this helper passes through exactly what Meridian accepts.
      board_id: input.board_id,
      title: input.title,
      body: input.body ?? null,
      priority: (input.priority ?? "medium") as never,
      client_id: (input.client_id ?? null) as never,
      custom_fields: [],
      idempotency_key: idempotencyKey,
    },
  });
  if (error) throw new Error(`ticket create rejected: ${JSON.stringify(error)}`);
  return (data ?? {}) as Record<string, unknown>;
}

export interface UpdateTicketInput {
  ticket_id: string;
  title?: string;
  body?: string;
  priority?: string;
}

export async function updateTicket(
  client: MeridianClient,
  input: UpdateTicketInput,
  idempotencyKey: string,
): Promise<Record<string, unknown>> {
  const { data, error } = await client.api.PATCH("/api/v1/tickets/{id}", {
    params: { path: { id: input.ticket_id } },
    body: {
      title: input.title ?? null,
      body: input.body ?? null,
      priority: (input.priority ?? null) as never,
      idempotency_key: idempotencyKey,
    },
  });
  if (error) throw new Error(`ticket update rejected: ${JSON.stringify(error)}`);
  return (data ?? {}) as Record<string, unknown>;
}

export interface CreateClientInput {
  name: string;
  notes?: string;
}

export async function createClient(
  client: MeridianClient,
  input: CreateClientInput,
  idempotencyKey: string,
): Promise<Record<string, unknown>> {
  const { data, error } = await client.api.POST("/api/v1/clients", {
    body: {
      name: input.name,
      notes: input.notes ?? null,
      idempotency_key: idempotencyKey,
    },
  });
  if (error) throw new Error(`client create rejected: ${JSON.stringify(error)}`);
  return (data ?? {}) as Record<string, unknown>;
}

export interface UpsertConfigItemInput {
  source_system: string;
  external_id: string;
  client_id: string;
  tag: string;
  name: string;
  /** Deterministic identity signals (serial, MACs, GUID) as JSON string. */
  identity_keys?: string;
  /** Observed-tier field values as JSON string. */
  observed_fields?: string;
}

/** Parse an optional JSON-string field into an object (Zapier fields are
 * strings; malformed JSON is a typed rejection, not a silent drop). */
export function parseJsonField(raw: string | undefined, label: string): object {
  if (raw === undefined || raw.trim() === "") return {};
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (parsed !== null && typeof parsed === "object") return parsed;
    throw new Error("not an object");
  } catch {
    throw new Error(`${label} must be a JSON object (got: ${raw.slice(0, 80)})`);
  }
}

/**
 * CI upsert rides the SAME public feeder seam the first-party connectors
 * use (POST /api/config-items/feed-upsert) — never a blind insert, and it
 * carries its idempotency key through to IF1.
 */
export async function upsertConfigItem(
  client: MeridianClient,
  input: UpsertConfigItemInput,
  idempotencyKey: string,
): Promise<Record<string, unknown>> {
  const { data, error } = await client.api.POST("/api/v1/config-items/feed-upsert", {
    body: {
      source_system: input.source_system,
      external_id: input.external_id,
      client_id: input.client_id,
      tag: input.tag,
      name: input.name,
      identity_keys: parseJsonField(input.identity_keys, "identity_keys"),
      observed_fields: parseJsonField(input.observed_fields, "observed_fields"),
      idempotency_key: idempotencyKey,
    },
  });
  if (error) throw new Error(`config item upsert rejected: ${JSON.stringify(error)}`);
  return (data ?? {}) as Record<string, unknown>;
}

/** RESTHook plumbing: subscribe/unsubscribe a Zapier hook URL with an
 * exact event-type filter (the typed trigger filter). */
export async function subscribeWebhook(
  client: MeridianClient,
  url: string,
  eventTypes: readonly string[],
): Promise<string> {
  const { data, error } = await client.api.POST("/api/v1/webhooks", {
    body: { url, event_types: [...eventTypes] },
  });
  if (error) throw new Error(`webhook subscribe rejected: ${JSON.stringify(error)}`);
  const id = data?.webhook.id;
  if (id === undefined) throw new Error("webhook subscribe returned no id");
  return id;
}

export async function unsubscribeWebhook(client: MeridianClient, id: string): Promise<void> {
  const { error } = await client.api.DELETE("/api/v1/webhooks/{id}", {
    params: { path: { id } },
  });
  if (error) throw new Error(`webhook unsubscribe rejected: ${JSON.stringify(error)}`);
}

/** Trigger poll/sample output: wrap a live webhook.test envelope. */
export function triggerRecord(envelope: MeridianEnvelope): Record<string, unknown> {
  return {
    ...envelope.payload,
    _event_id: envelope.id,
    _event_type: envelope.type,
    _entity_id: envelope.entity_id,
    _occurred_at: envelope.occurred_at,
  };
}
