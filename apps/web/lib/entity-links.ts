// Cross-entity link vocabulary + target search (Wave 17 lane 2).
//
// Purpose: ONE definition of the linkable target kinds the RecordLink
// seam accepts from staff surfaces, plus the per-kind search helpers the
// entity picker drives (replacing paste-a-UUID everywhere). The API
// surface for every kind ships today — documents (title q), assets
// (tag/name/fields q), KB articles (title q), tickets (client_id +
// client-side number/title filter), clients, and legacy passwords.
// Non-purpose: no link mutation here — callers POST the seam endpoints;
// this module only finds candidate targets.

import type { components } from "@meridian/sdk";
import { api } from "./api";

export interface LinkKindOption {
  value: components["schemas"]["LinkKind"];
  label: string;
}

/** The kinds the shared link panels offer, in pick order. */
export const LINK_KIND_OPTIONS: LinkKindOption[] = [
  { value: "document", label: "Document" },
  { value: "ticket", label: "Ticket" },
  { value: "asset", label: "Asset / config item" },
  { value: "kb_article", label: "KB article" },
  { value: "password", label: "Legacy password" },
  { value: "client", label: "Client" },
];

export interface LinkTarget {
  id: string;
  kind: components["schemas"]["LinkKind"];
  title: string;
  meta: string;
}

function includes(haystack: string | null | undefined, needle: string): boolean {
  return (haystack ?? "").toLowerCase().includes(needle);
}

/**
 * Search one kind's targets. `clientId` narrows client-scoped kinds
 * (documents, assets, tickets) when the picker is opened from a client
 * context; scope-visible results only — the backend already enforces
 * visibility, and empty results are a genuine empty state.
 */
export async function searchLinkTargets(
  kind: components["schemas"]["LinkKind"],
  query: string,
  clientId?: string,
): Promise<LinkTarget[]> {
  const q = query.trim().toLowerCase();
  if (kind === "document") {
    const { data } = await api.api.GET("/api/v1/documents", {
      params: { query: { ...(clientId ? { client_id: clientId } : {}), ...(q === "" ? {} : { q: query.trim() }) } },
    });
    return (data ?? [])
      .slice(0, 50)
      .map((d) => ({ id: d.id, kind, title: d.title, meta: d.client_id ? "client-scoped" : "org-global" }));
  }
  if (kind === "asset") {
    const { data } = await api.api.GET("/api/v1/assets", {
      params: { query: { ...(clientId ? { client_id: clientId } : {}), ...(q === "" ? {} : { q: query.trim() }) } },
    });
    return (data ?? [])
      .slice(0, 50)
      .map((a) => ({ id: a.id, kind, title: `${a.name}`, meta: `${a.tag} · ${a.status}` }));
  }
  if (kind === "kb_article") {
    const { data } = await api.api.GET("/api/v1/kb/articles", {
      params: { query: q === "" ? {} : { q: query.trim() } },
    });
    return (data ?? [])
      .slice(0, 50)
      .map((a) => ({ id: a.id, kind, title: a.title, meta: a.status }));
  }
  if (kind === "ticket") {
    const { data } = await api.api.GET("/api/v1/tickets", {
      params: { query: { ...(clientId ? { client_id: clientId } : {}), limit: 100 } },
    });
    return (data ?? [])
      .filter((t) => q === "" || includes(t.title, q) || t.number.toLowerCase().includes(q))
      .slice(0, 50)
      .map((t) => ({ id: t.id, kind, title: `${t.number} ${t.title}`, meta: t.category }));
  }
  if (kind === "client") {
    const { data } = await api.api.GET("/api/v1/clients");
    return (data ?? [])
      .filter((c) => q === "" || includes(c.name, q))
      .slice(0, 50)
      .map((c) => ({ id: c.id, kind, title: c.name, meta: "client" }));
  }
  // Legacy password kind.
  const { data } = await api.api.GET("/api/v1/passwords");
  return (data ?? [])
    .filter((p) => q === "" || includes(p.title, q) || includes(p.username, q))
    .slice(0, 50)
    .map((p) => {
      const user = p.username ?? "";
      return { id: p.id, kind, title: p.title, meta: user === "" ? "vault entry" : user };
    });
}
