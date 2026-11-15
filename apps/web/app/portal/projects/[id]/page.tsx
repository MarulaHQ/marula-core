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
import { useEffect, useState } from "react";
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

  useEffect(() => {
    getPortalProject(params.id)
      .then(setProject)
      .catch((e: Error) => setError(e.message));
  }, [params.id]);

  async function onAcknowledge(changeOrderId: string) {
    setBusy(changeOrderId);
    setError(null);
    try {
      await acknowledgeChangeOrder(params.id, changeOrderId);
      setProject(await getPortalProject(params.id));
      setNote("Thank you. Your project manager has been told.");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  }

  if (error !== null && project === null) {
    return (
      <div className="portal-page">
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
      <div className="portal-page">
        <p className="portal-hint">Loading the project…</p>
      </div>
    );
  }

  const waiting = project.change_orders.filter((order) => order.acknowledged_at === null);

  return (
    <div className="portal-page">
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

      {project.hold_reason !== null ? (
        <PortalCard title="Work is paused">
          <p className="portal-hint">{project.hold_reason}</p>
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
                className="portal-btn portal-btn--primary"
                disabled={busy === order.id}
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
              <span className="portal-pill portal-pill--open">{step.status_text}</span>
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
              <pre className="portal-prose">{update.body}</pre>
            </details>
          ))
        )}
      </PortalCard>
    </div>
  );
}
