// Portal API wiring for the client portal (portal v1).
//
// Purpose: token persistence is MEMORY-ONLY under a separate module jar
// (Wave 19 lane 5 — the portal refresh secret rides the httpOnly
// meridian_portal cookie the API sets), typed fetch helpers for
// the /api/v1/portal/* surface, and a React context owning portal auth state.
// Deliberately independent of lib/api.tsx so staff and portal sessions can
// never bleed into each other in one browser profile.
// Non-purpose: no SDK client usage (the portal is a small dedicated
// surface), no staff endpoints, no refresh scheduling beyond on-401
// rotation.
"use client";

import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import { resolveApiBaseUrl } from "./api";

/** Request paths on the canonical versioned surface only. The portal owns
 * hand-written transport (no generated client), so this is the narrow
 * anchor keeping it off undocumented legacy aliases forever. */
type PortalPath = `/api/v1/portal/${string}`;

const TOKENS_KEY = "meridian-portal-tokens";

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
  status_id: string;
  category: "open" | "in_progress" | "waiting" | "closed";
  priority: "p1" | "p2" | "p3" | "p4";
  board_id: string;
  created_at: string;
  updated_at: string;
  resolved_at?: string | null;
}

interface PortalComment {
  id: string;
  kind: "internal" | "client_visible" | "in_house_update";
  body: string;
  created_at: string;
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

/** The ambient double-submit csrf value (`meridian_csrf=...`). */
function csrfHeader(): string | null {
  if (typeof document === "undefined") return null;
  const pair = document.cookie
    .split(";")
    .map((c) => c.trim())
    .find((c) => c.startsWith("meridian_csrf="));
  return pair ? pair.slice("meridian_csrf=".length) : null;
}

export function clearPortalTokens(): void {
  memoryTokens = null;
  memoryTokensExpAtMs = 0;
}

const API_BASE = resolveApiBaseUrl();

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
      .find((c) => c.startsWith("meridian_csrf="));
    if (csrf) headers["x-meridian-csrf"] = csrf.slice("meridian_csrf=".length);
  }
  const response = await fetch(`${API_BASE}${path}`, {
    ...rest,
    credentials: "include",
    headers: {
      ...headers,
      ...(rest.headers as Record<string, string> | undefined),
    },
  });
  if (response.status === 401 && (stored?.access_token || document.cookie.includes("meridian_portal="))) {
    // Single rotation attempt; a second 401 means truly logged out. The
    // body credential is used when held; otherwise the httpOnly cookie
    // pair drives the rotation (empty body).
    const rotated = await fetch(`${API_BASE}/api/v1/portal/refresh`, {
      method: "POST",
      credentials: "include",
      headers: {
        "content-type": "application/json",
        ...(csrfHeader() ? { "x-meridian-csrf": csrfHeader() as string } : {}),
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
    if (rotated.ok) {
      const next = (await rotated.json()) as PortalTokens & { portal_user: PortalProfile };
      saveTokens(next);
      const retryHeaders: Record<string, string> = {
        "content-type": "application/json",
        authorization: `Bearer ${next.access_token}`,
        ...(rest.headers as Record<string, string> | undefined),
      };
      if (csrfHeader()) retryHeaders["x-meridian-csrf"] = csrfHeader() as string;
      const retry = await fetch(`${API_BASE}${path}`, {
        ...rest,
        credentials: "include",
        headers: retryHeaders,
      });
      if (retry.ok) return (await retry.json()) as T;
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
  signOut(): void;
}

const PortalAuthContext = createContext<PortalAuthState>({
  me: null,
  status: "loading",
  signOut: () => {},
});

/** Wraps the /portal tree: restores stored tokens and validates them via /api/v1/portal/me. */
export function PortalAuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<PortalProfile | null>(null);
  const [status, setStatus] = useState<PortalAuthState["status"]>("loading");

  useEffect(() => {
    let cancelled = false;
    async function restore() {
      if (loadTokens() === null && !document.cookie.includes("meridian_portal=")) {
        setStatus("anonymous");
        return;
      }
      try {
        const profile = await portalApi.me();
        if (cancelled) return;
        setMe(profile);
        setStatus("authed");
      } catch {
        if (cancelled) return;
        setStatus("anonymous");
      }
    }
    void restore();
    return () => {
      cancelled = true;
    };
  }, []);

  const signOut = useCallback(() => {
    clearPortalTokens();
    setMe(null);
    setStatus("anonymous");
  }, []);

  const value = useMemo(
    () => ({ me, status, signOut }),
    [me, status, signOut],
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
