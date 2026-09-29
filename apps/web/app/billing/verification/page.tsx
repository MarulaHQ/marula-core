// /billing/verification — the pre-invoice review surface (Wave 39 lane 2,
// migration 0111).
//
// Purpose: the one screen where a billing period is verified before it
// runs. A reviewer picks a contract and a window, reads every entry and
// its cost GROUPED BY TICKET, and works three things in context: the
// disposition (approve / reject / flip to non-billable), the minutes
// (adjust down with a reason), and the category (recategorize onto the
// org taxonomy). Each row shows the badge that decides whether it will
// bill at all: approved, draft, rejected, non-billable, unclassified,
// already invoiced, or adjusted.
//
// The header band carries the counters that gate the close ritual
// (unclassified / undispositioned / draft / non-billable / invoiced),
// the running total, and — beside it, not below — the block-consumption
// panel, because "how much of the retainer is left" is a question an
// admin asks WHILE reading the entries, not after.
//
// Closing the period is a ritual with teeth: the sign-off dialog states
// the three checklist items the server will re-verify (entries
// dispositioned, zero unclassified, preview seen), and warns when an
// issued run already overlaps the window. A sign-off is a record, not a
// soft suggestion: the server refuses a claim that doesn't match the
// live workspace.
//
// Non-purpose: no contract/rate editing (the contract editor owns it),
// no invoice mutation (the run composer owns it), no card payment
// surface. Locked (invoiced) rows render read-only with the reason on
// the row — the lock is visible, never silently inert.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Tag,
  type StatusTone,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { friendlyMutationError, rejectionText } from "../../../lib/error-copy";
import { fmtMins, money } from "../../../lib/format";
import { useTimeCategories } from "../../../lib/use-time-categories";

type Contract = components["schemas"]["Contract"];
type Workspace = components["schemas"]["VerificationWorkspace"];
type EntryView = components["schemas"]["VerificationEntryView"];
type BlockPanel = components["schemas"]["BlockPanelView"];
type SignoffResponse = components["schemas"]["SignoffResponse"];

const ACTIONS = [
  "approve",
  "reject",
  "flip_billable",
  "adjust_minutes",
  "recategorize",
  "add_note",
] as const;
type Action = (typeof ACTIONS)[number];

const REJECTION_CODES = [
  "missing_notes",
  "wrong_ticket",
  "incorrect_duration",
  "duplicate_entry",
  "non_billable_work",
  "policy_violation",
  "other",
] as const;

const UNBILLABLE_REASONS = [
  { code: "warranty", label: "Covered by warranty" },
  { code: "courtesy", label: "Courtesy / goodwill" },
  { code: "in_retainer_block", label: "Inside a retainer block" },
  { code: "sla_covered", label: "Covered by the SLA" },
  { code: "internal", label: "Internal work" },
  { code: "mistake", label: "Mistake — do not bill" },
] as const;

function isoDay(offsetDays: number): string {
  const d = new Date();
  d.setUTCHours(0, 0, 0, 0);
  d.setUTCDate(d.getUTCDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}

function firstOfMonth(offsetMonths = 0): string {
  const d = new Date();
  d.setUTCHours(0, 0, 0, 0);
  d.setUTCDate(1);
  d.setUTCMonth(d.getUTCMonth() + offsetMonths);
  return d.toISOString().slice(0, 10);
}

function when(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** The badge is the row's billing fate, so it reads first and reads
 * loudest when it needs attention: unclassified and draft entries are
 * what block a clean close, so they carry the warning tone. */
function badgeTone(entry: EntryView): { label: string; tone: StatusTone } {
  if (entry.invoiced) return { label: "Invoiced", tone: "neutral" };
  if (entry.disposition === "rejected") return { label: "Rejected", tone: "danger" };
  if (entry.disposition === "non_billable") return { label: "Non-billable", tone: "neutral" };
  if (entry.approval_status === "draft") return { label: "Draft", tone: "warning" };
  if (entry.approval_status === "submitted")
    return { label: "Awaiting approval", tone: "warning" };
  if (entry.disposition === "approved") return { label: "Approved", tone: "success" };
  if (entry.work_type === null || entry.work_type === "")
    return { label: "Unclassified", tone: "warning" };
  return { label: "Ready to bill", tone: "success" };
}

function blockTone(alert: string): StatusTone {
  if (alert === "depleted") return "danger";
  if (alert === "expiring") return "warning";
  return "success";
}

function BlockPanelCard({
  blocks,
  currency,
}: {
  blocks: BlockPanel[];
  currency: string;
}) {
  if (blocks.length === 0) return null;
  return (
    <Card className="m-rise p-3" data-testid="verify-block-panel">
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
          Block consumption
        </h2>
        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          this window
        </span>
      </div>
      <div className="mt-2 flex flex-col gap-3">
        {blocks.map((block) => {
          const used = block.drawn_minutes + block.prior_drawn_minutes;
          const pct =
            block.total_minutes > 0
              ? Math.min(100, Math.round((used / block.total_minutes) * 100))
              : 0;
          return (
            <div key={block.block_id} data-testid={`verify-block-${block.block_id}`}>
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <span className="text-xs" style={{ color: "var(--m-text)" }}>
                  {block.label}
                </span>
                <Tag tone={blockTone(block.alert)}>
                  {block.alert === "none" ? "healthy" : block.alert}
                </Tag>
              </div>
              {/* The bar is the balance: drawn (solid) against the whole
                  block. Width is a transform-free style, so it needs no
                  animation budget — the number is the fact. */}
              <div
                className="mt-1 h-1.5 w-full overflow-hidden rounded-full"
                style={{ background: "var(--m-surface-muted, var(--m-muted))" }}
                role="img"
                aria-label={`${pct}% of the block drawn`}
              >
                <div
                  className="h-full rounded-full"
                  style={{
                    width: `${pct}%`,
                    // The bar agrees with its own tag: a depleted block
                    // reads danger, an expiring one reads warning, and a
                    // healthy one stays neutral. Painting "healthy" with
                    // the interactive accent made dark mode look like an
                    // alert the tag then denied.
                    background:
                      block.alert === "depleted"
                        ? "var(--m-status-danger-fg)"
                        : block.alert === "expiring"
                          ? "var(--m-status-warning-fg)"
                          : "var(--m-text-secondary)",
                  }}
                />
              </div>
              <div
                className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] tabular-nums"
                style={{ color: "var(--m-text-secondary)" }}
              >
                <span data-testid={`verify-block-remaining-${block.block_id}`}>
                  {fmtMins(block.remaining_minutes)} left of {fmtMins(block.total_minutes)}
                </span>
                <span>{fmtMins(block.drawn_minutes)} drawn this window</span>
                {block.prior_drawn_minutes > 0 && (
                  <span>{fmtMins(block.prior_drawn_minutes)} drawn earlier</span>
                )}
                {block.overage_minutes > 0 && (
                  <span style={{ color: "var(--m-status-warning-fg)" }}>
                    {fmtMins(block.overage_minutes)} spilled to card rates
                  </span>
                )}
                {block.expires_on && (
                  <span style={{ color: "var(--m-text-muted)" }}>
                    expires {block.expires_on}
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </Card>
  );
}

/** A counter tile reads plain when its count is healthy and warm when it
 * blocks the close — the tone is the only place colour carries meaning
 * here, so the number is always spelled out beside it. */
function CounterTile({
  label,
  value,
  tone,
  testId,
}: {
  label: string;
  value: number;
  tone: StatusTone | "plain";
  testId: string;
}) {
  const color = tone === "plain" ? "var(--m-text)" : `var(--m-status-${tone}-fg)`;
  return (
    <div
      className="min-w-23 rounded-[var(--m-radius-control)] border px-3 py-2"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid={testId}
    >
      <div className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        {label}
      </div>
      <div className="text-[19px] font-semibold tabular-nums" style={{ color }}>
        {value}
      </div>
    </div>
  );
}

function ActionDialog({
  entry,
  currency,
  categories,
  onDone,
}: {
  entry: EntryView;
  currency: string;
  categories: { slug: string; label: string }[];
  onDone: (message: string) => void;
}) {
  const [action, setAction] = useState<Action>("adjust_minutes");
  const [reason, setReason] = useState("");
  const [reasonCode, setReasonCode] = useState<string>("missing_notes");
  const [unbillable, setUnbillable] = useState<string>("warranty");
  const [minutes, setMinutes] = useState(String(entry.effective_minutes));
  const [workType, setWorkType] = useState(entry.work_type ?? "");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reasonRequired = ["reject", "flip_billable", "adjust_minutes"].includes(action);
  const canSubmit =
    !busy &&
    (!reasonRequired || reason.trim().length > 0) &&
    (action !== "add_note" || note.trim().length > 0) &&
    (action !== "recategorize" || workType.trim().length > 0) &&
    (action !== "adjust_minutes" || Number.isFinite(Number(minutes)));

  const submit = async () => {
    setBusy(true);
    setError(null);
    const body: Record<string, unknown> = {
      action,
      idempotency_key: crypto.randomUUID(),
    };
    if (reasonRequired) body.reason = reason.trim();
    if (action === "reject") body.reason_code = reasonCode;
    if (action === "flip_billable" && !entry.disposition) body.unbillable_reason = unbillable;
    if (action === "adjust_minutes") body.minutes = Number(minutes);
    if (action === "recategorize") body.work_type = workType.trim();
    if (action === "add_note") body.note = note.trim();

    const result = await api.api.POST(
      "/api/v1/billing/verification/entries/{id}/action",
      { params: { path: { id: entry.id } }, body: body as never },
    );
    setBusy(false);
    if (result.error !== undefined) {
      setError(rejectionText(result.error, "The action was refused."));
      return;
    }
    onDone(`${action.replace("_", " ")} applied to this entry.`);
  };

  return (
    <DialogContent
      title="Verify entry"
      data-testid={`verify-action-dialog-${entry.id}`}
    >
      <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
        {fmtMins(entry.minutes)} logged
        {entry.adjustment_minutes !== 0 && ` · ${fmtMins(entry.effective_minutes)} after adjustments`}
        {` · ${money(entry.cost_minor, currency)} at ${money(entry.rate_minor, currency)}/h`}
      </p>
      <div className="mt-3 flex flex-col gap-3">
        <div>
          <Label htmlFor={`verify-action-${entry.id}`}>Action</Label>
          <Select value={action} onValueChange={(v) => setAction(v as Action)}>
            <SelectTrigger id={`verify-action-${entry.id}`} data-testid="verify-action-select">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="approve">Approve</SelectItem>
              <SelectItem value="reject">Reject</SelectItem>
              <SelectItem value="flip_billable">Flip billable</SelectItem>
              <SelectItem value="adjust_minutes">Adjust minutes</SelectItem>
              <SelectItem value="recategorize">Recategorize</SelectItem>
              <SelectItem value="add_note">Add note</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {action === "adjust_minutes" && (
          <div>
            <Label htmlFor={`verify-minutes-${entry.id}`}>Billable minutes</Label>
            <Input
              id={`verify-minutes-${entry.id}`}
              data-testid="verify-minutes-input"
              value={minutes}
              onChange={(e) => setMinutes(e.target.value)}
              inputMode="numeric"
            />
            <FieldHint>
              Verification lowers a bill, never raises it. The original {fmtMins(entry.minutes)}{" "}
              stays on the record; the ledger keeps the signed difference.
            </FieldHint>
          </div>
        )}

        {action === "reject" && (
          <div>
            <Label htmlFor={`verify-reason-code-${entry.id}`}>Rejection reason</Label>
            <Select value={reasonCode} onValueChange={setReasonCode}>
              <SelectTrigger id={`verify-reason-code-${entry.id}`} data-testid="verify-reason-code">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {REJECTION_CODES.map((code) => (
                  <SelectItem key={code} value={code}>
                    {code.replace(/_/g, " ")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}

        {action === "flip_billable" && (
          <div>
            <Label htmlFor={`verify-unbillable-${entry.id}`}>Why it is not billable</Label>
            <Select value={unbillable} onValueChange={setUnbillable}>
              <SelectTrigger id={`verify-unbillable-${entry.id}`} data-testid="verify-unbillable">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {UNBILLABLE_REASONS.map((r) => (
                  <SelectItem key={r.code} value={r.code}>
                    {r.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}

        {action === "recategorize" && (
          <div>
            <Label htmlFor={`verify-work-type-${entry.id}`}>Category</Label>
            <Select value={workType} onValueChange={setWorkType}>
              <SelectTrigger id={`verify-work-type-${entry.id}`} data-testid="verify-work-type">
                <SelectValue placeholder="Choose a category" />
              </SelectTrigger>
              <SelectContent>
                {categories.map((c) => (
                  <SelectItem key={c.slug} value={c.slug}>
                    {c.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}

        {action === "add_note" && (
          <div>
            <Label htmlFor={`verify-note-${entry.id}`}>Note</Label>
            <Input
              id={`verify-note-${entry.id}`}
              data-testid="verify-note-input"
              value={note}
              onChange={(e) => setNote(e.target.value)}
              placeholder="What the reviewer should know"
            />
          </div>
        )}

        {reasonRequired && (
          <div>
            <Label htmlFor={`verify-reason-${entry.id}`}>Reason</Label>
            <Input
              id={`verify-reason-${entry.id}`}
              data-testid="verify-reason-input"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              placeholder="Recorded on the audit row"
            />
            <FieldHint>The audit trail keeps this. An adjustment without one is refused.</FieldHint>
          </div>
        )}

        {error && (
          <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
            {error}
          </p>
        )}
      </div>
      <DialogActions>
        <Button onClick={submit} disabled={!canSubmit} data-testid="verify-action-submit">
          {busy ? "Saving…" : "Apply"}
        </Button>
      </DialogActions>
    </DialogContent>
  );
}

function EntryRow({
  entry,
  currency,
  categories,
  onAction,
}: {
  entry: EntryView;
  currency: string;
  categories: { slug: string; label: string }[];
  onAction: (entry: EntryView) => void;
}) {
  const badge = badgeTone(entry);
  const [open, setOpen] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  return (
    <tr data-testid={`verify-entry-${entry.id}`}>
      <td className="py-2 pr-3 align-top">
        <div className="flex flex-col gap-0.5">
          <span className="text-[13px] tabular-nums" style={{ color: "var(--m-text)" }}>
            {when(entry.started_at)} · {entry.user_name}
          </span>
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            {entry.work_type ?? "No category — prices at the contract default"}
            {entry.notes ? ` · ${entry.notes}` : ""}
          </span>
          {message && (
            <span className="text-[11px]" style={{ color: "var(--m-status-success-fg)" }}>
              {message}
            </span>
          )}
        </div>
      </td>
      <td className="py-2 pr-3 align-top text-[13px] tabular-nums" style={{ color: "var(--m-text)" }}>
        {fmtMins(entry.effective_minutes)}
        {entry.adjustment_minutes !== 0 && (
          <span className="ml-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            (was {fmtMins(entry.minutes)})
          </span>
        )}
      </td>
      <td className="py-2 pr-3 align-top text-[13px] tabular-nums" style={{ color: "var(--m-text)" }}>
        {money(entry.cost_minor, currency)}
      </td>
      <td className="py-2 pr-3 align-top">
        <Tag tone={badge.tone}>{badge.label}</Tag>
      </td>
      <td className="py-2 align-top">
        {entry.invoiced ? (
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Locked — invoiced
          </span>
        ) : (
          <div className="flex justify-end">
            <Dialog open={open} onOpenChange={setOpen}>
              <DialogTrigger asChild>
                <Button
                  variant="secondary"
                  size="sm"
                  data-testid={`verify-open-${entry.id}`}
                  onClick={() => onAction(entry)}
                >
                  Review
                </Button>
              </DialogTrigger>
              <ActionDialog
                entry={entry}
                currency={currency}
                categories={categories}
                onDone={(m) => {
                  setMessage(m);
                  setOpen(false);
                }}
              />
            </Dialog>
          </div>
        )}
      </td>
    </tr>
  );
}

export default function BillingVerificationPage() {
  // `useSearchParams` opts the subtree into client rendering, and this
  // app exports statically — without a boundary Next refuses to
  // prerender the route. The inner component owns the hooks, exactly as
  // `portal/csat/page.tsx` does it.
  return (
    <Suspense
      fallback={
        <div
          className="flex items-center gap-2 text-xs"
          style={{ color: "var(--m-text-secondary)" }}
        >
          <Spinner /> Loading the verification workspace…
        </div>
      }
    >
      <BillingVerificationWorkspace />
    </Suspense>
  );
}

function BillingVerificationWorkspace() {
  const auth = useRequireAuth();
  const { categories } = useTimeCategories();
  // The run composer links here with the window it was about to bill, so
  // the reviewer lands on the SAME dates instead of retyping them.
  const search = useSearchParams();
  const [contracts, setContracts] = useState<Contract[] | null>(null);
  // `contract_id` is honored so the workspace is deep-linkable: the run
  // composer and the capture/E2E flows both point at one exact contract,
  // and silently landing on "the first contract in the list" would show a
  // reviewer somebody else's empty window.
  const [contractId, setContractId] = useState<string | null>(search.get("contract_id"));
  const [from, setFrom] = useState(search.get("from") ?? firstOfMonth(-1));
  const [to, setTo] = useState(search.get("to") ?? isoDay(0));
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [signOpen, setSignOpen] = useState(false);
  const [signNote, setSignNote] = useState("");
  const [signBusy, setSignBusy] = useState(false);
  const [signError, setSignError] = useState<string | null>(null);

  const loadContracts = useCallback(async () => {
    const result = await api.api.GET("/api/v1/contracts");
    if (result.error !== undefined || result.data === undefined) {
      setError("Could not load contracts.");
      return;
    }
    setContracts(result.data as Contract[]);
    setContractId((current) => {
      if (current !== null && (result.data as Contract[]).some((c) => c.id === current)) {
        return current;
      }
      return (result.data as Contract[])[0]?.id ?? current;
    });
  }, []);

  const loadWorkspace = useCallback(async () => {
    if (contractId === null) {
      setWorkspace(null);
      return;
    }
    setLoading(true);
    setError(null);
    const result = await api.api.GET("/api/v1/billing/verification", {
      params: { query: { contract_id: contractId, from, to } },
    });
    setLoading(false);
    if (result.error !== undefined || result.data === undefined) {
      setWorkspace(null);
      setError("Could not load this window. Check the dates and try again.");
      return;
    }
    setWorkspace(result.data as Workspace);
  }, [contractId, from, to]);

  useEffect(() => {
    if (auth.status === "authed") void loadContracts();
  }, [auth.status, loadContracts]);

  useEffect(() => {
    if (auth.status === "authed") void loadWorkspace();
  }, [auth.status, loadWorkspace]);

  const categoryOptions = useMemo(
    () => (categories ?? []).map((c) => ({ slug: c.slug, label: c.label })),
    [categories],
  );

  const sign = async () => {
    if (contractId === null) return;
    setSignBusy(true);
    setSignError(null);
    const result = await api.api.POST("/api/v1/billing/verification/signoff", {
      body: {
        contract_id: contractId,
        from,
        to,
        entries_dispositioned: true,
        zero_unclassified: true,
        preview_seen: true,
        reviewer_note: signNote.trim() === "" ? null : signNote.trim(),
        idempotency_key: crypto.randomUUID(),
      } as never,
    });
    setSignBusy(false);
    if (result.error !== undefined) {
      setSignError(rejectionText(result.error, "The sign-off was refused."));
      return;
    }
    const data = result.data as SignoffResponse;
    setSignOpen(false);
    setSignNote("");
    const overlap = data.overlapping_runs.length;
    setNotice(
      overlap > 0
        ? `Signed off. ${overlap} issued run${overlap === 1 ? "" : "s"} overlap this window — check the warning before you run.`
        : "Signed off. The window is ready to invoice.",
    );
    await loadWorkspace();
  };

  const counters = workspace?.counters;
  const readyToSign =
    counters !== undefined && counters.undispositioned === 0 && counters.unclassified === 0;

  return (
    <div
      className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6"
      data-testid="billing-verification-page"
    >
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold" style={{ color: "var(--m-text)" }}>
            Billing verification
          </h1>
          <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            Read the period entry by entry, disposition it, then sign it off. Nothing here
            invoices anything — the run composer does that.
          </p>
        </div>
        <div className="flex flex-wrap items-end gap-2">
          <div>
            <Label htmlFor="verify-contract">Contract</Label>
            <Select value={contractId ?? ""} onValueChange={setContractId}>
              <SelectTrigger id="verify-contract" data-testid="verify-contract-select">
                <SelectValue placeholder="Choose a contract" />
              </SelectTrigger>
              <SelectContent>
                {(contracts ?? []).map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.number} — {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="verify-from">From</Label>
            <Input
              id="verify-from"
              type="date"
              data-testid="verify-from"
              value={from}
              onChange={(e) => setFrom(e.target.value)}
            />
          </div>
          <div>
            <Label htmlFor="verify-to">To</Label>
            <Input
              id="verify-to"
              type="date"
              data-testid="verify-to"
              value={to}
              onChange={(e) => setTo(e.target.value)}
            />
          </div>
          <Button variant="secondary" onClick={() => void loadWorkspace()} data-testid="verify-refresh">
            Reload
          </Button>
        </div>
      </header>

      {notice && (
        <p
          className="text-xs"
          style={{ color: "var(--m-status-success-fg)" }}
          role="status"
          data-testid="verify-notice"
        >
          {notice}
        </p>
      )}
      {error && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
          {error}
        </p>
      )}

      {loading && (
        <div className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          <Spinner /> Loading the window…
        </div>
      )}

      {workspace !== null && (
        <>
          <Card className="m-rise p-3" data-testid="verify-summary">
            <div className="flex flex-wrap items-baseline justify-between gap-3">
              <div>
                <div className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                  {workspace.contract_number} · {workspace.contract_name}
                </div>
                <div className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                  {workspace.client_name ?? "Client"} · {workspace.from} → {workspace.to}
                </div>
              </div>
              <div className="text-right">
                <div className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  Running total
                </div>
                <div
                  className="text-[22px] font-semibold tabular-nums"
                  style={{ color: "var(--m-text)" }}
                  data-testid="verify-total"
                >
                  {money(workspace.total_minor, workspace.currency)}
                </div>
                <div className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                  {fmtMins(workspace.total_minutes)} approved and billable
                </div>
              </div>
            </div>

            <div className="mt-3 flex flex-wrap gap-2">
              <CounterTile
                label="Undispositioned"
                value={counters?.entries !== undefined ? counters.undispositioned : 0}
                tone={counters !== undefined && counters.undispositioned > 0 ? "warning" : "plain"}
                testId="verify-counter-undispositioned"
              />
              <CounterTile
                label="Unclassified"
                value={counters?.unclassified ?? 0}
                tone={counters !== undefined && counters.unclassified > 0 ? "warning" : "plain"}
                testId="verify-counter-unclassified"
              />
              <CounterTile
                label="Not approved"
                value={counters?.draft ?? 0}
                tone={counters !== undefined && counters.draft > 0 ? "warning" : "plain"}
                testId="verify-counter-draft"
              />
              <CounterTile
                label="Non-billable"
                value={counters?.non_billable ?? 0}
                tone="plain"
                testId="verify-counter-non-billable"
              />
              <CounterTile
                label="Adjusted"
                value={counters?.adjusted ?? 0}
                tone="plain"
                testId="verify-counter-adjusted"
              />
              <CounterTile
                label="Invoiced"
                value={counters?.invoiced ?? 0}
                tone="plain"
                testId="verify-counter-invoiced"
              />
            </div>
          </Card>

          <BlockPanelCard blocks={workspace.blocks} currency={workspace.currency} />

          {workspace.overlapping_runs.length > 0 && (
            <div
              className="rounded-[var(--m-radius-control)] border px-3 py-2"
              style={{
                borderColor: "var(--m-status-warning-fg)",
                background: "var(--m-status-warning-bg)",
              }}
              role="alert"
              data-testid="verify-overlap-warning"
            >
              <div className="text-xs font-semibold" style={{ color: "var(--m-status-warning-fg)" }}>
                This window overlaps {workspace.overlapping_runs.length} issued run
                {workspace.overlapping_runs.length === 1 ? "" : "s"}
              </div>
              <ul className="mt-1 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                {workspace.overlapping_runs.map((run) => (
                  <li key={run.run_id} className="tabular-nums">
                    {run.number}: {run.period_from} → {run.period_to}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {workspace.groups.length === 0 ? (
            <EmptyState message="No time was logged for this client between the two dates. Widen the range or pick another contract." />
          ) : (
            <div className="flex flex-col gap-3">
              {workspace.groups.map((group) => (
                <Card key={group.group_id} className="p-3" data-testid={`verify-group-${group.group_id}`}>
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <div className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                      {group.label}
                      {group.project_name ? (
                        <span className="ml-2 text-[11px] font-normal" style={{ color: "var(--m-text-muted)" }}>
                          {group.project_name}
                        </span>
                      ) : null}
                    </div>
                    <div
                      className="text-xs tabular-nums"
                      style={{ color: "var(--m-text-secondary)" }}
                      data-testid={`verify-group-total-${group.group_id}`}
                    >
                      {fmtMins(group.total_minutes)} ·{" "}
                      {money(group.total_cost_minor, workspace.currency)}
                    </div>
                  </div>
                  <table className="mt-2 w-full border-collapse text-left">
                    <thead>
                      <tr>
                        {["Entry", "Billable", "Cost", "Disposition", ""].map((h, i) => (
                          <th
                            key={h + i}
                            className="border-b pb-1 text-[11px] font-medium uppercase tracking-wide"
                            style={{ color: "var(--m-text-muted)" }}
                          >
                            {h}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {group.entries.map((entry) => (
                        <EntryRow
                          key={entry.id}
                          entry={entry}
                          currency={workspace.currency}
                          categories={categoryOptions}
                          onAction={() => setNotice(null)}
                        />
                      ))}
                    </tbody>
                  </table>
                </Card>
              ))}
            </div>
          )}

          <Card className="p-3" data-testid="verify-close-card">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                  Close this period
                </h2>
                <ul className="mt-1 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                  <li data-testid="verify-check-entries">
                    {counters?.undispositioned === 0 ? "✓" : "•"} every entry dispositioned
                    {counters !== undefined && counters.undispositioned > 0
                      ? ` (${counters.undispositioned} left)`
                      : ""}
                  </li>
                  <li data-testid="verify-check-unclassified">
                    {counters?.unclassified === 0 ? "✓" : "•"} zero unclassified entries
                    {counters !== undefined && counters.unclassified > 0
                      ? ` (${counters.unclassified} left)`
                      : ""}
                  </li>
                  <li data-testid="verify-check-preview">
                    {workspace.preview_total_minor > 0 ? "✓" : "•"} preview read —{" "}
                    {money(workspace.preview_total_minor, workspace.currency)} this window
                  </li>
                </ul>
                {workspace.signoff && (
                  <div className="mt-1 text-xs" style={{ color: "var(--m-status-success-fg)" }} data-testid="verify-signoff-state">
                    <div>
                      Signed off {when(workspace.signoff.signed_at)}
                      {workspace.signoff.run_id ? " · claimed by a run" : " · not yet run"}
                    </div>
                    {workspace.signoff.reviewer_note ? (
                      // The note is the reviewer's own words and the only
                      // record of what they actually looked at, so it is
                      // read here rather than left in the API response.
                      <div style={{ color: "var(--m-text-secondary)" }}>
                        “{workspace.signoff.reviewer_note}”
                      </div>
                    ) : null}
                  </div>
                )}
              </div>
              <div className="flex items-center gap-2">
                <Button
                  onClick={() => {
                    setSignError(null);
                    setSignOpen(true);
                  }}
                  disabled={!readyToSign}
                  data-testid="verify-open-signoff"
                >
                  Sign off the period
                </Button>
                {!readyToSign && (
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    Work the counters first
                  </span>
                )}
              </div>
            </div>
          </Card>
        </>
      )}

      <Dialog open={signOpen} onOpenChange={setSignOpen}>
        <DialogContent
          title={`Sign off ${from} → ${to}`}
          data-testid="verify-signoff-dialog"
        >
          <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            The server re-checks all three items against the live window before it records
            anything. Your name and the moment are stored with it.
          </p>
          <div className="mt-3">
            <Label htmlFor="verify-signoff-note">Note for the record (optional)</Label>
            <Input
              id="verify-signoff-note"
              data-testid="verify-signoff-note"
              value={signNote}
              onChange={(e) => setSignNote(e.target.value)}
              placeholder="What you checked"
            />
            {workspace !== null && workspace.overlapping_runs.length > 0 && (
              <FieldHint>
                {workspace.overlapping_runs.length} issued run
                {workspace.overlapping_runs.length === 1 ? "" : "s"} overlap this window. Signing
                off is allowed; the warning stays on the record.
              </FieldHint>
            )}
          </div>
          {signError && (
            <p className="mt-2 text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {signError}
            </p>
          )}
          <DialogActions>
            <Button onClick={() => void sign()} disabled={signBusy} data-testid="verify-signoff-submit">
              {signBusy ? "Signing…" : "Sign off"}
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {workspace !== null && workspace.preview_total_minor !== workspace.total_minor && (
        // Direction decides the tone. A preview BELOW the entry sum is
        // money coming off (block rates, caps) — information, not a
        // problem, and colouring it amber trained reviewers to ignore
        // the one case that IS a problem: a preview ABOVE the sum, where
        // the invoice will carry charges this table never showed.
        <p
          className="text-xs"
          style={{
            color:
              workspace.preview_total_minor > workspace.total_minor
                ? "var(--m-status-warning-fg)"
                : "var(--m-text-secondary)",
          }}
          data-testid="verify-footing-note"
        >
          The engine preview for this window is{" "}
          {money(workspace.preview_total_minor, workspace.currency)} against{" "}
          {money(workspace.total_minor, workspace.currency)} of entry cost.{" "}
          {workspace.preview_total_minor > workspace.total_minor
            ? "The difference is a charge the entry table does not itemise — a base fee, a minimum, or a cap. Read the preview before you run."
            : workspace.blocks.length > 0
              ? "The difference is contract pricing this table does not itemise per entry — block rates and caps. The block panel above shows what drew the block."
              : "The difference is contract pricing this table does not itemise per entry."}
        </p>
      )}
    </div>
  );
}
