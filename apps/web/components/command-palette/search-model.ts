// Palette result model.
//
// Purpose: turn GET /api/v1/search into the palette's grouped rows in the same
// group order and with the same labels the /search page uses, plus the small
// Actions group the pack draws last. Pure functions only, so the grouping and
// the keyboard cursor can be tested without a DOM.
//
// Non-purpose: no fetching, no ranking of its own — the core already fused the
// legs and this renders its order.

import type { components } from "@marula-core/sdk";
import type { IconName } from "@marula-core/ui/kit";

export type SearchResult = components["schemas"]["SearchResult"];

/** Same order and labels as apps/web/app/search/page.tsx, which this lane
 *  must not edit. Keep the two in step if the search page's groups change. */
export const GROUP_ORDER = [
  "ticket",
  "comment",
  "client",
  "document",
  "kb_article",
  "site",
  "contact",
] as const;

export const GROUP_LABEL: Record<string, string> = {
  ticket: "Tickets",
  comment: "Comments",
  client: "Clients",
  site: "Sites",
  contact: "Contacts",
  kb_article: "KB articles",
  document: "Documents",
};

const GROUP_ICON: Record<string, IconName> = {
  ticket: "ticket",
  comment: "chat",
  client: "users",
  site: "monitor",
  contact: "user",
  kb_article: "book",
  document: "file",
};

export type ResultGroup = {
  key: string;
  label: string;
  icon: IconName;
  hits: SearchResult[];
};

export function groupOf(kind: string): string {
  if (kind in GROUP_LABEL) return kind;
  return "client";
}

export function groupResults(results: SearchResult[]): ResultGroup[] {
  const buckets = new Map<string, SearchResult[]>();
  for (const hit of results) {
    const key = groupOf(hit.entity_kind);
    const list = buckets.get(key) ?? [];
    list.push(hit);
    buckets.set(key, list);
  }
  const groups: ResultGroup[] = [];
  for (const key of GROUP_ORDER) {
    const hits = buckets.get(key);
    if (hits === undefined || hits.length === 0) continue;
    groups.push({ key, label: GROUP_LABEL[key] ?? key, icon: GROUP_ICON[key] ?? "search", hits });
  }
  return groups;
}

/** Where a result opens. `null` means the palette can show it but not open it
 *  (sites and contacts have no record route yet, exactly as on /search). */
export function resultHref(hit: SearchResult): string | null {
  if (hit.entity_kind === "ticket") return `/tickets/${hit.entity_id}`;
  if (hit.entity_kind === "comment" && hit.parent_id) return `/tickets/${hit.parent_id}`;
  if (hit.entity_kind === "client") return `/clients/${hit.entity_id}`;
  if (hit.entity_kind === "document") return `/documents/${hit.entity_id}`;
  if (hit.entity_kind === "kb_article") return `/kb/${hit.entity_id}`;
  return null;
}

/** A one-line secondary under the title. The search payload carries a body
 *  excerpt, not a client/status pair, so that is what it says. */
export function resultMeta(hit: SearchResult): string {
  const snippet = hit.snippet.replace(/\s+/g, " ").trim();
  if (snippet !== "") return snippet;
  return GROUP_LABEL[groupOf(hit.entity_kind)] ?? hit.entity_kind;
}

export type PaletteAction = {
  id: string;
  label: string;
  hint: string;
  href: string;
  icon: IconName;
};

/** Real destinations only. There is no create flow outside the board dialog,
 *  so the palette does not advertise a "New ticket" that nothing answers. */
export const ACTIONS: PaletteAction[] = [
  { id: "action-today", label: "Go to Today", hint: "Jump", href: "/today", icon: "today" },
  { id: "action-dispatch", label: "Go to Dispatch", hint: "Jump", href: "/dispatch", icon: "calendar" },
  { id: "action-search", label: "Open the full search page", hint: "Search", href: "/search", icon: "search" },
];

export type FlatRow =
  | { kind: "result"; href: string; hit: SearchResult }
  | { kind: "action"; href: string; action: PaletteAction };

/** The keyboard cursor only ever lands on a row Enter can open. */
export function flatOpenable(groups: ResultGroup[], actions: PaletteAction[]): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const group of groups) {
    for (const hit of group.hits) {
      const href = resultHref(hit);
      if (href !== null) rows.push({ kind: "result", href, hit });
    }
  }
  for (const action of actions) rows.push({ kind: "action", href: action.href, action });
  return rows;
}

/** A stable key for a rendered row. */
export function rowKey(row: FlatRow): string {
  return row.kind === "result"
    ? `result:${row.hit.entity_kind}:${row.hit.entity_id}`
    : `action:${row.action.id}`;
}
