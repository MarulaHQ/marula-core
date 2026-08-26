// /portal/tickets/[id] — ticket detail + reply composer.
//
// Purpose: the client's view of one request — summary fields, then the
// conversation thread containing ONLY the note kinds their tier may read
// (server-enforced via core_domain::notes::visible_kinds; this page simply
// renders what arrives). Replies always land as client-visible notes.
// Non-purpose: no status changes, no assignment, no internal-note
// composition — portal v1 is strictly view + reply.
"use client";

import { Button, Card, Label, Spinner, StatusTag, Tag, Textarea, type StatusTone } from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { portalApi, useRequirePortalAuth, type PortalTicketDetail } from "../../../../lib/portal-api";

const CATEGORY_LABEL: Record<string, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

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

/** In-house updates get a distinct quiet treatment; client-visible notes
 * render plainly. Internal notes never reach this component. */
function NoteKindMark({ kind }: { kind: string }) {
  if (kind !== "in_house_update") return null;
  return <Tag tone="outline">Update from the team</Tag>;
}

function formatWhen(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export default function PortalTicketPage() {
  useRequirePortalAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [detail, setDetail] = useState<PortalTicketDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [replyBody, setReplyBody] = useState("");
  const [sending, setSending] = useState(false);
  const [replyError, setReplyError] = useState<string | null>(null);
  const threadEndRef = useRef<HTMLDivElement | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await portalApi.getTicket(params.id));
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : "Failed to load this request");
    }
  }, [params.id]);

  useEffect(() => {
    void load();
  }, [load]);

  async function submitReply(e: React.FormEvent) {
    e.preventDefault();
    if (replyBody.trim().length === 0 || detail === null) return;
    setSending(true);
    setReplyError(null);
    try {
      await portalApi.reply(detail.ticket.id, replyBody.trim());
      setReplyBody("");
      await load();
      threadEndRef.current?.scrollIntoView({ block: "end" });
    } catch (err) {
      setReplyError(err instanceof Error ? err.message : "Reply failed");
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
    <section className="space-y-6">
      <div>
        <button
          type="button"
          className="text-xs"
          style={{ color: "var(--m-text-muted)" }}
          onClick={() => router.push("/portal/tickets")}
        >
          ← All requests
        </button>
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <span className="font-mono text-xs" style={{ color: "var(--m-text-muted)" }}>
            {t.number}
          </span>
          <StatusTag tone={categoryTone(t.category)}>
            {CATEGORY_LABEL[t.category] ?? t.category}
          </StatusTag>
        </div>
        <h1 className="mt-1 text-lg font-semibold">{t.title}</h1>
        <p className="mt-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
          Opened {formatWhen(t.created_at)} · Updated {formatWhen(t.updated_at)}
        </p>
      </div>

      <Card className="divide-y" style={{ borderColor: "var(--m-border)" }}>
        {detail.comments.length === 0 ? (
          <p className="px-4 py-6 text-sm" style={{ color: "var(--m-text-muted)" }}>
            No updates yet.
          </p>
        ) : (
          detail.comments.map((c) => (
            <article key={c.id} className="px-4 py-3">
              <header className="mb-1.5 flex items-center gap-2 text-xs" style={{ color: "var(--m-text-muted)" }}>
                <span>{formatWhen(c.created_at)}</span>
                <NoteKindMark kind={c.kind} />
              </header>
              <p className="whitespace-pre-wrap text-sm">{c.body}</p>
            </article>
          ))
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
              {replyError}
            </p>
          )}
          <div className="flex items-center justify-between">
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Your reply is visible to your organization and the service desk.
            </span>
            <Button type="submit" disabled={sending || replyBody.trim().length === 0}>
              {sending ? "Sending…" : "Send reply"}
            </Button>
          </div>
        </form>
      </Card>
    </section>
  );
}
