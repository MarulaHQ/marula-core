// /portal/tickets — the client's list, in plain words.
//
// Purpose: everything she has asked for, and where it has got to. The list is
// card rows (the pack's `.pr` row inside a `.cardx` card), split into Open and
// Recently resolved, and every row is worded for a client rather than for the
// service desk (lib/portal-copy.ts owns the words).
//
// WHAT NEVER APPEARS HERE: a priority, a board, an assignee, an SLA clock, an
// internal status, a due date on her own list, or a "new request" button that
// the portal API cannot honour. The pack's heading carries a primary "Raise a
// request"; the pinned portal API has no create-ticket operation, so a primary
// that cannot raise a request is a dead button and is not drawn. The honest
// route to raising one is the service desk, which the footer already names.
//
// "Waiting for you" IS THE ONLY HIGHLIGHTED ROW. It carries the gold tint and
// the gold edge bar, because it is the one state that needs an answer from her
// (DESIGN-NOTES-portal.md §3).
//
// Non-purpose: no bulk actions, no filters, no export, no views switch. A
// client list of a handful of rows does not need a data toolbar, and the pack
// does not draw one.
"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth } from "../../../lib/portal-api";
import { CLIENT_STATUS, clientStatusFor, longDate } from "../../../lib/portal-copy";
import {
  PortalCard,
  PortalEmpty,
  PortalIcon,
  PortalRow,
  StatusPill,
  ticketNote,
} from "../../../components/portal/portal-ui";

interface Row {
  id: string;
  number: string;
  title: string;
  category: "open" | "in_progress" | "waiting" | "closed";
  updated_at: string;
  resolved_at?: string | null;
}

export default function PortalTicketsPage() {
  useRequirePortalAuth();
  const router = useRouter();
  const [rows, setRows] = useState<Row[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [csatTicket, setCsatTicket] = useState<{ id: string; number: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listTickets()
      .then((tickets) => {
        if (cancelled) return;
        setRows(
          tickets.map((t) => ({
            id: t.id,
            number: t.number,
            title: t.title,
            category: t.category,
            updated_at: t.updated_at,
            resolved_at: t.resolved_at,
          })),
        );
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "We couldn't load your tickets. Try again in a moment."));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const { open, resolved } = useMemo(() => {
    if (rows === null) return { open: [] as Row[], resolved: [] as Row[] };
    return {
      open: rows.filter((r) => r.category !== "closed"),
      resolved: rows.filter((r) => r.category === "closed"),
    };
  }, [rows]);

  if (error !== null) {
    return (
      <div className="portal-card">
        <p className="portal-field__error" role="alert" style={{ padding: 18 }}>
          <PortalIcon name="alert" size={14} />
          <span>{error}</span>
        </p>
      </div>
    );
  }

  if (rows === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading your tickets" />
      </div>
    );
  }

  // The pack's state 11: a drawing, one sentence, one action. A client with
  // nothing open is not an error, and the sentence says where replies appear.
  if (rows.length === 0) {
    return (
      <>
        <div>
          <h1 className="portal-title">Your tickets</h1>
          <p className="portal-lead">Everything you've asked us for, and where it has got to.</p>
        </div>
        <PortalCard>
          <PortalEmpty
            title="Nothing open right now"
            action={
              <a className="portal-btn" href="mailto:support@brightline.example" style={{ marginTop: 8 }}>
                Contact your service desk
              </a>
            }
          >
            When you need help, contact your service desk and we&apos;ll reply right here. You
           &apos;ll get an email each time.
          </PortalEmpty>
        </PortalCard>
      </>
    );
  }

  return (
    <>
      <div className="portal-heading-row">
        <div>
          <h1 className="portal-title">Your tickets</h1>
          <p className="portal-lead">Everything you&apos;ve asked us for, and where it has got to.</p>
        </div>
      </div>

      <PortalCard
        title="Open"
        aside={<span className="portal-hint">{open.length} tickets</span>}
        testId="portal-tickets-open"
      >
        {open.length === 0 ? (
          <div className="portal-empty" style={{ padding: "32px 24px" }}>
            <p className="portal-hint">Nothing open. Anything you raise will show up here.</p>
          </div>
        ) : (
          open.map((t) => {
            const status = clientStatusFor(t.category);
            return (
              <PortalRow
                key={t.id}
                title={t.title}
                meta={`#${t.number} · ${ticketNote(t.category)}`}
                highlight={status === "waiting"}
                onClick={() => router.push(`/portal/tickets/${t.id}`)}
                trailing={<StatusPill status={status} />}
                testId={`portal-ticket-${t.number}`}
              />
            );
          })
        )}
      </PortalCard>

      {resolved.length > 0 && (
        <PortalCard title="Recently resolved" testId="portal-tickets-resolved">
          {resolved.map((t) => (
            <PortalRow
              key={t.id}
              title={t.title}
              meta={`#${t.number} · ${CLIENT_STATUS.resolved.note}`}
              onClick={() => router.push(`/portal/tickets/${t.id}`)}
              trailing={
                <>
                  <StatusPill status="resolved" />
                  <button
                    type="button"
                    className="portal-btn portal-btn--secondary portal-btn--small"
                    onClick={() => setCsatTicket({ id: t.id, number: t.number })}
                    data-testid={`portal-csat-open-${t.number}`}
                  >
                    How did we do?
                  </button>
                </>
              }
            />
          ))}
        </PortalCard>
      )}

      {csatTicket !== null && (
        <div className="portal-scrim" role="presentation" onClick={() => setCsatTicket(null)}>
          <div
            className="portal-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="How did we do?"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="portal-dialog__hd">
              <div>
                <h2>How did we do?</h2>
                <p>
                  Ticket #{csatTicket.number} · {longDate(new Date().toISOString())}
                </p>
              </div>
            </div>
            <div className="portal-dialog__mb">
              <p className="portal-hint" style={{ margin: 0 }}>
                Five choices, in words. It takes a few seconds, and it goes to the person who did
                the work.
              </p>
              <Link
                className="portal-btn portal-btn--full"
                href={`/portal/csat?ticket=${csatTicket.id}`}
                onClick={() => setCsatTicket(null)}
              >
                Rate the work
              </Link>
            </div>
            <div className="portal-dialog__mf">
              <span className="portal-hint">Not now is fine too.</span>
              <button
                type="button"
                className="portal-btn portal-btn--secondary portal-btn--small"
                onClick={() => setCsatTicket(null)}
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
