// Contact activity: the typed read behind the client record's Last contact
// column.
//
// The server derives this from email on the client's tickets, portal replies
// and calls. Nothing is written. The client is raw-fetch because the generated
// SDK is regenerated in one batch at the end of the lane; the route is stable
// and the shape is checked here.

import { apiBaseUrl, loadTokens } from "./api";
import { relativeTime } from "./ticket-ops";

export type ContactActivityEvent = {
  at: string;
  kind: string;
  detail: string | null;
};

export type ContactActivity = {
  contact_id: string;
  last_at: string | null;
  last_kind: string | null;
  recent: ContactActivityEvent[];
};

const KIND_LABEL: Record<string, string> = {
  email_in: "Emailed us",
  email_out: "We emailed",
  portal_reply: "Portal reply",
  call_in: "Called us",
  call_out: "We called",
};

export function contactKindLabel(kind: string | null | undefined): string {
  if (kind === null || kind === undefined) return "Contact";
  return KIND_LABEL[kind] ?? "Contact";
}

/** "Portal reply · 2d ago", or "Never" when nothing has been recorded. */
export function contactActivitySummary(row: ContactActivity | undefined): string {
  if (row === undefined || row.last_at === null) return "Never";
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return `${contactKindLabel(row.last_kind)} · ${relativeTime(row.last_at, zone)}`;
}

/** Every contact's activity for one client, keyed by contact id. */
export async function fetchContactActivity(clientId: string): Promise<Record<string, ContactActivity>> {
  const tokens = loadTokens();
  if (tokens === null) return {};
  try {
    const response = await fetch(
      `${apiBaseUrl()}/api/v1/clients/${encodeURIComponent(clientId)}/contacts/activity`,
      {
        headers: {
          accept: "application/json",
          authorization: `Bearer ${tokens.access_token}`,
        },
      },
    );
    if (!response.ok) return {};
    const rows = (await response.json().catch(() => null)) as ContactActivity[] | null;
    if (rows === null) return {};
    return Object.fromEntries(rows.map((row) => [row.contact_id, row]));
  } catch {
    return {};
  }
}
