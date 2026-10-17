// Portal API wiring for the client portal (portal v1).
//
// Purpose: token persistence is MEMORY-ONLY under a separate module jar
// (Wave 19 lane 5 — the portal refresh secret rides the httpOnly
// marula_portal cookie the API sets), typed fetch helpers for
// the /api/v1/portal/* surface, and a React context owning portal auth state.
// Deliberately independent of lib/api.tsx so staff and portal sessions can
// never bleed into each other in one browser profile.
// Non-purpose: no SDK client usage (the portal is a small dedicated
// surface), no staff endpoints, no refresh scheduling beyond on-401
// rotation.
"use client";

import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";

import { resolveApiBaseUrl } from "./api";

/** Request paths on the canonical versioned surface only. The portal owns
 * hand-written transport (no generated client), so this is the narrow
 * anchor keeping it off undocumented legacy aliases forever. */
type PortalPath = `/api/v1/portal/${string}`;

const TOKENS_KEY = "marula-portal-tokens";

export interface PortalTokens {
  access_token: string;
  refresh_token: string;
  session_id: string;
  expires_in: number;
}

export interface PortalProfile {
  id: string;
  org_id: string;
  client_id: string;
  email: string;
  display_name: string;
  role: "contact" | "approver" | "it_manager";
}

interface PortalTicket {
  id: string;
  number: string;
  title: string;
  /** The client's own opening words, carried on the detail's ticket (the
   *  pack's thread leads with it). Absent from the list row. */
  body?: string;
  status_id: string;
  category: "open" | "in_progress" | "waiting" | "closed";
  /** The status's own words when the org allows clients to see them; null
   * when the status is client-hidden, and the client shows the category word
   * instead (Wave 41 lane 12). */
  status_label?: string | null;
  priority: "p1" | "p2" | "p3" | "p4";
  board_id: string;
  created_at: string;
  updated_at: string;
  resolved_at?: string | null;
  due_at?: string | null;
}

interface PortalComment {
  id: string;
  kind: "internal" | "client_visible" | "in_house_update";
  body: string;
  created_at: string;
}

/** CSAT survey state (Wave 34 lane 2): `valid` renders the form,
 * `responded` renders the thank-you state. */
export interface PortalCsatView {
  ticket_id: string;
  ticket_number: string;
  ticket_subject: string;
  status: "valid" | "responded";
  expires_at: string;
}

export interface PortalTicketDetail {
  ticket: PortalTicket;
  comments: PortalComment[];
  can_view_in_house_updates: boolean;
}

let memoryTokens: PortalTokens | null = null;
let memoryTokensExpAtMs = 0;

function loadTokens(): PortalTokens | null {
  if (typeof window === "undefined") return null;
  return memoryTokens;
}

function saveTokens(tokens: PortalTokens): void {
  memoryTokens = tokens;
  memoryTokensExpAtMs = Date.now() + tokens.expires_in * 1000 - 30_000;
  // One-time legacy-storage hygiene (the cookie owns the secret now).
  window.localStorage.removeItem(TOKENS_KEY);
  window.localStorage.removeItem(`${TOKENS_KEY}-exp`);
}

/** The ambient double-submit csrf value (`marula_csrf=...`). */
function csrfHeader(): string | null {
  if (typeof document === "undefined") return null;
  const pair = document.cookie
    .split(";")
    .map((c) => c.trim())
    .find((c) => c.startsWith("marula_csrf="));
  return pair ? pair.slice("marula_csrf=".length) : null;
}

export function clearPortalTokens(): void {
  memoryTokens = null;
  memoryTokensExpAtMs = 0;
}

const API_BASE = resolveApiBaseUrl();

/**
 * One rotation, shared. A full page load fires several bearer-less calls at
 * once — the provider's /portal/me restore probe plus the page's first data
 * fetch. Every 401 rotates the same cookie-only pair, and the server retires
 * the pair on first use, so each concurrent loser draws a replay 401 and the
 * session reads as dead: either the provider flips to anonymous (the guard
 * then bounces a perfectly good session to /portal/login) or the page's
 * first fetch fails. Callers share the single in-flight rotation and each
 * retries its own request with the fresh bearer (Wave 47 lane A1).
 */
let rotationInFlight: Promise<PortalTokens> | null = null;

function rotateSession(stored: PortalTokens | null): Promise<PortalTokens> {
  if (rotationInFlight !== null) return rotationInFlight;
  const attempt = (async () => {
    // Single rotation attempt; a second 401 means truly logged out. The
    // body credential is used when held; otherwise the httpOnly cookie
    // pair drives the rotation (empty body). The rotation is attempted on
    // EVERY 401 rather than gated on a visible cookie check: the real
    // session cookie is httpOnly, so document.cookie never reports it and
    // such a gate would disable the only recovery path a full page load
    // has (Wave 34 row 11b). A browser with no session simply gets a 401
    // back, which is the same answer it had before.
    const rotated = await fetch(`${API_BASE}/api/v1/portal/refresh`, {
      method: "POST",
      credentials: "include",
      headers: {
        "content-type": "application/json",
        ...(csrfHeader() ? { "x-marula-csrf": csrfHeader() as string } : {}),
      },
      body: JSON.stringify(
        stored?.refresh_token
          ? {
              refresh_token: stored.refresh_token,
              session_id: stored.session_id,
            }
          : {},
      ),
    });
    if (!rotated.ok) throw new PortalError("unauthorized", 401);
    const next = (await rotated.json()) as PortalTokens & { portal_user: PortalProfile };
    saveTokens(next);
    return next;
  })();
  rotationInFlight = attempt;
  // Free the slot once settled; every awaiter already holds the resolved tokens
  // (or the shared rejection) in the promise value.
  //
  // `.then(release, release)` rather than `.finally()`: `finally` returns a NEW
  // promise that re-rejects, and this bookkeeping promise is deliberately not
  // awaited by anyone, so a failed rotation surfaced as an unhandled rejection
  // on this line's own throw. Vitest reports that and exits 1 even with every
  // test green, which failed any lane gating on the web suite (Wave 47 lane
  // A27). Handling both arms here changes nothing for callers: they await
  // `attempt`, which still rejects as before.
  const release = () => {
    if (rotationInFlight === attempt) rotationInFlight = null;
  };
  attempt.then(release, release);
  return attempt;
}

/**
 * One fetch wrapper for every portal call: attaches the access token,
 * rotates the refresh token once on 401, and clears state on hard logout.
 */
async function portalFetch<T>(
  path: PortalPath,
  init: RequestInit & { tokens?: PortalTokens | null },
): Promise<T> {
  const { tokens, ...rest } = init;
  const stored = tokens ?? loadTokens();
  const headers: Record<string, string> = {
    "content-type": "application/json",
    ...(stored?.access_token ? { authorization: `Bearer ${stored.access_token}` } : {}),
  };
  // Cookie-session CSRF double-submit with the ambient jar.
  if (typeof document !== "undefined") {
    const csrf = document.cookie
      .split(";")
      .map((c) => c.trim())
      .find((c) => c.startsWith("marula_csrf="));
    if (csrf) headers["x-marula-csrf"] = csrf.slice("marula_csrf=".length);
  }
  const response = await fetch(`${API_BASE}${path}`, {
    ...rest,
    credentials: "include",
    headers: {
      ...headers,
      ...(rest.headers as Record<string, string> | undefined),
    },
  });
  if (response.status === 401) {
    try {
      const next = await rotateSession(stored);
      const retry = await fetch(`${API_BASE}${path}`, {
        ...rest,
        credentials: "include",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${next.access_token}`,
          ...(rest.headers as Record<string, string> | undefined),
          ...(csrfHeader() ? { "x-marula-csrf": csrfHeader() as string } : {}),
        },
      });
      if (retry.ok) return (await retry.json()) as T;
    } catch {
      // The shared rotation failed (genuinely logged out, or a sibling
      // tab already retired the pair): fall through to the hard logout.
    }
    clearPortalTokens();
    throw new PortalError("unauthorized", 401);
  }
  if (!response.ok) {
    let message = `request failed (${response.status})`;
    try {
      const payload = (await response.json()) as { error?: { message?: string } };
      message = payload.error?.message ?? message;
    } catch {
      // non-JSON error body — keep the generic message
    }
    throw new PortalError(message, response.status);
  }
  return (await response.json()) as T;
}

export class PortalError extends Error {
  readonly status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

export const portalApi = {
  async login(email: string, password: string): Promise<PortalTokens & { portal_user: PortalProfile }> {
    const result = await portalFetch<
      PortalTokens & { portal_user: PortalProfile }
    >("/api/v1/portal/login", {
      method: "POST",
      tokens: null,
      body: JSON.stringify({ email, password }),
    });
    saveTokens(result);
    return result;
  },

  /**
   * Passwordless sign-in: ask for a link.
   *
   * Deny-by-default on the server, so the answer is identical for every
   * address. The UI must therefore never claim a message was sent to a
   * particular person — it says "if an account matches".
   */
  async requestMagicLink(email: string): Promise<void> {
    await portalFetch<{ ok: boolean }>("/api/v1/portal/magic-link/request", {
      method: "POST",
      tokens: null,
      body: JSON.stringify({ email }),
    });
  },

  /**
   * Spend a sign-in link. Stores the portal session, so a confirmed link lands
   * the client exactly where the password path would have.
   */
  async confirmMagicLink(
    token: string,
  ): Promise<PortalTokens & { portal_user: PortalProfile }> {
    const result = await portalFetch<
      PortalTokens & { portal_user: PortalProfile }
    >("/api/v1/portal/magic-link/confirm", {
      method: "POST",
      tokens: null,
      body: JSON.stringify({ token }),
    });
    saveTokens(result);
    return result;
  },

  me(): Promise<PortalProfile> {
    return portalFetch<PortalProfile>("/api/v1/portal/me", {});
  },

  listTickets(): Promise<PortalTicket[]> {
    return portalFetch<PortalTicket[]>("/api/v1/portal/tickets?limit=200", {});
  },

  getTicket(id: string): Promise<PortalTicketDetail> {
    return portalFetch<PortalTicketDetail>(`/api/v1/portal/tickets/${id}`, {});
  },

  reply(id: string, body: string): Promise<PortalComment> {
    return portalFetch<PortalComment>(`/api/v1/portal/tickets/${id}/reply`, {
      method: "POST",
      body: JSON.stringify({ body }),
    });
  },

  /** CSAT surveys on resolution (Wave 34 lane 2): tokenized, single-use,
   * scoped to the signed-in client. The view never burns the token. */
  csatView(token: string): Promise<PortalCsatView> {
    return portalFetch<PortalCsatView>(`/api/v1/portal/csat/${token}`, {});
  },

  csatSubmit(token: string, score: number, comment?: string): Promise<PortalCsatView> {
    return portalFetch<PortalCsatView>(`/api/v1/portal/csat/${token}`, {
      method: "POST",
      body: JSON.stringify({ score, comment: comment || undefined }),
    });
  },

  // --- portal v2: invoices, KB self-service, appointment requests ----------

  listInvoices(): Promise<PortalInvoice[]> {
    return portalFetch<PortalInvoice[]>("/api/v1/portal/invoices", {});
  },

  getInvoice(id: string): Promise<PortalInvoiceDetail> {
    return portalFetch<PortalInvoiceDetail>(`/api/v1/portal/invoices/${id}`, {});
  },

  /** Ledger-only settlement through the PaymentProvider seam (none-mode:
   * nothing is charged externally; the receipt records the settlement). */
  payInvoice(id: string, amountMinor: number, notes?: string): Promise<PortalPayResponse> {
    return portalFetch<PortalPayResponse>(`/api/v1/portal/invoices/${id}/pay`, {
      method: "POST",
      body: JSON.stringify({
        amount_minor: amountMinor,
        notes: notes ?? null,
        idempotency_key: `portal-pay-${crypto.randomUUID()}`,
      }),
    });
  },

  /** Attachment attachments on one ticket (portal v2 pins): metadata only;
   * bytes stream on download. */
  listTicketAttachments(id: string): Promise<PortalAttachmentView[]> {
    return portalFetch<PortalAttachmentView[]>(`/api/v1/portal/tickets/${id}/attachments`, {});
  },

  /** The attachment bytes as a blob URL (bearer-fetched; the caller opens
   * and later revokes it). */
  async ticketAttachmentUrl(id: string, attachmentId: string): Promise<string> {
    const response = await fetch(
      `${API_BASE}/api/v1/portal/tickets/${id}/attachments/${attachmentId}/download`,
      { headers: { authorization: `Bearer ${loadTokens()?.access_token ?? ""}` } },
    );
    if (!response.ok) throw new PortalError("attachment unavailable", response.status);
    return URL.createObjectURL(await response.blob());
  },

  kbCategories(): Promise<PortalKbCategory[]> {
    return portalFetch<PortalKbCategory[]>("/api/v1/portal/kb/categories", {});
  },

  kbArticles(categoryId?: string): Promise<PortalKbArticle[]> {
    const query = categoryId ? `?category_id=${encodeURIComponent(categoryId)}` : "";
    return portalFetch<PortalKbArticle[]>(`/api/v1/portal/kb/articles${query}`, {});
  },

  kbArticle(id: string): Promise<PortalKbArticle> {
    return portalFetch<PortalKbArticle>(`/api/v1/portal/kb/articles/${id}`, {});
  },

  kbSearch(q: string): Promise<PortalKbSearchHit[]> {
    return portalFetch<PortalKbSearchHit[]>(
      `/api/v1/portal/kb/search?q=${encodeURIComponent(q)}`,
      {},
    );
  },

  scheduleSlots(from: string, to: string): Promise<PortalStaffSlots[]> {
    return portalFetch<PortalStaffSlots[]>(
      `/api/v1/portal/schedule/slots?from=${from}&to=${to}`,
      {},
    );
  },

  listAppointmentRequests(): Promise<PortalAppointmentRequest[]> {
    return portalFetch<PortalAppointmentRequest[]>(
      "/api/v1/portal/appointment-requests",
      {},
    );
  },

  createAppointmentRequest(body: {
    user_id: string;
    title: string;
    starts_at: string;
    ends_at: string;
    kind?: "on_site" | "remote";
    notes?: string;
  }): Promise<{ id: string; status: string }> {
    return portalFetch<{ id: string; status: string }>(
      "/api/v1/portal/appointment-requests",
      {
        method: "POST",
        body: JSON.stringify({
          ...body,
          idempotency_key: `portal-req-${crypto.randomUUID()}`,
        }),
      },
    );
  },

  cancelAppointmentRequest(id: string): Promise<{ status: string }> {
    return portalFetch<{ status: string }>(
      `/api/v1/portal/appointment-requests/${id}/cancel`,
      { method: "POST", body: JSON.stringify({}) },
    );
  },

  // --- portal v2 wave 34: quote approval -----------------------------------

  listQuotes(): Promise<PortalQuote[]> {
    return portalFetch<PortalQuote[]>("/api/v1/portal/quotes", {});
  },

  getQuote(id: string): Promise<PortalQuoteDetail> {
    return portalFetch<PortalQuoteDetail>(`/api/v1/portal/quotes/${id}`, {});
  },

  /** Approve (accept) a sent quote — the same sent → accepted lifecycle
   * leg the provider's staff flow runs. */
  approveQuote(id: string, reason?: string): Promise<PortalQuoteDecisionResponse> {
    return portalFetch<PortalQuoteDecisionResponse>(`/api/v1/portal/quotes/${id}/approve`, {
      method: "POST",
      body: JSON.stringify({
        reason: reason ?? null,
        idempotency_key: `portal-quote-approve-${crypto.randomUUID()}`,
      }),
    });
  },

  /** Decline a sent quote, optionally with a reason (audit-only). */
  declineQuote(id: string, reason?: string): Promise<PortalQuoteDecisionResponse> {
    return portalFetch<PortalQuoteDecisionResponse>(`/api/v1/portal/quotes/${id}/decline`, {
      method: "POST",
      body: JSON.stringify({
        reason: reason ?? null,
        idempotency_key: `portal-quote-decline-${crypto.randomUUID()}`,
      }),
    });
  },

  /** The invoice's print-ready HTML document as a blob URL (bearer-fetched;
   * the caller opens and later revokes it). */
  async invoiceDocumentUrl(id: string): Promise<string> {
    const response = await fetch(`${API_BASE}/api/v1/portal/invoices/${id}/document`, {
      headers: { authorization: `Bearer ${loadTokens()?.access_token ?? ""}` },
    });
    if (!response.ok) throw new PortalError("document unavailable", response.status);
    return URL.createObjectURL(await response.blob());
  },
};

/** The client-facing invoice row (issued | paid; void/draft never leave
 * the staff surface). */
export interface PortalInvoice {
  id: string;
  number: string;
  status: "issued" | "paid";
  currency: string;
  issued_at: string;
  due_on: string;
  total_minor: number;
  total_paid_minor: number;
  outstanding_minor: number;
  created_at: string;
}

export interface PortalInvoiceLine {
  id: string;
  kind: string;
  description: string;
  quantity_minutes: number;
  unit_amount_minor: number;
  amount_minor: number;
}

export interface PortalPayment {
  id: string;
  amount_minor: number;
  currency: string;
  provider: string;
  received_at: string;
}

export interface PortalInvoiceDetail {
  invoice: PortalInvoice;
  lines: PortalInvoiceLine[];
  payments: PortalPayment[];
}

export interface PortalPayResponse {
  payment: PortalPayment;
  invoice: PortalInvoice;
}

/** Portal attachment view (pins: metadata only, bytes stream on download). */
export interface PortalAttachmentView {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
  created_at: string;
}

export interface PortalKbCategory {
  id: string;
  name: string;
  description: string;
}

export interface PortalKbArticle {
  id: string;
  category_id: string | null;
  title: string;
  body: string;
  updated_at: string;
}

export interface PortalKbSearchHit {
  article: PortalKbArticle;
  score: number;
}

export interface PortalSlotInterval {
  starts_at: string;
  ends_at: string;
}

export interface PortalStaffSlots {
  user_id: string;
  display_name: string;
  slots: PortalSlotInterval[];
}

export interface PortalAppointmentRequest {
  id: string;
  user_id: string;
  title: string;
  starts_at: string;
  ends_at: string;
  kind: "on_site" | "remote" | "internal" | "personal";
  notes: string | null;
  status: "pending" | "approved" | "declined" | "cancelled";
  decision_note: string | null;
  created_at: string;
}

interface PortalAuthState {
  me: PortalProfile | null;
  status: "loading" | "anonymous" | "authed";
  /** Runs the real login, then publishes the session to the provider. The
   *  login page must call this rather than `portalApi.login` directly:
   *  `restore()` ran once at mount and found nothing, so the provider is
   *  still `anonymous` when the credentials land — and
   *  `useRequirePortalAuth` would bounce the freshly signed-in page
   *  straight back to /portal/login (Wave 34 row 11a). */
  signIn(email: string, password: string): Promise<void>;
  signOut(): void;
}

const PortalAuthContext = createContext<PortalAuthState>({
  me: null,
  status: "loading",
  signIn: async () => {},
  signOut: () => {},
});

/** Wraps the /portal tree: restores stored tokens and validates them via /api/v1/portal/me. */
export function PortalAuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<PortalProfile | null>(null);
  const [status, setStatus] = useState<PortalAuthState["status"]>("loading");
  // Session-generation guard (Wave 47 lane A1, follow-up 11a). The mount-time
  // restore probe below can still be in flight when a sign-in lands — a cold
  // first load, a slow API, a paste-and-submit faster than the probe. Its
  // answer describes the session as it was BEFORE the login, so applying it
  // afterwards would publish `anonymous` over a session the provider had
  // already accepted, and useRequirePortalAuth would bounce the freshly
  // signed-in client straight back to /portal/login. Every signIn/signOut
  // bumps the generation; a restore result only lands when its generation is
  // still current. The guard itself is untouched — a restore that settles
  // with no session still publishes `anonymous` and still bounces.
  const sessionGeneration = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const generation = sessionGeneration.current;
    async function restore() {
      // No local short-circuit: the auth cookie is HttpOnly, so it never
      // shows up in document.cookie — the /portal/me probe below is the
      // only honest check (the browser attaches the cookie itself). On a
      // hard reload the probe goes out bearer-less and rides the 401
      // cookie-driven rotation in portalFetch: the httpOnly jar holds the
      // session pair, so the rotation answers with a fresh access token
      // and the retry lands signed-in.
      try {
        const profile = await portalApi.me();
        if (cancelled || generation !== sessionGeneration.current) return;
        setMe(profile);
        setStatus("authed");
      } catch {
        if (cancelled || generation !== sessionGeneration.current) return;
        setStatus("anonymous");
      }
    }
    void restore();
    return () => {
      cancelled = true;
    };
  }, []);

  const signIn = useCallback(async (email: string, password: string) => {
    const result = await portalApi.login(email, password);
    // Any restore still in flight is now stale: it predates this session.
    sessionGeneration.current += 1;
    setMe(result.portal_user);
    setStatus("authed");
  }, []);

  const signOut = useCallback(() => {
    clearPortalTokens();
    sessionGeneration.current += 1;
    setMe(null);
    setStatus("anonymous");
  }, []);

  const value = useMemo(
    () => ({ me, status, signIn, signOut }),
    [me, status, signIn, signOut],
  );
  return <PortalAuthContext.Provider value={value}>{children}</PortalAuthContext.Provider>;
}

export function usePortalAuth(): PortalAuthState {
  return useContext(PortalAuthContext);
}

/** Guard hook: pushes to the portal login when anonymous. */
export function useRequirePortalAuth(): void {
  const { status } = usePortalAuth();
  const router = useRouter();
  useEffect(() => {
    if (status === "anonymous") router.replace("/portal/login");
  }, [status, router]);
}

/** The client-facing quote row (sent | accepted | declined | expired;
 * drafts never leave the staff surface). */
export interface PortalQuote {
  id: string;
  number: string;
  status: "sent" | "accepted" | "declined" | "expired";
  currency: string;
  issued_on: string;
  valid_until: string;
  notes: string | null;
  terms: string | null;
  subtotal_minor: number;
  tax_rate_bps: number;
  tax_minor: number;
  total_minor: number;
  sent_at: string | null;
  decided_at: string | null;
  created_at: string;
}

export interface PortalQuoteLine {
  description: string;
  unit: string;
  quantity_milli: number;
  unit_amount_minor: number;
  taxable: boolean;
  amount_minor: number;
  position: number;
}

export interface PortalQuoteDetail extends PortalQuote {
  lines: PortalQuoteLine[];
}

/** Decision response: the refreshed quote (flattened) plus instant-first
 * mutation metadata the portal ignores. */
export type PortalQuoteDecisionResponse = PortalQuote & { mutation?: unknown };
