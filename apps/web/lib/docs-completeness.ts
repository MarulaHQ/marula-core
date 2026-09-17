// Documentation completeness scoring per client (Wave 17 lane 2).
//
// Purpose: a pure, honest completeness signal for a client's
// documentation posture, scored ENTIRELY from client-side metadata the
// pages already load (sites, contacts, assets, documents, portal
// users). No scoring API exists yet — this renders from metadata and
// the backend need is filed back UNCHECKED (never fake data). When a
// metadata leg is UNAVAILABLE to the caller (typed 403, role-gated) it
// is passed as null: the score PRO-RATES over the legs that are known,
// and the basis line says so — never a silent zero that pretends the
// client is undocumentable.
// Non-purpose: no fetches here — callers pass already-loaded metadata.

import type { components } from "@meridian/sdk";

type Document = components["schemas"]["Document"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type Site = components["schemas"]["Site"];
type Contact = components["schemas"]["Contact"];
type DeviceAsset = components["schemas"]["DeviceAsset"];
type PortalUser = components["schemas"]["PortalUser"];

export interface CompletenessGap {
  key: string;
  label: string;
  hint: string;
}

export interface ClientCompleteness {
  /** 0–100, pro-rated over the metadata legs the caller could load. */
  score: number;
  gaps: CompletenessGap[];
  /** Human basis line: the counts this score was computed from. */
  basis: string;
}

export interface CompletenessInput {
  sites: Site[] | null;
  contacts: Contact[] | null;
  assets: DeviceAsset[] | null;
  documents: Document[] | null;
  layouts: DocumentLayout[] | null;
  portalUsers: PortalUser[] | null;
}

/** Completeness window: documents staleness beyond this dents freshness. */
export const FRESH_WINDOW_DAYS = 180;

function proRate(
  earned: number,
  maxKnown: number,
): number {
  if (maxKnown <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((earned / maxKnown) * 100)));
}

/**
 * Score one client's documentation posture from loaded metadata.
 * Weights (sum 100): contacts-with-email 25, sites 20, a typed
 * document 25 (an untyped one 10), assets 15, portal users 5,
 * freshness 10 (document updated inside the window; weight counts in
 * the maximum only when documents exist). Null legs are unknown and
 * pro-rated out of the maximum.
 */
export function clientCompleteness(input: CompletenessInput): ClientCompleteness {
  const gaps: CompletenessGap[] = [];

  // Contacts (25): at least one contact with a reachable email.
  let contactsEarned = 0;
  let contactsMax = 0;
  if (input.contacts !== null) {
    contactsMax = 25;
    if (input.contacts.length === 0) {
      gaps.push({
        key: "contacts",
        label: "No contacts",
        hint: "Add at least one contact so dispatch and tickets have a named person.",
      });
    } else {
      const withEmail = input.contacts.some((c) => (c.email ?? "") !== "");
      if (withEmail) contactsEarned = 25;
      else
        gaps.push({
          key: "contact-email",
          label: "Contacts lack an email",
          hint: "Add an email to the primary contact so notifications and portal invites can reach a person.",
        });
      const primary = input.contacts.find((c) => c.is_primary);
      if (primary === undefined)
        gaps.push({
          key: "primary-contact",
          label: "No primary contact",
          hint: "Mark one contact as primary — approvals and outage notices route to the primary.",
        });
    }
  }

  // Sites (20): at least one physical location.
  let sitesEarned = 0;
  let sitesMax = 0;
  if (input.sites !== null) {
    sitesMax = 20;
    if (input.sites.length === 0)
      gaps.push({
        key: "sites",
        label: "No sites",
        hint: "Add the client's site(s) — travel estimates and routing require at least one.",
      });
    else sitesEarned = 20;
  }

  // Documents (25 typed / 10 untyped when any doc exists).
  let documentsEarned = 0;
  let documentsMax = 0;
  let freshnessEarned = 0;
  let freshnessMax = 0;
  if (input.documents !== null) {
    documentsMax = 25;
    const layoutIds = new Set(
      (input.layouts ?? []).map((l) => l.id),
    );
    const hasTypedDoc = input.documents.some(
      (d) => d.layout_id != null && layoutIds.has(d.layout_id),
    );
    if (input.documents.length === 0) {
      gaps.push({
        key: "documents",
        label: "No documents",
        hint: "Document the environment — manifests, credential references, and runbooks live here.",
      });
    } else {
      documentsEarned = hasTypedDoc ? 25 : 10;
      if (!hasTypedDoc)
        gaps.push({
          key: "typed-document",
          label: "No typed documents",
          hint: "Use a typed layout (e.g. network, service credentials) so fields structure the record.",
        });
      // Freshness (10) rides the document window.
      const newestDoc = input.documents.reduce<number>(
        (max, d) => Math.max(max, Date.parse(d.updated_at)),
        0,
      );
      const fresh = newestDoc !== 0 && Date.now() - newestDoc < FRESH_WINDOW_DAYS * 24 * 3600 * 1000;
      freshnessMax = 10;
      if (fresh) freshnessEarned = 10;
      else
        gaps.push({
          key: "freshness",
          label: "Documents stale",
          hint: "The newest document is older than 180 days — review and re-verify.",
        });
    }
  }

  // Assets (15): at least one anchored endpoint.
  let assetsEarned = 0;
  let assetsMax = 0;
  if (input.assets !== null) {
    assetsMax = 15;
    if (input.assets.length === 0)
      gaps.push({
        key: "assets",
        label: "No assets",
        hint: "Add at least the key endpoints — assets anchor tickets, secrets, and integrations.",
      });
    else assetsEarned = 15;
  }

  // Portal (5): a client-facing login exists.
  let portalEarned = 0;
  let portalMax = 0;
  if (input.portalUsers !== null) {
    portalMax = 5;
    if (input.portalUsers.length === 0)
      gaps.push({
        key: "portal",
        label: "No portal users",
        hint: "Provision a portal login so the client can raise tickets and view invoices.",
      });
    else portalEarned = 5;
  }

  const earned = contactsEarned + sitesEarned + documentsEarned + freshnessEarned + assetsEarned + portalEarned;
  const maxKnown = contactsMax + sitesMax + documentsMax + freshnessMax + assetsMax + portalMax;

  const count = (v: unknown[] | null) => (v === null ? "n/a" : String(v.length));
  const basis = `${count(input.sites)} site(s) · ${count(input.contacts)} contact(s) · ${count(input.assets)} asset(s) · ${count(input.documents)} document(s) · ${count(input.portalUsers)} portal user(s) · ${input.layouts === null ? "n/a" : `${count(input.layouts)} type(s)`}`;

  return { score: proRate(earned, maxKnown), gaps, basis };
}

/** Tone for the score pill (AA pairs ride the semantic status tokens). */
export function scoreTone(score: number): "success" | "warning" | "danger" {
  if (score >= 75) return "success";
  if (score >= 40) return "warning";
  return "danger";
}
