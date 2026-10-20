// API client wiring for the web app.
//
// Purpose: one MarulaClient bound to NEXT_PUBLIC_API_URL with MEMORY-ONLY
// token persistence (Wave 19 lane 5 — the refresh secret rides the httpOnly
// session cookie the API sets; the access token lives in this module's
// memory and never touches document storage), a React context that owns
// auth state, and helpers for guarded pages + 401 refresh rotation.
// Non-purpose: no server-side data fetching and no token refresh scheduling
// beyond on-401 rotation.
"use client";

import { MarulaClient } from "@marula-core/sdk";
import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { broadcastSession, onBusMessage } from "./popout/window-bus";

const TOKENS_KEY = "marula-tokens";

declare global {
  interface Window {
    /** Injected by the desktop shell (desktop/src/main.rs, ADR 0005). */
    __MARULA_API_URL__?: string;
  }
}

interface StoredTokens {
  access_token: string;
  refresh_token: string;
  session_id: string;
  expires_in: number;
}

/**
 * Memory-only jar (same public surface the localStorage versions had, so
 * every page keeps its calls verbatim — the mechanics changed, not the
 * shapes). Hygiene: the first save ALSO clears the legacy localStorage
 * family this module used to own.
 */
let memoryTokens: StoredTokens | null = null;
let memoryTokensExpAtMs = 0;

export function loadTokens(): StoredTokens | null {
  if (typeof window === "undefined") return null;
  return memoryTokens;
}

export function saveTokens(tokens: StoredTokens): void {
  memoryTokens = tokens;
  memoryTokensExpAtMs = Date.now() + tokens.expires_in * 1000 - 30_000;
  // One-time legacy-storage hygiene (httpOnly cookie owns the secret now).
  window.localStorage.removeItem(TOKENS_KEY);
  window.localStorage.removeItem(`${TOKENS_KEY}-exp`);
}

export function clearTokens(): void {
  memoryTokens = null;
  memoryTokensExpAtMs = 0;
}

/** Desktop shell (ADR 0005) injects the embedded API base before page scripts run. */
export function resolveApiBaseUrl(): string {
  const raw =
    (typeof window !== "undefined" && window.__MARULA_API_URL__) ||
    process.env.NEXT_PUBLIC_API_URL ||
    "http://localhost:8080";
  // The env value can carry a trailing slash; every caller appends "/api/...",
  // so strip it or the request goes to "//api/..." and fails.
  return raw.replace(/\/+$/, "");
}

function createClient(): MarulaClient {
  return new MarulaClient({ baseUrl: resolveApiBaseUrl() });
}

/** App-wide singleton; safe in the browser only (this app is fully client-rendered). */
export const api = createClient();

export interface Me {
  user_id: string;
  org_id: string;
  email: string;
  display_name: string;
  timezone: string;
  permissions: readonly string[];
}

interface AuthState {
  me: Me | null;
  status: "loading" | "anonymous" | "authed";
  logout(): Promise<void>;
  /**
   * Applies the server's own echo of a profile change (display name, timezone)
   * so every reader of `me` — `useTimeFormat()` included — sees the new value
   * without a reload.
   */
  updateMe(patch: Partial<Me>): void;
}

const AuthContext = createContext<AuthState>({
  me: null,
  status: "loading",
  logout: async () => {},
  updateMe: () => {},
});

// --- auth-ready gate (p29-accessibility-deep) --------------------------------
// Full page loads race AuthProvider's cookie-driven refresh against view
// hydration; the loser fires bearer-less requests whose 401s the instant
// store then caches as an empty snapshot (the "board has no statuses" bug).
// Hydrates await this once so the first snapshot carries credentials.
let authReadyResolve: (() => void) | null = null;
const authReadyPromise = new Promise<void>((resolve) => {
  authReadyResolve = resolve;
});

/** Resolves when the first auth restore settles (authed OR anonymous, so a
 * genuinely signed-out page never hangs). Instant-store hydrates await it. */
export function authReady(timeoutMs = 5_000): Promise<void> {
  const timer = setTimeout(() => authReadyResolve?.(), timeoutMs);
  void timer;
  return authReadyPromise;
}

function notifyAuthSettled(): void {
  authReadyResolve?.();
  authReadyResolve = null;
}

/**
 * Wraps the app: restores stored tokens, fetches /api/v1/me once, and rotates
 * the refresh token when the access token has expired. Exposes the identity
 * via {@link useAuth}.
 */
export function AuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  const [status, setStatus] = useState<AuthState["status"]>("loading");

  // Wave 38 session honesty: one logout expires EVERY window (soft
  // sign-out here, matching the local logout path); one re-login restores
  // every window (reload re-bootstraps from the shared session cookie).
  useEffect(() => {
    return onBusMessage((message) => {
      if (message.kind !== "session") return;
      if (message.state === "expired") {
        clearTokens();
        setMe(null);
        setStatus("anonymous");
        notifyAuthSettled();
        return;
      }
      // restored elsewhere: the cookie session is shared, so a reload
      // re-runs the cookie-driven rotation and this window comes back.
      if (typeof window !== "undefined") window.location.reload();
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    async function restore() {
      const stored = loadTokens();
      if (stored === null) {
        // No in-memory tokens (fresh page load): the httpOnly session
        // cookie is the credential — one cookie-driven rotation bootstraps
        // the access token (the server answers the same TokenPair shape).
        try {
          saveTokens(await api.refresh());
        } catch {
          if (cancelled) return;
          setStatus("anonymous");
          notifyAuthSettled();
          return;
        }
      } else if (Date.now() >= memoryTokensExpAtMs) {
        // Proactive rotation keeps long-open tabs working without
        // waiting for a failed request.
        try {
          saveTokens(await api.refresh());
        } catch {
          if (cancelled) return;
          clearTokens();
          setStatus("anonymous");
          notifyAuthSettled();
          return;
        }
      }
      const { data, error } = await api.api.GET("/api/v1/me");
      if (cancelled) return;
      if (error !== undefined) {
        clearTokens();
        setStatus("anonymous");
        notifyAuthSettled();
        return;
      }
      setMe(data);
      setStatus("authed");
      notifyAuthSettled();
    }
    void restore();
    return () => {
      cancelled = true;
    };
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      clearTokens();
      setMe(null);
      setStatus("anonymous");
      broadcastSession("expired");
    }
  }, []);

  const updateMe = useCallback((patch: Partial<Me>) => {
    setMe((current) => (current === null ? current : { ...current, ...patch }));
  }, []);

  const value = useMemo(
    () => ({ me, status, logout, updateMe }),
    [me, status, logout, updateMe],
  );
  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  return useContext(AuthContext);
}

/** Client-side guard: redirects to /login unless authenticated. */
export function useRequireAuth(): AuthState {
  const auth = useAuth();
  const router = useRouter();
  useEffect(() => {
    if (auth.status === "anonymous") {
      router.replace("/login");
    }
  }, [auth.status, router]);
  return auth;
}

// --- attachments (TASKLIST 2.14) --------------------------------------------
// Raw-byte surfaces (octet-stream upload, streamed download) bypass the
// generated client on purpose: openapi-fetch models JSON bodies, while
// attachments are binary with sidecar headers.
//
// The URL literals below are `satisfies`-anchored to the canonical /api/v1
// contract, so a spec rename fails `tsc` here instead of silently 404ing.

import type { MarulaPaths } from "@marula-core/sdk";

const TICKET_ATTACHMENTS_PATH = "/api/v1/tickets/{id}/attachments" satisfies keyof MarulaPaths;
const ATTACHMENT_DOWNLOAD_PATH =
  "/api/v1/attachments/{id}/download" satisfies keyof MarulaPaths;
const DOCUMENT_ATTACHMENTS_PATH =
  "/api/v1/documents/{id}/attachments" satisfies keyof MarulaPaths;
const DOCUMENT_ATTACHMENT_DOWNLOAD_PATH =
  "/api/v1/documents/{id}/attachments/{attachment_id}/download" satisfies keyof MarulaPaths;
const PLUGIN_ZIP_STAGE_PATH = "/api/v1/plugins/distribution/staged/zip" satisfies keyof MarulaPaths;

/** Auth header for manual fetches against the API base URL. */
export function authHeaders(): HeadersInit {
  const tokens = loadTokens();
  return tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` };
}

export function apiBaseUrl(): string {
  return resolveApiBaseUrl();
}

/** Kept as API_BASE for callers binding to the documented constant. */
export const API_BASE = resolveApiBaseUrl();

export interface UploadedAttachment {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
}

/**
 * Uploads one file against a ticket (composer flow: upload first, link the
 * returned ids when the comment is posted). Throws with the server's
 * message on validation failures — callers surface it verbatim.
 */
export async function uploadAttachment(ticketId: string, file: File): Promise<UploadedAttachment> {
  const response = await fetch(
    `${apiBaseUrl()}${TICKET_ATTACHMENTS_PATH.replace("{id}", ticketId)}`,
    {
    method: "POST",
    headers: {
      ...authHeaders(),
      "content-type": "application/octet-stream",
      "x-filename": file.name,
    },
    body: file,
  });
  if (!response.ok) {
    const detail = (await response.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new Error(detail?.error?.message ?? `Upload failed (${response.status})`);
  }
  return (await response.json()) as UploadedAttachment;
}

/**
 * What a download endpoint answers when asked for a ticket: `mode` says
 * whether this deployment could sign a URL for these bytes, so the client
 * never has to guess from a header or a status code.
 */
export interface AttachmentDownloadTicket {
  mode: "presigned" | "stream";
  /** Present exactly when `mode` is `presigned`; carries its own signature. */
  url: string | null;
  expires_at: string | null;
  filename: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
}

function parseDownloadTicket(value: unknown): AttachmentDownloadTicket | null {
  if (value === null || typeof value !== "object") return null;
  const candidate = value as Partial<AttachmentDownloadTicket>;
  if (candidate.mode !== "presigned" && candidate.mode !== "stream") return null;
  return {
    mode: candidate.mode,
    url: typeof candidate.url === "string" ? candidate.url : null,
    expires_at: typeof candidate.expires_at === "string" ? candidate.expires_at : null,
    filename: typeof candidate.filename === "string" ? candidate.filename : "",
    content_type: candidate.content_type ?? "application/octet-stream",
    size_bytes: typeof candidate.size_bytes === "number" ? candidate.size_bytes : 0,
    sha256: candidate.sha256 ?? "",
  };
}

function filenameFromDisposition(response: Response): string | null {
  const disposition = response.headers.get("content-disposition") ?? "";
  const match = /filename\*?=(?:UTF-8'')?"?([^";]+)/i.exec(disposition);
  return match?.[1]?.trim() ?? null;
}

function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

/**
 * Asks the endpoint whether it can hand out a signed URL. A deployment that
 * cannot (local-fs, the self-host default) answers `mode: "stream"` — an
 * answer, not a failure — and a network error answers `null`, which the
 * caller treats the same way. Only an HTTP error propagates: that is the
 * server refusing the download itself.
 */
async function fetchDownloadTicket(
  url: string,
): Promise<AttachmentDownloadTicket | null> {
  try {
    const response = await fetch(`${url}?presign=true`, { headers: authHeaders() });
    if (!response.ok) throw new Error(`Download failed (${response.status})`);
    return parseDownloadTicket(await response.json().catch(() => null));
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("Download failed")) {
      throw error;
    }
    return null;
  }
}

/**
 * Fetches the signed URL straight from the store, so the API is not in the
 * byte path at all.
 *
 * Returns false when the store's host cannot be read from this page — a
 * bucket with no CORS rule for this origin, or an endpoint a browser cannot
 * resolve. The caller then streams, so a misconfigured bucket costs bandwidth
 * instead of the download.
 */
async function saveFromPresignedUrl(ticket: AttachmentDownloadTicket): Promise<boolean> {
  if (ticket.url === null) return false;
  try {
    const response = await fetch(ticket.url);
    if (!response.ok) return false;
    saveBlob(await response.blob(), ticket.filename);
    return true;
  } catch {
    return false;
  }
}

/**
 * One download path for both attachment doors: ask for a ticket, save from
 * the store when one arrives, stream when it does not. The stream is the
 * documented fallback, not an error state.
 */
async function downloadViaPath(url: string, fallbackFilename: string): Promise<void> {
  const ticket = await fetchDownloadTicket(url);
  if (ticket !== null && (await saveFromPresignedUrl(ticket))) return;
  const response = await fetch(url, { headers: authHeaders() });
  if (!response.ok) throw new Error(`Download failed (${response.status})`);
  saveBlob(await response.blob(), filenameFromDisposition(response) ?? fallbackFilename);
}

/**
 * Downloads a ticket attachment: saves it from the store when the deployment
 * can sign a URL, and streams it when it cannot (or when the browser cannot
 * read the store's host).
 */
export async function downloadAttachment(attachmentId: string, filename: string): Promise<void> {
  await downloadViaPath(
    `${apiBaseUrl()}${ATTACHMENT_DOWNLOAD_PATH.replace("{id}", attachmentId)}`,
    filename,
  );
}

export interface UploadedDocumentAttachment {
  id: string;
  document_id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
}

/**
 * Uploads one file against a document (same raw bypass as the ticket
 * surface: octet-stream body, `x-filename` + declared content-type).
 * Throws with the server's message on validation failures — size-cap
 * rejections surface verbatim.
 */
export async function uploadDocumentAttachment(
  documentId: string,
  file: File,
): Promise<UploadedDocumentAttachment> {
  const response = await fetch(
    `${apiBaseUrl()}${DOCUMENT_ATTACHMENTS_PATH.replace("{id}", documentId)}`,
    {
      method: "POST",
      headers: {
        ...authHeaders(),
        "content-type": file.type === "" ? "application/octet-stream" : file.type,
        "x-filename": file.name,
      },
      body: file,
    },
  );
  if (!response.ok) {
    const detail = (await response.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new Error(detail?.error?.message ?? `Upload failed (${response.status})`);
  }
  return (await response.json()) as UploadedDocumentAttachment;
}

/** Downloads a document attachment; honors content-disposition when it streams. */
export async function downloadDocumentAttachment(
  documentId: string,
  attachmentId: string,
  fallbackFilename: string,
): Promise<void> {
  await downloadViaPath(
    `${apiBaseUrl()}${DOCUMENT_ATTACHMENT_DOWNLOAD_PATH.replace("{id}", documentId).replace(
      "{attachment_id}",
      attachmentId,
    )}`,
    fallbackFilename,
  );
}

export interface StagedItemResponse {
  id: string;
  artifact_kind: string;
  source_kind: string;
}

/**
 * Uploads one plugin/skill zip to the staged sandbox import (raw binary
 * `application/zip` — same bypass as attachments: the generated client
 * models JSON bodies while this route is binary).
 */
export async function uploadPluginZip(
  file: File,
): Promise<{ ok: true; staged: StagedItemResponse } | { ok: false; status: number; message?: string }> {
  const response = await fetch(`${apiBaseUrl()}${PLUGIN_ZIP_STAGE_PATH}`, {
    method: "POST",
    headers: { ...authHeaders(), "content-type": "application/zip" },
    body: file,
  });
  const typed = response.headers.get("content-type")?.includes("application/json") ?? false;
  const detail = typed ? ((await response.json().catch(() => null)) as { error?: { message?: string; reason?: string } } | null) : null;
  if (!response.ok) {
    return {
      ok: false,
      status: response.status,
      message:
        detail?.error?.message ??
        `The archive was rejected during the sandbox import (${response.status}).`,
    };
  }
  return { ok: true, staged: (await response.json()) as StagedItemResponse };
}
