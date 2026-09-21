// /settings/plugins — staged-approvals queue (ADR 0009 staged pipeline).
//
// Purpose: pending member uploads (skills AND plugins share this queue)
// as PENDING rows: provenance chain summary, consent diff review (the ONE
// consent screen across source kinds — ConsentSummary), approve wiring
// (install created DISABLED + provenance; skill → version row) and
// refuse wiring (note required, terminal). Members see only their own
// stagings server-side; owner/admin see all.
//
// Non-purpose: no editing of staged content — the manifest is immutable
// at review time, exactly as uploaded.
"use client";

import type { components } from "@meridian/sdk";
import { Button, Card, EmptyState, Input, Spinner, Tag } from "@meridian/ui";
import { useState } from "react";
import { api } from "../../../lib/api";
import { reasonCopy } from "../../../lib/error-copy";
import { ConsentSummary } from "./consent";

type StagedItemView = components["schemas"]["StagedItemView"];
type ConsentView = components["schemas"]["ConsentView"];

const STATUS_TONE: Record<string, "success" | "danger" | "warning" | "neutral"> = {
  pending: "warning",
  approved: "success",
  refused: "danger",
};

function shortDigest(digest: string): string {
  return digest.length > 16 ? `${digest.slice(0, 10)}…` : digest;
}

function StagedRow({
  item,
  canManage,
  onChanged,
}: {
  item: StagedItemView;
  canManage: boolean;
  /** Authoritative reconcile after approve/refuse (parent refetch). */
  onChanged: () => void;
}) {
  const [consent, setConsent] = useState<ConsentView | null>(null);
  const [open, setOpen] = useState(false);
  const [grants, setGrants] = useState<string[] | null>(null);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [rowError, setRowError] = useState<string | null>(null);

  /** The uploader identity rides the staged row — truncated to a
   * reviewable prefix; the full chain lives on the adopted install. */
  const uploader = item.uploaded_by.length > 12 ? `${item.uploaded_by.slice(0, 12)}…` : item.uploaded_by;

  const loadConsent = async () => {
    if (consent !== null) {
      setOpen(!open);
      return;
    }
    setBusy(true);
    const { data, error } = await api.api.GET("/api/v1/plugins/distribution/staged/{id}/consent", {
      params: { path: { id: item.id } },
    });
    setBusy(false);
    if (error !== undefined || data === undefined) {
      const envelope = (error as { error?: { reason?: string } } | undefined)?.error;
      setRowError(reasonCopy(envelope?.reason ?? "not_found", envelope));
      return;
    }
    setConsent(data);
    setGrants(item.declared_capabilities.slice());
    setOpen(true);
  };

  const decide = async (decision: "approve" | "refuse") => {
    setBusy(true);
    setRowError(null);
    const body =
      decision === "approve"
        ? { granted_capabilities: grants ?? item.declared_capabilities, review_note: note.trim() === "" ? null : note.trim() }
        : { review_note: note.trim() };
    const { error } =
      decision === "approve"
        ? await api.api.POST("/api/v1/plugins/distribution/staged/{id}/approve", {
            params: { path: { id: item.id } },
            body,
          })
        : await api.api.POST("/api/v1/plugins/distribution/staged/{id}/refuse", {
            params: { path: { id: item.id } },
            body: { review_note: note.trim() },
          });
    setBusy(false);
    if (error !== undefined) {
      const envelope = (error as { error?: { reason?: string } } | undefined)?.error;
      setRowError(reasonCopy(envelope?.reason ?? "validation_failed", envelope));
      return;
    }
    // Authoritative reconcile: the decision is terminal; the list refresh
    // pulls the adopted install/skill id + final status.
    onChanged();
  };

  return (
    <div
      className="rounded-[var(--m-radius-container)] border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid={`staged-row-${item.id}`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Tag tone={STATUS_TONE[item.status] ?? "neutral"}>{item.status}</Tag>
        <span className="text-[13px] font-semibold">{item.artifact_kind}</span>
        <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {item.source_kind} · {item.source_url}
          {item.fetched_ref !== null ? ` · ${item.fetched_ref}` : ""}
        </span>
        <span className="font-mono text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          {shortDigest(item.artifact_digest)}
        </span>
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          up by {uploader}
        </span>
        <div className="ml-auto flex gap-2">
          <Button size="sm" variant="ghost" onClick={() => void loadConsent()} aria-expanded={open}>
            {busy ? <Spinner /> : null}
            {open ? "Hide review" : "Review consent"}
          </Button>
        </div>
      </div>
      {rowError !== null ? (
        <p role="alert" className="mt-2 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {rowError}
        </p>
      ) : null}
      {open && consent !== null ? (
        <div className="mt-3 grid gap-3">
          <ConsentSummary
            artifact={{ ...consent, uploaded_by: item.uploaded_by }}
            settingsSchema={consent.settings_schema}
            connectorProfile={consent.connector_profile}
            grants={grants}
            onToggleGrant={
              canManage
                ? (capability) =>
                    setGrants(
                      (g) =>
                        g === null
                          ? [capability]
                          : g.includes(capability)
                            ? g.filter((c) => c !== capability)
                            : [...g, capability],
                    )
                : undefined
            }
          />
          {item.status === "pending" && canManage ? (
            <>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="primary"
                  disabled={busy || (grants ?? []).length === 0}
                  onClick={() => void decide("approve")}
                >
                  {busy ? <Spinner /> : null} Approve — installs disabled
                </Button>
                <Button size="sm" variant="destructive" onClick={() => void decide("refuse")} disabled={busy || note.trim() === ""}>
                  Refuse
                </Button>
                <div className="min-w-56 flex-1">
                  <Input
                    placeholder="Refusal note (required)"
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                    aria-label="Refusal note"
                    data-testid="refuse-note"
                  />
                </div>
              </div>
              <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                Approve creates the install DISABLED with the provenance chain appended; enabling stays a
                separate explicit step. Refusing is terminal — the staged row can never be re-reviewed.
              </p>
            </>
          ) : item.status === "pending" && !canManage ? (
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Waiting on an owner/admin review — you'll see the result on this row.
            </p>
          ) : item.review_note !== null && item.review_note !== undefined ? (
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Review note: {item.review_note}
            </p>
          ) : null}
        </div>
      ) : null}
      {item.status !== "pending" && item.review_note !== null && item.review_note !== undefined ? (
        <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          Note: {item.review_note}
        </p>
      ) : null}
    </div>
  );
}

export function StagedApprovalsTab({
  staged,
  canManage,
  onChanged,
}: {
  staged: StagedItemView[] | null;
  canManage: boolean;
  onChanged: () => void;
}) {
  const pending = (staged ?? []).filter((s) => s.status === "pending");
  const decided = (staged ?? []).filter((s) => s.status !== "pending");
  return (
    <div className="grid gap-3">
      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        Member uploads (zips, GitHub pulls, registry links, skills) land here PENDING —
        validated and never executed until an owner/admin consents.
      </p>
      {staged === null ? (
        <Spinner />
      ) : (
        <>
          <Card>
            <h3 className="p-4 pb-2 text-sm font-semibold">Pending ({pending.length})</h3>
            <div className="grid gap-3 px-4 pb-4">
              {pending.length === 0 ? (
                <EmptyState message="No staged imports awaiting review. Members' uploads appear here for owner/admin consent before anything executes." />
              ) : (
                pending.map((item) => (
                  <StagedRow key={item.id} item={item} canManage={canManage} onChanged={onChanged} />
                ))
              )}
            </div>
          </Card>
          {decided.length > 0 ? (
            <Card>
              <h3 className="p-4 pb-2 text-sm font-semibold">Reviewed</h3>
              <div className="grid gap-3 px-4 pb-4">
                {decided.map((item) => (
                  <StagedRow key={item.id} item={item} canManage={canManage} onChanged={onChanged} />
                ))}
              </div>
            </Card>
          ) : null}
        </>
      )}
    </div>
  );
}
