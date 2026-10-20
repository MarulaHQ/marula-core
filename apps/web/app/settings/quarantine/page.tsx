// /settings/quarantine — the quarantine admin surface (Wave 52 lane P6c).
//
// Purpose: show what the malware scanner refused, in the state it is in now
// (held, or released anyway), and give an org admin the two decisions they
// can make about it: release the object despite the verdict, or delete the
// held bytes. Every row carries the scanner's own words, the file's identity
// (digest, type, size), where the upload was aimed, and — once released —
// who overrode it, when, and why.
//
// Release is the product's most consequential override, so the surface says
// so plainly, in the header AND beside the action, and it refuses to submit
// without a reason. The API audits the release with the actor, the reason and
// the timestamp; this page never invents a default reason on the operator's
// behalf.
//
// Delete removes the bytes and the row. The record that malware was uploaded
// survives as the `attachment_scan.deleted` audit event, which is why the
// confirm dialog can say "the trail stays" without lying.
//
// Non-purpose: no upload-path change, no re-scan, no bulk actions, no
// retention policy. A released object is not deletable here because its bytes
// are admitted and its trail is evidence.
//
// The payload is fetched with an authenticated request instead of the typed
// SDK: these routes ship with this lane, and the OpenAPI pin/SDK regeneration
// runs in the lane's gated verification sequence
// (scripts/regen-openapi-pins.sh). The interfaces below mirror the server DTOs
// in crates/core-api/src/routes/quarantine_admin.rs.
"use client";

import {
  Badge,
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
  FieldHint,
  Label,
  Spinner,
  Table,
  Td,
  Textarea,
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

type QuarantineState = "pending" | "released";

interface QuarantineItem {
  sha256: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  scanner: string;
  detail: string | null;
  state: QuarantineState;
  quarantined_at: string;
  scanned_at: string;
  ticket_id: string | null;
  uploaded_by_user_id: string | null;
  uploaded_by_portal_user_id: string | null;
  storage_key: string;
  released_at: string | null;
  released_by_user_id: string | null;
  release_reason: string | null;
}

interface QuarantinePageView {
  items: QuarantineItem[];
  pending: number;
  released: number;
  next_offset?: number;
}

const BASE = "/api/v1/admin/quarantine";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function shortId(id: string | null): string {
  if (id === null) return "—";
  return id.slice(0, 8);
}

/** The uploader is one of two actor families; say which rather than a bare uuid. */
function uploaderLabel(item: QuarantineItem): string {
  if (item.uploaded_by_user_id !== null) return `staff ${shortId(item.uploaded_by_user_id)}`;
  if (item.uploaded_by_portal_user_id !== null) {
    return `portal ${shortId(item.uploaded_by_portal_user_id)}`;
  }
  return "—";
}

export default function QuarantineSettingsPage() {
  const auth = useRequireAuth();
  const { timeZone } = useTimeFormat();
  const canRead = (auth.me?.permissions ?? []).includes("admin:view");
  const canRelease = (auth.me?.permissions ?? []).includes("admin:edit");
  const canDelete = (auth.me?.permissions ?? []).includes("admin:delete");

  const [page, setPage] = useState<QuarantinePageView | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyDigest, setBusyDigest] = useState<string | null>(null);

  const [inspecting, setInspecting] = useState<QuarantineItem | null>(null);
  const [releasing, setReleasing] = useState<QuarantineItem | null>(null);
  const [reason, setReason] = useState("");
  const [reasonTouched, setReasonTouched] = useState(false);
  const [deleting, setDeleting] = useState<QuarantineItem | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const res = await fetch(`${apiBaseUrl()}${BASE}?limit=200`, {
        headers: authHeaders(),
        cache: "no-store",
      });
      if (res.status === 403) {
        setLoadError("forbidden");
        return;
      }
      if (!res.ok) {
        setLoadError(`The quarantine list could not be loaded (${res.status}).`);
        return;
      }
      const body = (await res.json()) as QuarantinePageView;
      setPage({
        items: Array.isArray(body.items) ? body.items : [],
        pending: typeof body.pending === "number" ? body.pending : 0,
        released: typeof body.released === "number" ? body.released : 0,
        next_offset: body.next_offset,
      });
    } catch {
      setLoadError("The quarantine list could not be loaded — check the connection and try again.");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed" || !canRead) return;
    void load();
  }, [auth.status, canRead, load]);

  const items = useMemo(() => page?.items ?? [], [page]);
  const reasonOk = reason.trim().length > 0;
  const showReasonError = reasonTouched && !reasonOk;

  const closeRelease = () => {
    if (busyDigest !== null) return;
    setReleasing(null);
    setReason("");
    setReasonTouched(false);
    setActionError(null);
  };

  const release = async () => {
    if (releasing === null || !reasonOk) {
      setReasonTouched(true);
      return;
    }
    setBusyDigest(releasing.sha256);
    setActionError(null);
    try {
      const res = await fetch(
        `${apiBaseUrl()}${BASE}/${releasing.sha256}/release`,
        {
          method: "POST",
          headers: { ...authHeaders(), "content-type": "application/json" },
          body: JSON.stringify({ reason: reason.trim() }),
        },
      );
      if (!res.ok) {
        setActionError(
          `The release was not recorded (${res.status}). Nothing moved; try again.`,
        );
        return;
      }
      setReleasing(null);
      setReason("");
      setReasonTouched(false);
      await load();
    } catch {
      setActionError("The release could not be sent — check the connection and try again.");
    } finally {
      setBusyDigest(null);
    }
  };

  const remove = async () => {
    if (deleting === null) return;
    setBusyDigest(deleting.sha256);
    setActionError(null);
    try {
      const res = await fetch(`${apiBaseUrl()}${BASE}/${deleting.sha256}`, {
        method: "DELETE",
        headers: authHeaders(),
      });
      if (!res.ok && res.status !== 404) {
        setActionError(`The held file was not deleted (${res.status}). Try again.`);
        return;
      }
      setDeleting(null);
      await load();
    } catch {
      setActionError("The delete could not be sent — check the connection and try again.");
    } finally {
      setBusyDigest(null);
    }
  };

  if (auth.status !== "authed") return null;

  if (!canRead) {
    return (
      <div className="grid max-w-4xl gap-6">
        <SettingsHeader
          title="Quarantined files"
          description="Files the malware scanner rejected, and the two decisions an org admin can make about them."
        />
        <NotAvailable
          what="Quarantined files"
          hint="this page is limited to roles that can read the admin module"
        />
      </div>
    );
  }

  return (
    <div className="grid max-w-4xl gap-6">
      <SettingsHeader
        title="Quarantined files"
        description="Uploads the malware scanner rejected. Their bytes are held outside the attachment store: no ticket, note or document can reach them."
      />

      {/*
        The override statement is not a tooltip and not fine print. An admin
        who releases is admitting a file the scanner called dangerous into the
        same store every other download serves from.
      */}
      <div
        role="note"
        data-testid="release-override-note"
        className="rounded-[var(--m-radius-container)] border px-4 py-3 text-[13px] leading-[1.5]"
        style={{
          borderColor: "var(--m-status-due-fg)",
          background: "var(--m-status-due-bg)",
          color: "var(--m-status-due-fg)",
        }}
      >
        <strong className="font-semibold">Release is an override.</strong> It admits a
        file the scanner rejected into the normal attachment store. You need a reason,
        and both the reason and your name are written to the audit trail. Delete removes
        the held bytes; the record that a file was rejected stays in the trail.
      </div>

      {loadError === "forbidden" ? (
        <NotAvailable what="Quarantined files" hint="your role cannot read the admin module" />
      ) : loadError !== null ? (
        <div
          role="alert"
          data-testid="quarantine-load-error"
          className="flex flex-wrap items-center gap-3 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        >
          <span className="flex-1">{loadError}</span>
          <Button variant="secondary" size="sm" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : null}

      {actionError !== null && (
        <div
          role="alert"
          data-testid="quarantine-action-error"
          className="rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        >
          {actionError}
        </div>
      )}

      <Card className="overflow-x-auto">
        {loading && page === null ? (
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        ) : items.length === 0 ? (
          <div className="p-4">
            <EmptyState
              message={
                loadError !== null
                  ? "Nothing to show until the list loads."
                  : "Nothing is quarantined. Every upload the scanner has seen was admitted, skipped, or never arrived."
              }
            />
          </div>
        ) : (
          <Table data-testid="quarantine-table">
            <thead>
              <tr>
                <Th>File</Th>
                <Th>Verdict</Th>
                <Th className="tabular-nums">Size</Th>
                <Th className="tabular-nums">Quarantined</Th>
                <Th>Ticket</Th>
                <Th>State</Th>
                <Th>
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody>
              {items.map((item) => {
                const busy = busyDigest === item.sha256;
                return (
                  <Tr key={item.sha256} data-testid={`quarantine-row-${item.sha256}`}>
                    <Td>
                      <div className="grid gap-0.5">
                        <span
                          className="truncate text-[13px] font-medium"
                          style={{ color: "var(--m-text)" }}
                        >
                          {item.filename}
                        </span>
                        <span
                          className="font-mono text-[11px] tabular-nums"
                          style={{ color: "var(--m-text-muted)" }}
                        >
                          {item.content_type} · {shortId(item.sha256)}
                        </span>
                      </div>
                    </Td>
                    <Td>
                      <div className="grid gap-0.5">
                        <span className="text-xs" style={{ color: "var(--m-text)" }}>
                          {item.detail ?? "no signature detail"}
                        </span>
                        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          by {item.scanner}
                        </span>
                      </div>
                    </Td>
                    <Td className="tabular-nums">
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {formatBytes(item.size_bytes)}
                      </span>
                    </Td>
                    <Td className="tabular-nums">
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {formatInTimeZone(item.quarantined_at, timeZone, "datetime")}
                      </span>
                    </Td>
                    <Td>
                      {item.ticket_id !== null ? (
                        <Link
                          href={`/tickets/${item.ticket_id}`}
                          className="text-xs underline underline-offset-2"
                          style={{ color: "var(--m-text-secondary)" }}
                        >
                          open
                        </Link>
                      ) : (
                        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                          —
                        </span>
                      )}
                    </Td>
                    <Td>
                      <div className="grid gap-1">
                        <Badge tone={item.state === "released" ? "warning" : "danger"}>
                          {item.state === "released" ? "released" : "held"}
                        </Badge>
                        {item.state === "released" && item.released_at !== null ? (
                          <span
                            className="text-[11px]"
                            style={{ color: "var(--m-text-muted)" }}
                          >
                            {formatInTimeZone(item.released_at, timeZone, "date")}
                          </span>
                        ) : null}
                      </div>
                    </Td>
                    <Td>
                      <div className="flex flex-wrap justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setInspecting(item)}
                          data-testid={`inspect-${item.sha256}`}
                        >
                          Inspect
                        </Button>
                        {item.state === "pending" && canRelease ? (
                          <Button
                            variant="secondary"
                            size="sm"
                            disabled={busy}
                            onClick={() => {
                              setReleasing(item);
                              setReason("");
                              setReasonTouched(false);
                              setActionError(null);
                            }}
                            data-testid={`release-${item.sha256}`}
                          >
                            Release
                          </Button>
                        ) : null}
                        {item.state === "pending" && canDelete ? (
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={busy}
                            onClick={() => {
                              setDeleting(item);
                              setActionError(null);
                            }}
                            data-testid={`delete-${item.sha256}`}
                          >
                            Delete
                          </Button>
                        ) : null}
                      </div>
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
          <span data-testid="quarantine-counts">
            <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
              {page?.pending ?? 0}
            </b>{" "}
            held ·{" "}
            <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
              {page?.released ?? 0}
            </b>{" "}
            released
          </span>
          {page?.next_offset !== undefined ? (
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Showing the newest 200; older held objects are not on this page yet.
            </span>
          ) : null}
          <span className="flex-1" />
          <Link
            href="/audit"
            className="text-xs underline underline-offset-2"
            style={{ color: "var(--m-text-muted)" }}
          >
            Every release and delete is in the audit trail
          </Link>
        </div>
      </Card>

      {/* --- inspect ------------------------------------------------------ */}
      <Dialog open={inspecting !== null} onOpenChange={(open) => !open && setInspecting(null)}>
        <DialogContent
          title={inspecting?.filename ?? "Quarantined file"}
          description="Everything the record holds about this object."
        >
          {inspecting !== null ? (
            <dl
              data-testid="quarantine-inspect"
              className="grid gap-x-4 gap-y-2 text-xs sm:grid-cols-[9rem_1fr]"
            >
              <dt style={{ color: "var(--m-text-muted)" }}>Digest</dt>
              <dd className="font-mono break-all" style={{ color: "var(--m-text)" }}>
                {inspecting.sha256}
              </dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Type</dt>
              <dd style={{ color: "var(--m-text)" }}>
                {inspecting.content_type} · {formatBytes(inspecting.size_bytes)}
              </dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Verdict</dt>
              <dd style={{ color: "var(--m-text)" }}>
                {inspecting.detail ?? "no signature detail"} ({inspecting.scanner})
              </dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Quarantined</dt>
              <dd style={{ color: "var(--m-text)" }}>
                {formatInTimeZone(inspecting.quarantined_at, timeZone, "datetime")}
              </dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Uploader</dt>
              <dd style={{ color: "var(--m-text)" }}>{uploaderLabel(inspecting)}</dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Ticket</dt>
              <dd style={{ color: "var(--m-text)" }}>
                {inspecting.ticket_id !== null ? (
                  <Link
                    href={`/tickets/${inspecting.ticket_id}`}
                    className="underline underline-offset-2"
                  >
                    {inspecting.ticket_id}
                  </Link>
                ) : (
                  "not uploaded against a ticket"
                )}
              </dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Bytes</dt>
              <dd className="font-mono break-all" style={{ color: "var(--m-text)" }}>
                {inspecting.storage_key}
              </dd>
              {inspecting.state === "released" ? (
                <>
                  <dt style={{ color: "var(--m-text-muted)" }}>Released</dt>
                  <dd style={{ color: "var(--m-text)" }}>
                    {inspecting.released_at !== null
                      ? formatInTimeZone(inspecting.released_at, timeZone, "datetime")
                      : "—"}{" "}
                    by {shortId(inspecting.released_by_user_id)}
                  </dd>
                  <dt style={{ color: "var(--m-text-muted)" }}>Reason</dt>
                  <dd style={{ color: "var(--m-text)" }}>
                    {inspecting.release_reason ?? "—"}
                  </dd>
                </>
              ) : null}
            </dl>
          ) : null}
          <DialogActions>
            <Button variant="secondary" onClick={() => setInspecting(null)}>
              Close
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* --- release ------------------------------------------------------ */}
      <Dialog open={releasing !== null} onOpenChange={(open) => !open && closeRelease()}>
        <DialogContent
          title="Release this file?"
          description="Release is an override: the bytes move into the normal attachment store even though the scanner named a signature. The reason is required and is recorded with your name."
        >
          {releasing !== null ? (
            <div className="grid gap-2">
              <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                {releasing.filename} · {releasing.detail ?? "no signature detail"}
              </span>
              <Label htmlFor="release-reason">Why is this file safe to admit?</Label>
              <Textarea
                id="release-reason"
                data-testid="release-reason"
                value={reason}
                maxLength={500}
                rows={3}
                aria-describedby="release-reason-hint"
                aria-invalid={showReasonError}
                onBlur={() => setReasonTouched(true)}
                onChange={(e) => setReason(e.target.value)}
                placeholder="e.g. Vendor confirmed a false positive on invoice 8821"
              />
              {showReasonError ? (
                <FieldError id="release-reason-error">
                  A reason is required to release a quarantined file.
                </FieldError>
              ) : (
                <FieldHint id="release-reason-hint">
                  Written to the audit trail with your name and the time.
                </FieldHint>
              )}
            </div>
          ) : null}
          <DialogActions>
            <Button variant="secondary" onClick={closeRelease} disabled={busyDigest !== null}>
              Cancel
            </Button>
            <Button
              variant="primary"
              data-testid="confirm-release"
              disabled={!reasonOk || busyDigest !== null}
              onClick={() => void release()}
            >
              {busyDigest !== null ? "Releasing…" : "Release file"}
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* --- delete ------------------------------------------------------- */}
      <Dialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open && busyDigest === null) setDeleting(null);
        }}
      >
        <DialogContent
          title="Delete the held bytes?"
          description="The quarantined copy is removed. The audit trail keeps the record that this file was rejected, with the filename and the signature."
        >
          <p className="text-[13px]" style={{ color: "var(--m-text)" }}>
            {deleting?.filename ?? "This file"}
          </p>
          <DialogActions>
            <Button
              variant="secondary"
              onClick={() => setDeleting(null)}
              disabled={busyDigest !== null}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              data-testid="confirm-delete"
              disabled={busyDigest !== null}
              onClick={() => void remove()}
            >
              {busyDigest !== null ? "Deleting…" : "Delete held bytes"}
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>
    </div>
  );
}
