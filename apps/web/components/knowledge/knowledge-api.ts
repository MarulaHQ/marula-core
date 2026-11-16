"use client";
/**
 * The typed fetch layer for the knowledge surfaces.
 *
 * Purpose: one place for the new `/knowledge/*` calls, which are not in the
 * generated SDK yet (they land with the lane's OpenAPI registration). It uses
 * the same base URL and bearer token as `api.tsx`, so auth, proxying and the
 * desktop shell's injected API base all behave identically.
 *
 * Non-purpose: no caching, no retries, no toast. Callers own their loading and
 * error states, because a hub and a row action want to say different things.
 */

import { apiBaseUrl, authHeaders, authReady, loadTokens } from "../../lib/api";

export class KnowledgeRequestError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "KnowledgeRequestError";
  }
}

async function messageFor(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { message?: string; reason?: string; error?: string };
    if (typeof body.message === "string" && body.message.trim() !== "") return body.message;
    if (typeof body.error === "string" && body.error.trim() !== "") return body.error;
    if (typeof body.reason === "string" && body.reason.trim() !== "") {
      return body.reason.replaceAll("_", " ");
    }
  } catch {
    // A non-JSON body carries nothing a person can act on.
  }
  return response.status === 403
    ? "You do not have permission for this — ask an owner or admin."
    : "That request did not land. Try again.";
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  // A full reload restores the access token from the session cookie; a raw
  // fetch that fires first would carry no bearer. Wait for that restore.
  // A full page load restores the token from the session cookie. Wait for
  // that only when there is no token yet; a seeded test session proceeds.
  if (loadTokens() === null) await authReady();
  const response = await fetch(`${apiBaseUrl()}${path}`, {
    ...init,
    // The refresh secret lives in an httpOnly cookie, so a raw fetch must ask
    // for it explicitly; a full reload has no in-memory access token yet.
    credentials: "include",
    headers: {
      ...authHeaders(),
      ...(init?.body === undefined ? {} : { "content-type": "application/json" }),
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw new KnowledgeRequestError(response.status, await messageFor(response));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export const knowledgeApi = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "POST", body: body === undefined ? undefined : JSON.stringify(body) }),
  patch: <T>(path: string, body: unknown) =>
    request<T>(path, { method: "PATCH", body: JSON.stringify(body) }),
  put: <T>(path: string, body: unknown) =>
    request<T>(path, { method: "PUT", body: JSON.stringify(body) }),
  del: <T>(path: string) => request<T>(path, { method: "DELETE" }),
};

/* ---------- response shapes (mirror crates/core-db/src/knowledge_home.rs) ---------- */

export interface Completeness {
  complete: number;
  total: number;
  gaps: string[];
}

export interface KnowledgeClient {
  id: string;
  name: string;
  sites_n: number;
  assets_n: number;
  documents_n: number;
  articles_n: number;
  secrets_n: number;
  racks_n: number;
  networks_n: number;
  domains_n: number;
  procedures_n: number;
  stale_secrets_n: number;
  expiring_n: number;
  completeness: Completeness;
  last_activity: string | null;
}

export interface KnowledgeHome {
  clients: KnowledgeClient[];
  org: {
    published_articles_n: number;
    draft_articles_n: number;
    org_documents_n: number;
    templates_n: number;
    org_secrets_n: number;
  };
  expiring_soon_n: number;
}

export interface Hub {
  header: {
    client_id: string;
    name: string;
    owner_name: string | null;
    review_note: string | null;
    completeness: Completeness;
    last_activity: string | null;
    pinned_document_ids: string[];
  };
  sections: {
    articles_n: number;
    documents_n: number;
    assets_n: number;
    secrets_n: number;
    domains_n: number;
    networks_n: number;
    racks_n: number;
    procedures_n: number;
    sites_n: number;
    contacts_n: number;
  };
  expirations: ExpiryRow[];
  recent_changes: Array<{
    kind: string;
    title: string;
    entity_id: string;
    actor_name: string | null;
    changed_at: string;
  }>;
  secrets_glance: {
    total_n: number;
    stale_n: number;
    expiring_n: number;
    revealed_week_n: number;
  };
}

export interface ExpiryRow {
  kind: string;
  title: string;
  client_id: string;
  client_name: string;
  due_on: string;
  owner: string | null;
  source_id: string | null;
  provenance: string;
}

export interface Rack {
  id: string;
  org_id: string;
  client_id: string;
  site_id: string | null;
  name: string;
  code: string | null;
  units: number;
  width_mm: number | null;
  depth_mm: number | null;
  power_capacity_kw: number | null;
  power_used_kw: number | null;
  cooling: string | null;
  location_note: string | null;
  notes: string | null;
  fields: unknown;
  created_at: string;
  updated_at: string;
}

export interface RackDevice {
  id: string;
  rack_id: string;
  asset_id: string | null;
  name: string;
  device_type: "network" | "compute" | "storage" | "power" | "patch" | "other";
  face: "front" | "rear";
  unit_start: number;
  unit_height: number;
  model: string | null;
  serial: string | null;
  status: "online" | "attention" | "offline" | "passive";
  power_notes: string | null;
  network_notes: string | null;
}

export interface RackDetail extends Rack {
  devices: RackDevice[];
  links: KnowledgeLink[];
  used_units: number;
}

export interface Network {
  id: string;
  client_id: string;
  site_id: string | null;
  name: string;
  kind: string;
  gateway: string | null;
  dns: string | null;
  vlan_zone: string | null;
  notes: string | null;
  fields: unknown;
  created_at: string;
  updated_at: string;
}

export interface Subnet {
  id: string;
  network_id: string;
  cidr: string;
  vlan_id: number | null;
  name: string | null;
  gateway: string | null;
  used_for: string | null;
  dhcp: boolean;
  notes: string | null;
}

export interface IpReservation {
  id: string;
  subnet_id: string;
  address: string;
  hostname: string | null;
  asset_id: string | null;
  status: "assigned" | "reserved" | "deprecated";
  notes: string | null;
}

export interface NetworkDetail extends Network {
  subnets: Subnet[];
  reservations: IpReservation[];
  links: KnowledgeLink[];
}

export interface Domain {
  id: string;
  client_id: string;
  domain: string;
  kind: "domain" | "website";
  registrar: string | null;
  registrar_expiry: string | null;
  ssl_expiry: string | null;
  ssl_issuer: string | null;
  dns_provider: string | null;
  hosting: string | null;
  manages_dns: boolean;
  monitoring: boolean;
  status_url: string | null;
  notes: string | null;
  updated_at: string;
}

export interface Procedure {
  id: string;
  client_id: string;
  title: string;
  summary: string | null;
  body: string;
  owner_user_id: string | null;
  owner_name: string | null;
  is_checklist: boolean;
  client_visible: boolean;
  version: number;
  updated_at: string;
}

export interface ProcedureStep {
  id: string;
  procedure_id: string;
  position: number;
  title: string;
  body: string | null;
  required: boolean;
}

export interface ProcedureDetail extends Procedure {
  steps: ProcedureStep[];
  links: KnowledgeLink[];
}

export interface KnowledgeLink {
  id: string;
  client_id: string | null;
  from_kind: string;
  from_id: string;
  to_kind: string;
  to_id: string;
  label: string | null;
  created_at: string;
}

export interface SearchHit {
  kind: string;
  id: string;
  title: string;
  subtitle: string;
  client_id: string | null;
  client_name: string | null;
  href: string;
}

export interface PortalDoc {
  id: string;
  kind: "document" | "article";
  title: string;
  summary: string;
  href: string;
  updated_at: string;
}
