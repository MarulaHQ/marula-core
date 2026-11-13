// /settings/object-integrity — what the object-integrity pass found (Wave 67
// lane `w67-read-surfaces`).
//
// Purpose: the worker's object-GC pass classifies every finding
// `orphan_deleted` or `bytes_missing` and writes it to
// `object_integrity_findings`, and until now nothing read the rows back an
// operator could see — the pass's own query selected `storage_key` for
// de-duplication and that was the whole audience. This page is the reader: the
// ledger, newest first, with the two open counts above it.
//
// # Why the two directions are one list, not two pages
//
// They answer one question with two clauses: "do the rows and the bytes still
// agree". An orphan is bytes nothing references; a missing key is a row whose
// bytes are gone. An operator triaging a restore wants both on one screen,
// because the same restore produces one of each.
//
// # Why there is no fix button
//
// The pass is the writer, and repairing a finding is an upload, not a click:
// `attachments.missing_since` clears when the object is stored again under the
// same content address, and `resolved_at` follows it. A button here would have
// to fake one of those two facts.
//
// The payload is fetched with an authenticated request instead of the typed
// SDK (the quarantine page's posture): the OpenAPI pins for this route are
// regenerated in the lane's gated verification sequence. The interfaces below
// mirror crates/core-api/src/routes/object_integrity_admin.rs.
"use client";

import {
  Badge,
  Button,
  Card,
  EmptyState,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
  formatInTimeZone,
} from "@marula-core/ui";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  NotAvailable,
  SettingsHeader,
} from "../../../components/settings/settings-shell";
import { apiBaseUrl, authHeaders, useRequireAuth } from "../../../lib/api";
import { useTimeFormat } from "../../../lib/use-time-format";

type FindingKind = "orphan_deleted" | "bytes_missing";

interface IntegrityFinding {
  id: string;
  pass_id: string;
  found_at: string;
  kind: FindingKind;
  storage_key: string;
  size_bytes: number | null;
  source_table: string | null;
  resolved_at: string | null;
  note: string | null;
}

interface IntegrityPageView {
  items: IntegrityFinding[];
  next_offset?: number;
  open_orphans: number;
  open_missing: number;
}

const BASE = "/api/v1/admin/object-integrity/findings";

function formatBytes(bytes: number | null): string {
  if (bytes === null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

/** One line per direction: what the token means to a person. */
const KIND_COPY: Record<FindingKind, { label: string; blurb: string; tone: "warning" | "danger" }> = {
  orphan_deleted: {
    label: "orphan deleted",
    blurb: "bytes nothing referenced; the pass reclaimed them",
    tone: "warning",
  },
  bytes_missing: {
    label: "bytes missing",
    blurb: "a row claims bytes the store does not have",
    tone: "danger",
  },
};

export default function ObjectIntegritySettingsPage() {
  const auth = useRequireAuth();
  const { timeZone } = useTimeFormat();
  const canRead = (auth.me?.permissions ?? []).includes("admin:view");

  const [kind, setKind] = useState<"" | FindingKind>("");
  const [includeResolved, setIncludeResolved] = useState(false);
  const [page, setPage] = useState<IntegrityPageView | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const query = new URLSearchParams({ limit: "200" });
      if (kind !== "") query.set("kind", kind);
      if (includeResolved) query.set("include_resolved", "true");
      const res = await fetch(`${apiBaseUrl()}${BASE}?${query.toString()}`, {
        headers: authHeaders(),
        cache: "no-store",
      });
      if (res.status === 403) {
        setLoadError("forbidden");
        return;
      }
      if (!res.ok) {
        setLoadError(`The findings could not be loaded (${res.status}).`);
        return;
      }
      const body = (await res.json()) as IntegrityPageView;
      setPage({
        items: Array.isArray(body.items) ? body.items : [],
        next_offset: body.next_offset,
        open_orphans: typeof body.open_orphans === "number" ? body.open_orphans : 0,
        open_missing: typeof body.open_missing === "number" ? body.open_missing : 0,
      });
    } catch {
      setLoadError("The findings could not be loaded — check the connection and try again.");
    } finally {
      setLoading(false);
    }
  }, [kind, includeResolved]);

  useEffect(() => {
    if (auth.status !== "authed" || !canRead) return;
    void load();
  }, [auth.status, canRead, load]);

  const items = useMemo(() => page?.items ?? [], [page]);
  const openTotal = (page?.open_orphans ?? 0) + (page?.open_missing ?? 0);

  if (auth.status !== "authed") return null;

  if (!canRead) {
    return (
      <div className="grid max-w-5xl gap-6">
        <SettingsHeader
          title="Object integrity"
          description="What the object-integrity pass found between the database and the object store."
        />
        <NotAvailable
          what="Object integrity"
          hint="this page is limited to roles that can read the admin module"
        />
      </div>
    );
  }

  return (
    <div className="grid max-w-5xl gap-6">
      <SettingsHeader
        title="Object integrity"
        description="What the periodic object-integrity pass found. An orphan is bytes no row references, reclaimed after the grace window; a missing key is a row whose bytes the store no longer has."
      />

      <div className="grid gap-3 sm:grid-cols-3">
        <Card>
          <div className="grid gap-0.5 p-4">
            <span className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Open findings
            </span>
            <span
              className="text-[22px] font-semibold tabular-nums leading-tight"
              style={{ color: "var(--m-text)" }}
              data-testid="open-total"
            >
              {openTotal}
            </span>
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              {openTotal === 0 ? "rows and bytes agree" : "still to be explained or repaired"}
            </span>
          </div>
        </Card>
        <Card>
          <div className="grid gap-0.5 p-4">
            <span className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Orphans reclaimed
            </span>
            <span
              className="text-[22px] font-semibold tabular-nums leading-tight"
              style={{ color: "var(--m-text)" }}
              data-testid="open-orphans"
            >
              {page?.open_orphans ?? 0}
            </span>
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              open rows of bytes deleted with no reference
            </span>
          </div>
        </Card>
        <Card>
          <div className="grid gap-0.5 p-4">
            <span className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Missing bytes
            </span>
            <span
              className="text-[22px] font-semibold tabular-nums leading-tight"
              style={{ color: "var(--m-text)" }}
              data-testid="open-missing"
            >
              {page?.open_missing ?? 0}
            </span>
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              downloads that fail until the bytes return
            </span>
          </div>
        </Card>
      </div>

      {loadError === "forbidden" ? (
        <NotAvailable what="Object integrity" hint="your role cannot read the admin module" />
      ) : loadError !== null ? (
        <div
          role="alert"
          data-testid="integrity-load-error"
          className="flex flex-wrap items-center gap-3 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
        >
          <span className="flex-1">{loadError}</span>
          <Button variant="secondary" size="sm" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <select
          className="m-input"
          aria-label="Restrict to one finding direction"
          value={kind}
          onChange={(e) => setKind(e.target.value as "" | FindingKind)}
        >
          <option value="">Both directions</option>
          <option value="orphan_deleted">Orphan deleted</option>
          <option value="bytes_missing">Bytes missing</option>
        </select>
        <label className="inline-flex items-center gap-1.5 text-[12px]">
          <input
            type="checkbox"
            checked={includeResolved}
            onChange={(e) => setIncludeResolved(e.target.checked)}
          />
          Include findings a later pass resolved
        </label>
        <span className="flex-1" />
        <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
          Refresh
        </Button>
      </div>

      <Card className="overflow-x-auto">
        {loading && page === null ? (
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        ) : items.length === 0 ? (
          <div className="p-4">
            <EmptyState
              message={
                includeResolved
                  ? "The pass has recorded no findings yet."
                  : "Nothing is open. Every finding the pass recorded has been resolved, or it has not run yet."
              }
            />
          </div>
        ) : (
          <Table data-testid="integrity-table">
            <thead>
              <tr>
                <Th>Finding</Th>
                <Th>Key</Th>
                <Th>Claimed by</Th>
                <Th className="tabular-nums">Size</Th>
                <Th className="tabular-nums">Found</Th>
                <Th>State</Th>
              </tr>
            </thead>
            <tbody>
              {items.map((item) => {
                const copy = KIND_COPY[item.kind];
                return (
                  <Tr key={item.id} data-testid={`integrity-row-${item.id}`}>
                    <Td>
                      <div className="grid gap-1">
                        <Badge tone={copy.tone}>{copy.label}</Badge>
                        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          {copy.blurb}
                        </span>
                      </div>
                    </Td>
                    <Td>
                      <div className="grid gap-0.5">
                        <code
                          className="break-all text-[11px]"
                          style={{ color: "var(--m-text)" }}
                        >
                          {item.storage_key}
                        </code>
                        <span
                          className="font-mono text-[10px]"
                          style={{ color: "var(--m-text-muted)" }}
                        >
                          pass {item.pass_id.slice(0, 8)}
                        </span>
                      </div>
                    </Td>
                    <Td>
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {item.source_table ?? "—"}
                      </span>
                    </Td>
                    <Td className="tabular-nums">
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {formatBytes(item.size_bytes)}
                      </span>
                    </Td>
                    <Td className="tabular-nums">
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {formatInTimeZone(item.found_at, timeZone, "datetime")}
                      </span>
                    </Td>
                    <Td>
                      {item.resolved_at !== null ? (
                        <div className="grid gap-0.5">
                          <Badge tone="success">resolved</Badge>
                          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                            {formatInTimeZone(item.resolved_at, timeZone, "date")}
                          </span>
                        </div>
                      ) : (
                        <Badge tone="neutral">open</Badge>
                      )}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
        <div
          className="flex min-h-13 flex-wrap items-center gap-3 px-3 pb-2 text-[13px]"
          style={{ color: "var(--m-text-secondary)" }}
        >
          <span data-testid="integrity-counts">
            <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
              {items.length}
            </b>{" "}
            finding{items.length === 1 ? "" : "s"} shown
          </span>
          {page?.next_offset !== undefined ? (
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Showing the newest 200; older findings are not on this page yet.
            </span>
          ) : null}
          <span className="flex-1" />
          <Link
            href="/settings/self-host"
            className="text-xs underline underline-offset-2"
            style={{ color: "var(--m-text-muted)" }}
          >
            The pass runs from the worker; its schedule is on the ops page
          </Link>
        </div>
      </Card>

      <p className="text-[12px] leading-[1.5]" style={{ color: "var(--m-text-muted)" }}>
        A finding is not a button. Missing bytes clear when the object is stored again
        under the same content address, which also sets <code>missing_since</code> back to
        null on the rows that claim it; an orphan is already deleted, and its finding stays
        as the record of what the pass reclaimed.
      </p>
    </div>
  );
}
