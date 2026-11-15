// Client updates on the project record (S2.11 review the draft, S2.12
// withheld when nothing moved).
//
// The draft is built from records, the person edits it, approves it and sends
// it. The model never sends anything: there is no automatic send path here.

"use client";

import {
  Badge,
  Button,
  EmptyState,
  Input,
  Label,
  Spinner,
} from "@marula-core/ui";
import { useState } from "react";
import { useAuth } from "../../lib/api";
import {
  clientUpdateAction,
  draftClientUpdate,
  editClientUpdate,
  type ClientUpdate,
} from "./delivery-api";

const STATUS_TONE: Record<string, "neutral" | "warning" | "success" | "offline"> = {
  draft: "warning",
  approved: "success",
  sent: "success",
  withheld: "offline",
};

export function ClientUpdatePanel({
  projectId,
  updates,
  onChanged,
}: {
  projectId: string;
  updates: ClientUpdate[];
  onChanged: () => Promise<void>;
}) {
  const auth = useAuth();
  const canEdit = (auth.me?.permissions ?? []).includes("projects:edit");
  const [editing, setEditing] = useState<string | null>(null);
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [withholdId, setWithholdId] = useState<string | null>(null);
  const [withholdReason, setWithholdReason] = useState("");
  const [closing, setClosing] = useState("");

  async function run(id: string, fn: () => Promise<unknown>) {
    setBusy(id);
    setError(null);
    try {
      await fn();
      await onChanged();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  }

  return (
    <section aria-label="Client updates" className="flex flex-col gap-3 border-t border-border pt-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className="text-[14px] font-semibold">Client updates</h3>
          <p className="text-[13px] text-muted-foreground">
            Built from what moved. A person reviews it, approves it and sends it.
          </p>
        </div>
        {canEdit ? (
          <div className="flex items-end gap-2">
            <div>
              <Label htmlFor="closing">What comes next (optional)</Label>
              <Input
                id="closing"
                value={closing}
                onChange={(e) => setClosing(e.target.value)}
                placeholder="Next milestone date, if you want to promise it"
                className="w-64"
              />
            </div>
            <Button
              size="sm"
              disabled={busy === "new"}
              onClick={() =>
                void run("new", () =>
                  draftClientUpdate(projectId, {
                    period_start: null,
                    period_end: null,
                    closing: closing.trim() === "" ? null : closing.trim(),
                  }),
                )
              }
            >
              Draft from what moved
            </Button>
          </div>
        ) : null}
      </div>

      {error !== null ? (
        <p role="alert" className="rounded-ctl border border-destructive px-3 py-2 text-[13px] text-destructive">
          {error}
        </p>
      ) : null}

      {updates.length === 0 ? (
        <EmptyState message="No updates yet. Draft one from the work completed in the period; if nothing moved, the record says so rather than inventing a letter." />
      ) : (
        <ul className="flex flex-col gap-2">
          {updates.map((update) => (
            <li key={update.id} className="rounded-ctl border border-border bg-card p-3">
              <div className="flex flex-wrap items-center gap-2">
                <Badge tone={STATUS_TONE[update.status] ?? "neutral"}>{update.status}</Badge>
                <span className="text-[12px] text-muted-foreground">
                  {update.period_start ?? "—"} → {update.period_end ?? "—"} ·{" "}
                  {update.facts.length} record{update.facts.length === 1 ? "" : "s"} behind it
                </span>
                {update.sent_at !== null ? (
                  <span className="ml-auto font-mono text-[12px] text-muted-foreground">
                    sent {update.sent_at.slice(0, 10)}
                  </span>
                ) : null}
              </div>

              {editing === update.id ? (
                <div className="mt-2 flex flex-col gap-2">
                  <Label htmlFor={`body-${update.id}`}>The client's words</Label>
                  <textarea
                    id={`body-${update.id}`}
                    value={body}
                    onChange={(e) => setBody(e.target.value)}
                    className="min-h-[140px] w-full rounded-ctl border border-border bg-card p-2 text-[13px]"
                  />
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      disabled={busy === update.id}
                      onClick={() =>
                        void run(update.id, async () => {
                          await editClientUpdate(projectId, update.id, body);
                          setEditing(null);
                        })
                      }
                    >
                      Save the draft
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setEditing(null)}>
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : (
                <pre className="mt-2 whitespace-pre-wrap font-sans text-[13px] leading-[1.55]">
                  {update.body}
                </pre>
              )}

              {update.status === "withheld" && update.withheld_reason !== null ? (
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Withheld: {update.withheld_reason}
                </p>
              ) : null}

              {canEdit && editing !== update.id ? (
                <div className="mt-2 flex flex-wrap gap-2">
                  {update.status === "draft" ? (
                    <>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => {
                          setEditing(update.id);
                          setBody(update.body);
                        }}
                      >
                        Edit
                      </Button>
                      <Button
                        size="sm"
                        disabled={busy === update.id}
                        onClick={() =>
                          void run(update.id, () =>
                            clientUpdateAction(projectId, update.id, "approve"),
                          )
                        }
                      >
                        Approve
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => {
                          setWithholdId(update.id);
                          setWithholdReason("");
                        }}
                      >
                        Withhold
                      </Button>
                    </>
                  ) : null}
                  {update.status === "approved" ? (
                    <Button
                      size="sm"
                      disabled={busy === update.id}
                      onClick={() =>
                        void run(update.id, () =>
                          clientUpdateAction(projectId, update.id, "send"),
                        )
                      }
                    >
                      Send to the client
                    </Button>
                  ) : null}
                </div>
              ) : null}

              {withholdId === update.id ? (
                <div className="mt-2 flex flex-col gap-2 rounded-ctl bg-muted p-2">
                  <Label htmlFor={`withhold-${update.id}`}>Why is it withheld?</Label>
                  <Input
                    id={`withhold-${update.id}`}
                    value={withholdReason}
                    onChange={(e) => setWithholdReason(e.target.value)}
                  />
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={withholdReason.trim() === "" || busy === update.id}
                      onClick={() =>
                        void run(update.id, async () => {
                          await clientUpdateAction(
                            projectId,
                            update.id,
                            "withhold",
                            withholdReason.trim(),
                          );
                          setWithholdId(null);
                        })
                      }
                    >
                      Withhold it
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setWithholdId(null)}>
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      )}

      {busy === "new" ? (
        <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
          <Spinner /> Reading what moved…
        </div>
      ) : null}
    </section>
  );
}
