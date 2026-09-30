// /portal/tickets/[id] — one request, the thread, and the reply box.
//
// Purpose: the client's view of a single request. A title in her words, the
// plain-worded state, and the conversation. The pack's state 03 is the shape:
// messages with a name and a time, and a reply box underneath that says who
// sees it.
//
// THE PACK'S OTHER STATE, THE ONE THAT MATTERS MOST. State 04 is a brand-new
// ticket with no reply yet — the failure to design against. It carries
// "We have it, and someone is looking": what has happened, what is happening
// now, and what comes next, plus the promise of an email the moment there is a
// reply. That card renders here whenever the desk has not answered yet, so an
// empty ticket is never an empty screen.
//
// TWO HONEST GAPS, BOTH VISIBLE IN THE CODE AND RECORDED AS FOLLOW-UPS.
// 1. AUTHORSHIP. The pinned `PortalComment` carries a `kind` and a `body`, and
//    no author field, so the API cannot tell us which side of the conversation
//    a `client_visible` message came from. A technician's name is therefore
//    never invented. A desk message is labelled when the server says it is one
//    (`in_house_update`); the rest of the thread is presented as the
//    conversation, and a message we recognise as hers because she just sent
//    it is marked "You" and takes the gold tint.
// 2. UPLOAD. The pins expose attachment metadata and a download but no upload
//    operation, so there is no "Attach a file" button — a button that cannot
//    attach is exactly the dead button this repo forbids. The line under the
//    composer says where files go instead.
//
// WHAT IS NOT SHOWN. No internal note reaches this component (the API
// withholds them by tier), no priority, no board, no SLA clock, no due date.
// The API exposes no assignee, so the line says when it was raised and, while
// nobody has answered, that it is not yet assigned.
//
// REPLY. Rides the instant-first envelope: it appears immediately and
// reconciles when the server answers (docs/instant-first.md). A rejection
// surfaces in words and keeps the text. Send is disabled until there is
// something to send.
"use client";

import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { friendlyMutationText } from "../../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalAttachmentView,
  type PortalTicketDetail,
} from "../../../../lib/portal-api";
import { clientStatusFor, initials, longDate } from "../../../../lib/portal-copy";
import {
  Note,
  PortalCard,
  PortalIcon,
  StatusPill,
  TicketProgress,
} from "../../../../components/portal/portal-ui";

function when(iso: string): string {
  return new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export default function PortalTicketPage() {
  useRequirePortalAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [detail, setDetail] = useState<PortalTicketDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<PortalAttachmentView[] | null>(null);
  const [reply, setReply] = useState("");
  const [replyError, setReplyError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  // The last body she sent this session, so a message we can recognise as hers
  // is attributed to her. See the authorship note in the header.
  const lastSent = useRef<string | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await portalApi.getTicket(params.id));
    } catch (err) {
      setLoadError(
        friendlyMutationText(err, "We couldn't open this ticket. Try again in a moment."),
      );
    }
  }, [params.id]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listTicketAttachments(params.id)
      .then((list) => {
        if (!cancelled) setAttachments(list);
      })
      .catch(() => {
        if (!cancelled) setAttachments([]);
      });
    return () => {
      cancelled = true;
    };
  }, [params.id]);

  async function submitReply(e: React.FormEvent) {
    e.preventDefault();
    const body = reply.trim();
    if (body.length === 0 || detail === null || sending) return;
    setSending(true);
    setReplyError(null);
    setPending(body);
    setReply("");
    try {
      await portalApi.reply(detail.ticket.id, body);
      lastSent.current = body;
      setPending(null);
      await load();
    } catch (err) {
      setReplyError(
        friendlyMutationText(
          err,
          "Your reply didn't go through. It's kept below, so you can send it again.",
        ),
      );
      setPending(null);
      setReply(body);
    } finally {
      setSending(false);
    }
  }

  if (loadError !== null) {
    return (
      <PortalCard>
        <div className="portal-card__body">
          <p className="portal-field__error" role="alert">
            <PortalIcon name="alert" size={14} />
            <span>{loadError}</span>
          </p>
          <p>
            <button
              type="button"
              className="portal-link"
              onClick={() => router.push("/portal/tickets")}
            >
              Back to your tickets
            </button>
          </p>
        </div>
      </PortalCard>
    );
  }

  if (detail === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading this ticket" />
      </div>
    );
  }

  const t = detail.ticket;
  const status = clientStatusFor(t.category);
  // "Nobody has answered yet" is the case the pack's state 04 is drawn for,
  // and the only staffing fact the pinned API supports.
  const nothingYet = status === "received";
  const deskSaidSomething = detail.comments.some((c) => c.kind === "in_house_update");

  return (
    <>
      <div>
        <button
          type="button"
          className="portal-back"
          onClick={() => router.push("/portal/tickets")}
        >
          <PortalIcon name="back" size={14} />
          Your tickets
        </button>
      </div>

      <div>
        <h1 className="portal-title">{t.title}</h1>
        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            alignItems: "center",
            gap: 12,
            marginTop: 8,
          }}
        >
          <StatusPill status={status} />
          <span className="portal-hint" style={{ fontSize: 14 }}>
            #{t.number} · Raised {longDate(t.created_at)}
            {nothingYet ? " · Not yet assigned" : ""}
          </span>
        </div>
      </div>

      {detail.comments.length === 0 && pending === null ? (
        <p className="portal-hint" style={{ margin: 0 }}>
          You haven&apos;t written anything on this yet. Tell us what you need below.
        </p>
      ) : (
        <div
          style={{ display: "flex", flexDirection: "column", gap: 18 }}
          data-testid="portal-thread"
        >
          {detail.comments.map((c) => {
            const fromDesk = c.kind === "in_house_update";
            const mine = !fromDesk && lastSent.current !== null && c.body === lastSent.current;
            const speaker = fromDesk ? "An update from your service desk" : mine ? "You" : null;
            return (
              <article className={`portal-msg${mine ? " portal-msg--mine" : ""}`} key={c.id}>
                <span className="portal-avatar portal-avatar--sm" aria-hidden>
                  {initials(speaker === "You" ? "You" : "Service desk")}
                </span>
                <div className="portal-msg__body">
                  <div className="portal-msg__who">
                    {speaker !== null && <b>{speaker}</b>}
                    <span>{when(c.created_at)}</span>
                  </div>
                  <p className="portal-msg__text" style={{ margin: 0 }}>
                    {c.body}
                  </p>
                </div>
              </article>
            );
          })}
          {pending !== null && (
            <article className="portal-msg portal-msg--mine" data-testid="portal-pending-reply">
              <span className="portal-avatar portal-avatar--sm" aria-hidden>
                {initials("You")}
              </span>
              <div className="portal-msg__body">
                <div className="portal-msg__who">
                  <b>You</b>
                  <span>Sending…</span>
                </div>
                <p className="portal-msg__text" style={{ margin: 0 }}>
                  {pending}
                </p>
              </div>
            </article>
          )}
        </div>
      )}

      {nothingYet && <TicketProgress unassigned />}

      {deskSaidSomething && (
        <Note>
          You&apos;ll get an email the moment your service desk replies, and the reply will appear
          here too.
        </Note>
      )}

      {/* The reply box: a plain textarea, who sees it, and Send — which stays
          disabled until there is something to send. */}
      <form onSubmit={submitReply}>
        <div className="portal-composer">
          <label htmlFor="portal-reply" className="sr-only">
            Your reply
          </label>
          <textarea
            id="portal-reply"
            value={reply}
            onChange={(e) => setReply(e.target.value)}
            placeholder={
              nothingYet
                ? "Add anything that would help us, such as what you have already tried."
                : "Reply to your service desk"
            }
            aria-describedby="portal-reply-who"
            aria-invalid={replyError !== null}
          />
          {replyError !== null && (
            <p className="portal-field__error" role="alert">
              <PortalIcon name="alert" size={14} />
              <span>{replyError}</span>
            </p>
          )}
          <div className="portal-composer__row">
            <span className="portal-hint" id="portal-reply-who">
              Only you and your service desk see this.
            </span>
            <span style={{ flex: 1 }} />
            <button
              type="submit"
              className="portal-btn"
              disabled={sending || reply.trim().length === 0}
              data-testid="portal-send-reply"
            >
              {sending ? "Sending…" : "Send"}
            </button>
          </div>
          <p className="portal-hint" style={{ margin: 0 }}>
            Need to send a file? Email your service desk and it arrives on this ticket. Anything
            they attach shows below.
          </p>
        </div>
      </form>

      {attachments !== null && attachments.length > 0 && (
        <PortalCard title="Files" testId="portal-attachments">
          {attachments.map((att) => (
            <div className="portal-row" key={att.id}>
              <span className="portal-row__title">
                <b>{att.filename}</b>
                <span>{Math.max(1, Math.round(att.size_bytes / 1024))} KB</span>
              </span>
              <button
                type="button"
                className="portal-btn portal-btn--secondary portal-btn--small"
                onClick={() => {
                  void portalApi.ticketAttachmentUrl(t.id, att.id).then((url) => {
                    const a = document.createElement("a");
                    a.href = url;
                    a.download = att.filename;
                    document.body.appendChild(a);
                    a.click();
                    a.remove();
                    window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
                  });
                }}
              >
                <PortalIcon name="download" size={16} />
                Download
              </button>
            </div>
          ))}
        </PortalCard>
      )}
    </>
  );
}
