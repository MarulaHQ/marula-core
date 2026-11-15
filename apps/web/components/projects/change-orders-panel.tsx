// Change orders on a project (S13.1, S13.2, S13.5, S13.5b, S13.6, S13.7,
// S13.8, S13.9, S13.10) plus the billing-release ledger (S6.5, S6.8).
//
// The rule is drawn into the layout: the document's steps and the two acts are
// separate controls, and each says what it will do. Approval never applies;
// apply never bills. A failed release keeps the plan change and says why.

"use client";

import {
  Badge,
  Button,
  ConfirmDialog,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldError,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
} from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";
import { api, useAuth } from "../../lib/api";
import { money } from "../../lib/format";
import {
  changeOrderAction,
  createChangeOrder,
  fetchChangeOrders,
  fetchClientUpdates,
  type ApplyResult,
  type ChangeOrder,
  type ClientUpdate,
} from "./delivery-api";
import { ClientUpdatePanel } from "./client-update-panel";

interface ReleaseRow {
  id: string;
  label: string;
  amount_minor: number;
  currency: string;
  released_on: string;
  status: "pending" | "billed" | "cancelled";
}

const STATUS_WORDS: Record<string, string> = {
  draft: "Draft",
  submitted: "Awaiting approval",
  approved: "Approved",
  rejected: "Rejected",
  awaiting_client: "Waiting for the client",
  applied: "Applied",
  billing_failed: "Applied, billing failed",
  billed: "Billed",
};

const STEPS = ["draft", "submitted", "approved", "applied", "billed"];

function stepIndex(status: string): number {
  if (status === "awaiting_client") return 2;
  if (status === "billing_failed") return 3;
  if (status === "rejected") return 1;
  const index = STEPS.indexOf(status);
  return index < 0 ? 0 : index;
}

function StepTrail({ status }: { status: string }) {
  const current = stepIndex(status);
  return (
    <ol className="flex items-center gap-1 text-[11px] text-muted-foreground" aria-label="Change order steps">
      {STEPS.map((step, index) => (
        <li key={step} className="flex items-center gap-1">
          <span
            className={
              index <= current
                ? "rounded-pill bg-mc-act px-2 py-[2px] font-semibold text-foreground"
                : "rounded-pill border border-border px-2 py-[2px]"
            }
          >
            {step === "awaiting_client" ? "client" : step}
          </span>
          {index < STEPS.length - 1 ? <span aria-hidden="true">›</span> : null}
        </li>
      ))}
    </ol>
  );
}

function CreateChangeOrderDialog({
  projectId,
  onCreated,
}: {
  projectId: string;
  onCreated: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [source, setSource] = useState<ChangeOrder["source"]>("manual");
  const [title, setTitle] = useState("");
  const [reason, setReason] = useState("");
  const [amount, setAmount] = useState("");
  const [currency, setCurrency] = useState("USD");
  const [summary, setSummary] = useState("");
  const [ackRequired, setAckRequired] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onCreate() {
    setBusy(true);
    setError(null);
    try {
      await createChangeOrder(projectId, {
        title: title.trim(),
        source,
        reason,
        scope_changes: {},
        client_summary: summary.trim() === "" ? null : summary.trim(),
        evidence: [],
        coverage_note: null,
        confidence: null,
        contract_clause: null,
        amount_minor: amount.trim() === "" ? null : Math.round(Number(amount) * 100),
        currency: amount.trim() === "" ? null : currency,
        client_ack_required: ackRequired,
      });
      setOpen(false);
      setTitle("");
      setReason("");
      setAmount("");
      setSummary("");
      await onCreated();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm">New change order</Button>
      </DialogTrigger>
      <DialogContent
        title="New change order"
        description="Why it is needed, what changes, what it costs, and what the client is told."
      >
        <div className="flex flex-col gap-3">
          <div>
            <Label htmlFor="co-source">How it arrived</Label>
            <Select value={source} onValueChange={(value) => setSource(value as ChangeOrder["source"])}>
              <SelectTrigger id="co-source">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="manual">Written by hand</SelectItem>
                <SelectItem value="scope_signal">From a scope-drift signal</SelectItem>
                <SelectItem value="client_request">From a client request</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="co-title">Title</Label>
            <Input id="co-title" value={title} onChange={(e) => setTitle(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="co-reason">Why</Label>
            <textarea
              id="co-reason"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              className="min-h-[72px] w-full rounded-ctl border border-border bg-card p-2 text-[13px]"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Label htmlFor="co-amount">Amount</Label>
              <Input
                id="co-amount"
                inputMode="decimal"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="Leave empty for no money"
              />
            </div>
            <div>
              <Label htmlFor="co-currency">Currency</Label>
              <Input
                id="co-currency"
                value={currency}
                onChange={(e) => setCurrency(e.target.value.toUpperCase())}
              />
            </div>
          </div>
          <div>
            <Label htmlFor="co-summary">What the client is told</Label>
            <textarea
              id="co-summary"
              value={summary}
              onChange={(e) => setSummary(e.target.value)}
              className="min-h-[64px] w-full rounded-ctl border border-border bg-card p-2 text-[13px]"
            />
          </div>
          <label className="flex items-center gap-2 text-[13px]">
            <input
              type="checkbox"
              checked={ackRequired}
              onChange={(e) => setAckRequired(e.target.checked)}
            />
            Wait for the client to acknowledge in the portal before it can be applied
          </label>
          {error !== null ? <FieldError>{error}</FieldError> : null}
        </div>
        <DialogActions>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button disabled={busy || title.trim() === ""} onClick={() => void onCreate()}>
            {busy ? "Saving…" : "Save draft"}
          </Button>
        </DialogActions>
      </DialogContent>
    </Dialog>
  );
}

function RejectDialog({ projectId, order, onDone }: { projectId: string; order: ChangeOrder; onDone: () => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm" variant="ghost">
          Reject
        </Button>
      </DialogTrigger>
      <DialogContent title="Reject this change" description="The reason stays on the record, and the author can reopen it with everything kept.">
        <Label htmlFor="reject-note">Reason</Label>
        <textarea
          id="reject-note"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          className="min-h-[72px] w-full rounded-ctl border border-border bg-card p-2 text-[13px]"
        />
        {error !== null ? <FieldError>{error}</FieldError> : null}
        <DialogActions>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive-solid"
            disabled={note.trim() === ""}
            onClick={() => {
              changeOrderAction(projectId, order.id, "reject", note)
                .then(async () => {
                  setOpen(false);
                  await onDone();
                })
                .catch((e: Error) => setError(e.message));
            }}
          >
            Reject with reason
          </Button>
        </DialogActions>
      </DialogContent>
    </Dialog>
  );
}

export function ChangeOrdersPanel({ projectId }: { projectId: string }) {
  const auth = useAuth();
  const canReadMoney = (auth.me?.permissions ?? []).includes("financial:read");
  const canEdit = (auth.me?.permissions ?? []).includes("projects:edit");

  const [orders, setOrders] = useState<ChangeOrder[] | null>(null);
  const [releases, setReleases] = useState<ReleaseRow[]>([]);
  const [updates, setUpdates] = useState<ClientUpdate[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNote, setActionNote] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [cancelTarget, setCancelTarget] = useState<ReleaseRow | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [nextOrders, releaseBody, nextUpdates] = await Promise.all([
        fetchChangeOrders(projectId),
        api.api.GET("/api/v1/projects/{id}/billing-releases", {
          params: { path: { id: projectId } },
        }),
        fetchClientUpdates(projectId),
      ]);
      setOrders(nextOrders);
      setReleases((releaseBody.data ?? []) as ReleaseRow[]);
      setUpdates(nextUpdates);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [projectId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function act(order: ChangeOrder, action: "submit" | "approve" | "apply" | "bill") {
    setBusyId(order.id);
    setActionError(null);
    setActionNote(null);
    try {
      const result = await changeOrderAction(projectId, order.id, action);
      if (action === "apply") {
        const applied = result as ApplyResult;
        setActionNote(
          `Applied: ${applied.tasks_added} task${
            applied.tasks_added === 1 ? "" : "s"
          } added, ${applied.tasks_shifted} shifted, baseline captured. Nothing has been billed.`,
        );
      } else if (action === "bill") {
        setActionNote("The release was staged. The next billing run that reaches its window picks it up.");
      }
      await load();
    } catch (e) {
      setActionError((e as Error).message);
      await load();
    } finally {
      setBusyId(null);
    }
  }

  async function cancelRelease() {
    if (cancelTarget === null) return;
    try {
      await api.api.POST("/api/v1/projects/{id}/billing-releases/{release_id}/cancel", {
        params: { path: { id: projectId, release_id: cancelTarget.id } },
      });
      setCancelTarget(null);
      await load();
    } catch (e) {
      setActionError((e as Error).message);
    }
  }

  if (orders === null && error === null) {
    return (
      <div className="flex items-center gap-2 p-6 text-[13px] text-muted-foreground">
        <Spinner /> Reading change orders…
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-[16px] font-semibold">Change orders</h2>
          <p className="text-[13px] text-muted-foreground">
            The document records the change. Applying changes the plan; billing releases
            the money. They are separate on purpose.
          </p>
        </div>
        {canEdit ? <CreateChangeOrderDialog projectId={projectId} onCreated={load} /> : null}
      </div>

      {error !== null ? (
        <div role="alert" className="flex items-center gap-3 text-[13px]">
          <span className="flex-1">Change orders could not be read: {error}</span>
          <Button variant="secondary" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : null}
      {actionError !== null ? (
        <p role="alert" className="rounded-ctl border border-destructive px-3 py-2 text-[13px] text-destructive">
          {actionError}
        </p>
      ) : null}
      {actionNote !== null ? (
        <p className="rounded-ctl bg-muted px-3 py-2 text-[13px]">{actionNote}</p>
      ) : null}

      {(orders ?? []).length === 0 ? (
        <EmptyState message="No change orders. When the scope moves, write it down before the work does — a change order keeps the plan, the money and the client's expectation in one record." />
      ) : (
        <ul className="flex flex-col gap-3">
          {(orders ?? []).map((order) => (
            <li key={order.id} className="rounded-card border border-border bg-card p-4">
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-[12px] text-muted-foreground">
                      {order.reference}
                    </span>
                    <h3 className="truncate text-[14px] font-semibold">{order.title}</h3>
                    <Badge tone={order.status === "rejected" ? "danger" : order.status === "billing_failed" ? "warning" : "neutral"}>
                      {STATUS_WORDS[order.status]}
                    </Badge>
                  </div>
                  <p className="mt-1 text-[13px] text-muted-foreground">{order.reason}</p>
                </div>
                <div className="text-right">
                  {order.amount_minor !== null ? (
                    canReadMoney ? (
                      <div className="font-mono text-[14px] tabular-nums">
                        {money(order.amount_minor, order.currency ?? "USD")}
                      </div>
                    ) : (
                      <div className="text-[12px] text-muted-foreground" title="Needs financial access">
                        Amount hidden
                      </div>
                    )
                  ) : (
                    <div className="text-[12px] text-muted-foreground">No money</div>
                  )}
                </div>
              </div>

              <div className="mt-3 flex flex-wrap items-center gap-3">
                <StepTrail status={order.status} />
                <span className="ml-auto flex flex-wrap items-center gap-2">
                  {canEdit && order.status === "draft" ? (
                    <Button size="sm" variant="secondary" disabled={busyId === order.id} onClick={() => void act(order, "submit")}>
                      Submit for approval
                    </Button>
                  ) : null}
                  {canEdit && order.status === "submitted" ? (
                    <>
                      <Button size="sm" variant="ghost" disabled={busyId === order.id} onClick={() => void act(order, "approve")}>
                        Approve
                      </Button>
                      <RejectDialog projectId={projectId} order={order} onDone={load} />
                    </>
                  ) : null}
                  {canEdit && order.status === "approved" ? (
                    <Button size="sm" disabled={busyId === order.id} onClick={() => void act(order, "apply")}>
                      Apply to the plan
                    </Button>
                  ) : null}
                  {canEdit && (order.status === "applied" || order.status === "billing_failed") ? (
                    <Button size="sm" disabled={busyId === order.id} onClick={() => void act(order, "bill")}>
                      {order.status === "billing_failed" ? "Retry billing" : "Release the billing line"}
                    </Button>
                  ) : null}
                </span>
              </div>

              {order.status === "awaiting_client" ? (
                <p className="mt-2 text-[13px] text-muted-foreground">
                  Waiting for the client to acknowledge in the portal. Approved
                  internally{order.decided_at !== null ? ` on ${order.decided_at.slice(0, 10)}` : ""};
                  it can be applied once they have seen it.
                </p>
              ) : null}
              {order.billing_error !== null ? (
                <p role="alert" className="mt-2 rounded-ctl border border-border px-3 py-2 text-[13px] text-destructive">
                  The release did not go through: {order.billing_error}
                </p>
              ) : null}
              {order.decision_note !== null ? (
                <p className="mt-2 text-[13px] text-muted-foreground">
                  Decision: {order.decision_note}
                </p>
              ) : null}
              {order.status === "billed" ? (
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Billed{order.billed_at !== null ? ` ${order.billed_at.slice(0, 10)}` : ""}. This
                  record is read-only history.
                </p>
              ) : null}
            </li>
          ))}
        </ul>
      )}

      <section aria-label="Billing releases" className="flex flex-col gap-2">
        <h3 className="text-[14px] font-semibold">Billing releases</h3>
        {releases.length === 0 ? (
          <p className="text-[13px] text-muted-foreground">Nothing staged for billing.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {releases.map((release) => (
              <li
                key={release.id}
                className="flex flex-wrap items-center gap-3 rounded-ctl border border-border px-3 py-2 text-[13px]"
              >
                <span className="flex-1">{release.label}</span>
                <span className="font-mono tabular-nums">
                  {canReadMoney ? money(release.amount_minor, release.currency) : "Hidden"}
                </span>
                <Badge tone={release.status === "pending" ? "due" : release.status === "billed" ? "success" : "offline"}>
                  {release.status}
                </Badge>
                <span className="font-mono text-[12px] text-muted-foreground">{release.released_on}</span>
                {release.status === "pending" && canEdit ? (
                  <Button size="sm" variant="ghost" onClick={() => setCancelTarget(release)}>
                    Cancel
                  </Button>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </section>

      <ConfirmDialog
        open={cancelTarget !== null}
        onOpenChange={(open) => {
          if (!open) setCancelTarget(null);
        }}
        title="Cancel this pending release?"
        description={
          cancelTarget === null
            ? ""
            : `${cancelTarget.label} will not be billed. The plan, the change order and any invoice already issued are not changed.`
        }
        confirmLabel="Cancel the release"
        onConfirm={async () => {
          await cancelRelease();
        }}
      />

      <ClientUpdatePanel projectId={projectId} updates={updates} onChanged={load} />
    </div>
  );
}
