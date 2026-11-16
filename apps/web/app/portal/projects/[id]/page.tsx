// /portal/projects/[id] — one project, in the client's language (S16.1, S16.2,
// S16.3, S16.4, S16.5, S16.6, S16.7, S16.8).
//
// Four steps instead of eighteen tasks. A held project gives the reason the PM
// marked safe to show. A change order waiting for an answer is the one
// highlighted block, and its amount is shown as the agreed number, never as a
// rate or a margin.
"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { useRequirePortalAuth } from "../../../../lib/portal-api";
import { longDate } from "../../../../lib/portal-copy";
import { useTimeFormat } from "../../../../lib/use-time-format";
import { PortalCard, PortalEmpty } from "../../../../components/portal/portal-ui";
import {
  acknowledgeChangeOrder,
  getPortalProject,
  type PortalProjectDetail,
} from "../portal-projects-api";

function money(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(minor / 100);
}

export default function PortalProjectPage() {
  useRequirePortalAuth();
  const { timeZone } = useTimeFormat();
  const params = useParams<{ id: string }>();
  const [project, setProject] = useState<PortalProjectDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  // The route id the in-flight work belongs to. A slower answer for project A
  // must not paint over project B.
  const currentId = useRef(params.id);
  currentId.current = params.id;

  useEffect(() => {
    let cancelled = false;
    // Clear the previous project, its error and its note before the next load:
    // otherwise A's page stands under B's URL.
    setProject(null);
    setError(null);
    setNote(null);
    getPortalProject(params.id)
      .then((loaded) => {
        if (!cancelled) setProject(loaded);
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [params.id]);

  async function onAcknowledge(changeOrderId: string) {
    const projectId = params.id;
    // The acknowledgement is sent with the project id on screen, so it is only
    // offered once that project is the one loaded.
    if (project === null || project.id !== projectId) return;
    setBusy(changeOrderId);
    setError(null);
    setNote(null);
    try {
      await acknowledgeChangeOrder(projectId, changeOrderId);
    } catch (e) {
      if (currentId.current === projectId) {
        setBusy(null);
        setError((e as Error).message);
      }
      return;
    }
    setBusy(null);
    if (currentId.current !== projectId) return;
    // The server has the answer; the page must say so even if the refresh
    // below fails, so the acknowledgement and the reload are separate steps.
    setNote("Thank you. Your project manager has been told.");
    // Reflect the answer locally too: a failed refresh must not leave the order
    // waiting for a second "I agree".
    setProject((loaded) =>
      loaded === null
        ? loaded
        : {
            ...loaded,
            change_orders: loaded.change_orders.map((order) =>
              order.id === changeOrderId
                ? { ...order, acknowledged_at: new Date().toISOString() }
                : order,
            ),
          },
    );
    try {
      const refreshed = await getPortalProject(projectId);
      if (currentId.current === projectId) setProject(refreshed);
    } catch {
      // The acknowledgement stands; the list catches up on the next load.
    }
  }

  if (error !== null && project === null) {
    return (
      <div>
        <PortalCard title="We could not load this project">
          <p className="portal-hint" role="alert">
            {error}
          </p>
          <p className="portal-hint">
            <Link href="/portal/projects">Back to your projects</Link>
          </p>
        </PortalCard>
      </div>
    );
  }

  if (project === null) {
    return (
      <div>
        <p className="portal-hint">Loading the project…</p>
      </div>
    );
  }

  const waiting = project.change_orders.filter((order) => order.acknowledged_at === null);

  return (
    <div>
      <p className="portal-back">
        <Link href="/portal/projects">← All projects</Link>
      </p>

      <div className="portal-heading-row">
        <div>
          <h1 className="portal-title">{project.name}</h1>
          <p className="portal-lead">
            {project.status_text}
            {project.due_date === null ? "" : `, with completion planned for ${longDate(project.due_date, timeZone)}`}
            .
          </p>
        </div>
      </div>

      {!project.plan_shared ? (
        <PortalCard title="This project is just getting started">
          <p className="portal-hint">
            We are putting the plan together. You will see the steps, the dates and the
            first update here as soon as it is shared — your contact will tell you when.
          </p>
        </PortalCard>
      ) : null}

      {project.status === "on_hold" ? (
        <PortalCard title="Work is paused" testId="portal-hold">
          <p className="portal-hint">{project.hold_reason ?? "Work is paused."}</p>
        </PortalCard>
      ) : null}

      {waiting.length > 0 ? (
        <PortalCard title="We need your answer" testId="portal-change-orders">
          {waiting.map((order) => (
            <div key={order.id} className="portal-row portal-row--waiting">
              <span className="portal-row__title">
                <b>
                  {order.reference} · {order.title}
                </b>
                <span>{order.summary ?? "A change to the agreed scope."}</span>
                <span className="portal-hint">
                  {order.amount_minor === null
                    ? "No change to the amount"
                    : `${money(order.amount_minor, order.currency ?? "USD")}`}
                  {order.changes_date ? " · this moves the completion date" : ""}
                </span>
              </span>
              <button
                type="button"
                className="portal-btn"
                disabled={busy === order.id || project.id !== params.id}
                onClick={() => void onAcknowledge(order.id)}
              >
                {busy === order.id ? "Sending…" : "I agree"}
              </button>
            </div>
          ))}
          {note !== null ? <p className="portal-hint">{note}</p> : null}
        </PortalCard>
      ) : null}

      {error !== null ? (
        <p className="portal-hint" role="alert">
          {error}
        </p>
      ) : null}

      <PortalCard title="How it is going">
        {project.steps.length === 0 ? (
          <PortalEmpty title="No steps shared yet">
            The plan has not been shared. Nothing here is a promise until it is.
          </PortalEmpty>
        ) : (
          project.steps.map((step) => (
            <div key={step.label} className="portal-row">
              <span className="portal-row__title">
                <b>{step.label}</b>
                <span>
                  {step.start_date === null || step.end_date === null
                    ? "Dates to be confirmed"
                    : `${longDate(step.start_date, timeZone)} – ${longDate(step.end_date, timeZone)}`}
                </span>
              </span>
              <span className="portal-pill portal-pill--working">{step.status_text}</span>
            </div>
          ))
        )}
      </PortalCard>

      <PortalCard title="Updates">
        {project.updates.length === 0 ? (
          <p className="portal-hint">
            No update yet. You will get one when something moves.
          </p>
        ) : (
          project.updates.map((update, index) => (
            <details key={update.id} className="portal-row" open={index === 0}>
              <summary className="portal-row__title">
                <b>
                  {update.sent_at === null
                    ? "Update"
                    : longDate(update.sent_at.slice(0, 10), timeZone)}
                </b>
                <span>{index === 0 ? "Latest" : "Earlier update"}</span>
              </summary>
              <pre style={{ whiteSpace: "pre-wrap", margin: 0 }}>{update.body}</pre>
            </details>
          ))
        )}
      </PortalCard>
    </div>
  );
}
