//! The Zapier platform app definition for Meridian, as plain TypeScript.
//!
//! Purpose: a copy-pasteable reference — `npx zapier init`, drop these
//! files' shapes into your app, and push. It is deliberately shaped per
//! zapier-platform-schema (authentication, triggers with RESTHook
//! subscribe/unsubscribe, creates) WITHOUT importing zapier-platform-core,
//! so this connector typechecks against @meridian/sdk alone; the module
//! stays the single source of truth for both the field definitions and
//! the typed perform() bodies.
//!
//! None-mode doctrine: without credentials the definition still renders
//! in full (print-app); only the live probe requires an API key.

import type { Config } from "./config.ts";
import { MERIDIAN_EVENT_TYPES, TRIGGERS, parseEnvelope, type TriggerDefinition } from "./events.ts";
import {
  connectMeridian,
  createClient as createClientAction,
  createTicket,
  subscribeWebhook,
  unsubscribeWebhook,
  updateTicket,
  upsertConfigItem,
  verifyAccess,
} from "./meridian.ts";
import type { MeridianClient } from "@meridian/sdk";

// ---- Zapier platform shape (subset of zapier-platform-schema) ------------

export interface ZapierField {
  key: string;
  label: string;
  type?: string;
  required?: boolean;
  default?: string;
  helpText?: string;
}

export interface ZapierPerformContext {
  /** The auth session's fields (here: { apiKey, baseUrl }). */
  authData: Record<string, string>;
  /** The user's mapped input data for this action/trigger. */
  inputData: Record<string, string>;
  /** The raw webhook body (triggers). */
  cleanedRequest?: { body?: unknown };
  /** Zapier-provided hook URL (RESTHook subscribe). */
  targetUrl?: string;
  /** The stored subscription id (RESTHook unsubscribe). */
  subscribeData?: { id?: string };
}

export interface ZapierTrigger {
  key: string;
  noun: string;
  /** RESTHook: Zapier calls subscribe on zap creation. */
  subscribe?: (ctx: ZapierPerformContext) => Promise<Record<string, unknown>>;
  unsubscribe?: (ctx: ZapierPerformContext) => Promise<void>;
  perform?: (ctx: ZapierPerformContext) => Promise<unknown[]>;
  performList?: (ctx: ZapierPerformContext) => Promise<unknown[]>;
  inputFields: ZapierField[];
}

export interface ZapierCreate {
  key: string;
  noun: string;
  display: { label: string; description: string };
  operation: { inputFields: ZapierField[]; perform: (ctx: ZapierPerformContext) => Promise<unknown> };
}

export interface ZapierApp {
  platformVersion: string;
  /** Meridian auth: one scoped API key (x-api-key) per installed Zap. */
  authentication: { type: "custom"; fields: ZapierField[]; test: string };
  triggers: Record<string, ZapierTrigger>;
  creates: Record<string, ZapierCreate>;
}

// ---- Wiring --------------------------------------------------------------

function clientFrom(ctx: ZapierPerformContext, config: Config): MeridianClient {
  const apiKey = ctx.authData["apiKey"] ?? config.meridianApiKey ?? "";
  return connectMeridian(config, apiKey);
}

function idempotencyKey(prefix: string): string {
  return `${prefix}-zapier-${crypto.randomUUID()}`;
}

function triggerFrom(def: TriggerDefinition, config: Config): ZapierTrigger {
  return {
    key: def.key,
    noun: def.noun,
    inputFields: [
      {
        key: "event_filter",
        label: "Event types",
        type: "string",
        required: false,
        default: def.eventTypes.join(","),
        helpText: `Comma-separated Meridian event types (default: this trigger's filter: ${def.eventTypes.join(", ")})`,
      },
    ],
    subscribe: async (ctx) => {
      const client = clientFrom(ctx, config);
      const eventTypes = (ctx.inputData["event_filter"] ?? def.eventTypes.join(","))
        .split(",")
        .map((s) => s.trim())
        .filter((s) => MERIDIAN_EVENT_TYPES.includes(s as never));
      const id = await subscribeWebhook(
        client,
        ctx.targetUrl ?? "https://hooks.zapier.com/replace-me",
        eventTypes,
      );
      return { id };
    },
    unsubscribe: async (ctx) => {
      const id = ctx.subscribeData?.id;
      if (!id) return;
      const client = clientFrom(ctx, config);
      await unsubscribeWebhook(client, id);
    },
    perform: async (ctx) => {
      const envelope = parseEnvelope(ctx.cleanedRequest?.body);
      return envelope === null ? [] : [def.pick(envelope)];
    },
  };
}

function field(key: string, label: string, required: boolean, helpText?: string): ZapierField {
  return { key, label, required, helpText, type: required ? undefined : "string" };
}

function ticketActions(config: Config): ZapierCreate[] {
  return [
    {
      key: "create_ticket",
      noun: "Ticket",
      display: {
        label: "Create Ticket",
        description: "Creates a Meridian ticket (idempotent — retries never duplicate).",
      },
      operation: {
        inputFields: [
          field("board_id", "Board ID", true, "UUID of the target board"),
          field("title", "Title", true),
          field("body", "Body", false),
          field("priority", "Priority", false, "low | medium | high | urgent (default medium)"),
          field("client_id", "Client ID", false, "UUID to link the ticket to a client"),
        ],
        perform: async (ctx) => {
          const client = clientFrom(ctx, config);
          return createTicket(
            client,
            {
              board_id: ctx.inputData["board_id"] ?? "",
              title: ctx.inputData["title"] ?? "",
              body: ctx.inputData["body"],
              priority: ctx.inputData["priority"],
              client_id: ctx.inputData["client_id"],
            },
            idempotencyKey("create_ticket"),
          );
        },
      },
    },
    {
      key: "update_ticket",
      noun: "Ticket",
      display: {
        label: "Update Ticket",
        description: "Updates title/body/priority of an existing Meridian ticket.",
      },
      operation: {
        inputFields: [
          field("ticket_id", "Ticket ID", true, "UUID of the ticket to update"),
          field("title", "Title", false),
          field("body", "Body", false),
          field("priority", "Priority", false, "low | medium | high | urgent"),
        ],
        perform: async (ctx) => {
          const client = clientFrom(ctx, config);
          return updateTicket(
            client,
            {
              ticket_id: ctx.inputData["ticket_id"] ?? "",
              title: ctx.inputData["title"],
              body: ctx.inputData["body"],
              priority: ctx.inputData["priority"],
            },
            idempotencyKey("update_ticket"),
          );
        },
      },
    },
  ];
}

/** The complete app definition (also what print-app serializes). */
export function buildZapierApp(config: Config): ZapierApp {
  return {
    platformVersion: "zapier-platform-schema@16",
    authentication: {
      type: "custom",
      fields: [
        field("baseUrl", "Meridian base URL", true, "https://meridian.example.test"),
        field("apiKey", "Meridian API key", true, "Scoped key: tickets:read/write + inventory:write + integrations:write"),
      ],
      test: "GET /api/v1/tickets?limit=1 with x-api-key — 200 proves key + scope",
    },
    triggers: Object.fromEntries(TRIGGERS.map((t) => [t.key, triggerFrom(t, config)])),
    creates: Object.fromEntries(
      (
        [
          ...ticketActions(config),
        {
          key: "create_client",
          noun: "Client",
          display: {
            label: "Create Client",
            description: "Creates a Meridian client org (idempotent).",
          },
          operation: {
            inputFields: [
              field("name", "Name", true),
              field("notes", "Notes", false),
            ],
            perform: async (ctx) => {
              const client = clientFrom(ctx, config);
              return createClientAction(
                client,
                { name: ctx.inputData["name"] ?? "", notes: ctx.inputData["notes"] },
                idempotencyKey("create_client"),
              );
            },
          },
        },
        {
          key: "upsert_config_item",
          noun: "Config Item",
          display: {
            label: "Upsert Config Item",
            description:
              "Writes a device/config item through the public feed-upsert seam (dedupe-aware, idempotent).",
          },
          operation: {
            inputFields: [
              field("source_system", "Source system", true, "e.g. zapier"),
              field("external_id", "External ID", true, "Stable id in the source system"),
              field("client_id", "Client ID", true, "UUID of the owning client"),
              field("tag", "Tag", true),
              field("name", "Name", true),
              field("identity_keys", "Identity keys", false, "JSON object: serial, macs, source GUID"),
              field("observed_fields", "Observed fields", false, "JSON object of observed-tier values"),
            ],
            perform: async (ctx) => {
              const client = clientFrom(ctx, config);
              return upsertConfigItem(
                client,
                {
                  source_system: ctx.inputData["source_system"] ?? "",
                  external_id: ctx.inputData["external_id"] ?? "",
                  client_id: ctx.inputData["client_id"] ?? "",
                  tag: ctx.inputData["tag"] ?? "",
                  name: ctx.inputData["name"] ?? "",
                  identity_keys: ctx.inputData["identity_keys"],
                  observed_fields: ctx.inputData["observed_fields"],
                },
                idempotencyKey("upsert_config_item"),
              );
            },
          },
        },
        ] as ZapierCreate[]
      ).map((c) => [c.key, c]),
    ),
  };
}

export { verifyAccess };
