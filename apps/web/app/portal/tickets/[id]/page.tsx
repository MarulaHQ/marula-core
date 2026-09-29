// /portal/tickets/[id] — ticket detail + reply composer.
//
// Purpose: the client's view of one request — a quiet, exact status bar at
// the top (color + text, never color alone), then the conversation thread
// containing ONLY the note kinds their tier may read (server-enforced via
// core_domain::notes::visible_kinds; this page simply renders what
// arrives). Replies land as client-visible notes and render optimistically:
// the reply appears immediately and reconciles when the server answers
// (docs/instant-first.md — optimistic render, authoritative reconcile);
// a rejection surfaces a typed banner and keeps the text. Attachments
// listed under the pinned portal endpoint: metadata rows with real
// downloads; an empty attachments set renders a note, never a dead button.
// Non-purpose: no status changes, no assignment, no internal-note
// composition, no attachment upload (no client-facing upload op in the
// pins) — portal v1 is strictly view + reply.
//
// Wave 17 lane 8 UX pass: status bar, priority + due glances, thread
// bubbles with distinct update-from-the-team treatment, optimistic reply,
// attachments wiring.
"use client";

import { friendlyMutationText } from "../../../../lib/error-copy";
import { Button, Card, Label, Spinner, StatusTag, Tag, Textarea, type StatusTone } from "@marula-core/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalAttachmentView,
  type PortalTicketDetail,
} from "../../../../lib/portal-api";

const CATEGORY_LABEL: Record<string, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

const PRIORITY_LABEL: Record<string, string> = { p1: "P1", p2: "P2", p3: "P3", p4: "P4" };

function categoryTone(category: string): StatusTone {
  switch (category) {
    case "waiting":
      return "warning";
    case "closed":
      return "success";
    default:
      return "neutral";
  }
}

function formatWhen(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Exact status bar: color + text pairing on the token set (docs/theme.md
 * §3.3). Mirrors the staff workbench bar at portal-appropriate density. */
function TicketStatusBar({ category }: { category: string }) {
  const tone = category === "closed" ? "success" : category === "waiting" ? "warning" : "neutral";
  const bgVar = `var(--m-status-${tone === "neutral" ? "neutral" : tone === "success" ? "success" : "warning"}-bg)`;
  const fgVar = `var(--m-status-${tone === "neutral" ? "neutral" : tone === "success" ? "success" : "warning"}-fg)`;
  const label = CATEGORY_LABEL[category] ?? category;
  const line =
    category === "closed"
      ? "Resolved — your service desk closed this request."
      : category === "waiting"
        ? "Waiting on your organization — a reply from you moves it forward."
        : category === "in_progress"
          ? "In progress — your service desk is working on it."
          : "Open — queued with the service desk.";
  return (
    <div
      className="flex items-center gap-2 rounded-[0.75rem] px-3 py-2 text-xs"
      style={{ background: bgVar, color: fgVar }}
      role="status"
      data-testid="ticket-status-bar"
    >
      <span className="font-semibold uppercase tracking-wide">{label}</span>
      <span aria-hidden>·</span>
      <span>{line}</span>
    </div>
  );
}

interface PendingReply {
  id: string;
  body: string;
}

/** In-house updates get a distinct quiet treatment; client-visible notes
 * render plainly. Internal notes never reach this component. Locally
 * pending replies (optimistic) render in the composer's side of the
 * thread until the authoritative reload replaces them. */
function NoteKindMark({ kind }: { kind: string }) {
  if (kind !== "in_house_update") return null;
  return <Tag tone="outline">Update from the team</Tag>;
}

export default function PortalTicketPage() {
  useRequirePortalAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [detail, setDetail] = useState<PortalTicketDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<PortalAttachmentView[] | null>(null);
  const [replyBody, setReplyBody] = useState("");
  const [replyError, setReplyError] = useState<string | null>(null);
  const [pendingReply, setPendingReply] = useState<PendingReply | null>(null);
  const [sending, setSending] = useState(false);
  const threadEndRef = useRef<HTMLDivElement | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await portalApi.getTicket(params.id));
    } catch (err) {
      setLoadError(friendlyMutationText(err, "Failed to load this request"));
    }
  }, [params.id]);

  useEffect(() => {
    void load();
    let cancelled = false;
    portalApi
      .listTicketAttachments(params.id)
      .then((a) => {
        if (!cancelled) setAttachments(a);
      })
      .catch(() => {
        if (!cancelled) setAttachments([]);
      });
    return () => {
      cancelled = true;
    };
  }, [load, params.id]);

  async function submitReply(e: React.FormEvent) {
    e.preventDefault();
    const body = replyBody.trim();
    if (body.length === 0 || detail === null || sending) return;
    setSending(true);
    setReplyError(null);
    setPendingReply({ id: `pending-${Date.now()}`, body });
    setReplyBody("");
    try {
      await portalApi.reply(detail.ticket.id, body);
      setPendingReply(null);
      await load();
    } catch (err) {
      setReplyError(friendlyMutationText(err, "Reply failed"));
      setPendingReply(null);
      setReplyBody(body);
    } finally {
      setSending(false);
    }
  }

  if (loadError !== null) {
    return (
      <div className="space-y-3">
        <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>
          {loadError}
        </p>
        <Button variant="secondary" size="sm" onClick={() => router.push("/portal/tickets")}>
          Back to requests
        </Button>
      </div>
    );
  }
  if (detail === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading request" />
      </div>
    );
  }

  const t = detail.ticket;
  return (
    <section className="m-rise space-y-5">
      <div>
        <button
          type="button"
          className="text-xs transition-colors duration-100 hover:underline"
          style={{ color: "var(--m-text-muted)" }}
          onClick={() => router.push("/portal/tickets")}
        >
          ← All requests
        </button>
        <div className="mt-2 flex flex-wrap items-baseline gap-2">
          <h1 className="text-xl font-semibold tracking-tight">{t.title}</h1>
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs" style={{ color: "var(--m-text-muted)" }}>
          <span className="font-mono tabular-nums">{t.number}</span>
          <StatusTag tone={categoryTone(t.category)}>
            {CATEGORY_LABEL[t.category] ?? t.category}
          </StatusTag>
          <Tag tone="outline">{PRIORITY_LABEL[t.priority] ?? t.priority}</Tag>
          {t.due_at !== null && t.due_at !== undefined && t.category !== "closed" && (
            <span className="tabular-nums">
              Due {new Date(t.due_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
            </span>
          )}
          <span className="tabular-nums">
            Opened {formatWhen(t.created_at)} · Updated {formatWhen(t.updated_at)}
          </span>
        </div>
        <div className="mt-3">
          <TicketStatusBar category={t.category} />
        </div>
      </div>

      <Card className="divide-y" style={{ borderColor: "var(--m-border)" }}>
        {detail.comments.length === 0 && pendingReply === null ? (
          <p className="px-4 py-6 text-sm" style={{ color: "var(--m-text-muted)" }}>
            No updates yet.
          </p>
        ) : (
          <>
            {detail.comments.map((c, i) => {
              const isUpdate = c.kind === "in_house_update";
              return (
                <article
                  key={c.id}
                  className="m-rise px-4 py-3"
                  style={{ animationDelay: `${Math.min(i * 30, 180)}ms` }}
                >
                  <header
                    className="mb-1.5 flex items-center gap-2 text-xs"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    <span className="tabular-nums">{formatWhen(c.created_at)}</span>
                    <NoteKindMark kind={c.kind} />
                  </header>
                  <p
                    className="whitespace-pre-wrap text-sm leading-6"
                    style={{ color: isUpdate ? "var(--m-text-secondary)" : "var(--m-text)" }}
                  >
                    {c.body}
                  </p>
                </article>
              );
            })}
            {pendingReply !== null && (
              <article className="px-4 py-3" data-testid="portal-pending-reply">
                <header className="mb-1.5 flex items-center gap-2 text-xs" style={{ color: "var(--m-text-muted)" }}>
                  <span>You</span>
                  <span className="inline-flex items-center gap-1 opacity-70">
                    <span className="m-spinner" style={{ width: 10, height: 10 }} aria-hidden />
                    sending…
                  </span>
                </header>
                <p className="whitespace-pre-wrap text-sm leading-6">{pendingReply.body}</p>
              </article>
            )}
          </>
        )}
        <div ref={threadEndRef} />
      </Card>

      <Card className="p-4">
        <form className="space-y-3" onSubmit={submitReply}>
          <Label htmlFor="portal-reply">Reply</Label>
          <Textarea
            id="portal-reply"
            rows={4}
            required
            value={replyBody}
            onChange={(e) => setReplyBody(e.target.value)}
            placeholder="Write an update for the service desk…"
          />
          {replyError !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger)" }} role="alert">
              {replyError} — your reply was not delivered; the text is preserved above.
            </p>
          )}
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Your reply is visible to your organization and the service desk.
            </span>
            <Button type="submit" disabled={sending || replyBody.trim().length === 0}>
              {sending ? "Sending…" : "Send reply"}
            </Button>
          </div>
        </form>
      </Card>

      {attachments !== null && attachments.length > 0 && (
        <Card>
          <div className="border-b px-4 py-2.5 text-sm font-semibold" style={{ borderColor: "var(--m-border)" }}>
            Attachments
          </div>
          <ul className="divide-y" style={{ borderColor: "var(--m-border)" }}>
            {attachments.map((att, i) => (
              <li
                key={att.id}
                className="m-rise flex items-center justify-between gap-3 px-4 py-2.5"
                style={{ animationDelay: `${Math.min(i * 30, 180)}ms` }}
                data-testid={`portal-attachment-${i}`}
              >
                <div className="min-w-0">
                  <div className="truncate text-sm">{att.filename}</div>
                  <div className="text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                    {sizeLabel(att.size_bytes)}
                  </div>
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    void portalApi
                      .ticketAttachmentUrl(params.id, att.id)
                      .then((url) => {
                        const a = document.createElement("a");
                        a.href = url;
                        a.download = att.filename;
                        document.body.appendChild(a);
                        a.click();
                        a.remove();
                        window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
                      })
                      .catch(() => {
                        /* 404/412: the download simply doesn't start — no fake state */
                      });
                  }}
                >
                  Download
                </Button>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </section>
  );
}
