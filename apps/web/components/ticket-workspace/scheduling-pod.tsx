// Scheduling-link send pod — Calendly-style scheduling from a ticket.
//
// Purpose: send the client a scheduling request from the ticket — either
// the meeting-type's template auto-email (backend-owned, fires on link
// create when the template enables it) and/or a shareable link the tech
// can attach to their own message. The raw token arrives EXACTLY ONCE in
// the create response and is never stored server-side, so the UI surfaces
// a copy-once dialog. Lifecycle (created → viewed → scheduled / expired /
// revoked) renders from the link rows; expiry is DERIVED from expires_at
// per the lane-4 contract. Completed bookings land back on this ticket
// (ticket_id is passed at create).
// Non-purpose: no template/group admin here (settings owns those).

"use client";

import type { components } from "@meridian/sdk";
import { Button, Input, Label, Spinner, StatusTag } from "@meridian/ui";
import { useEffect, useState, type ReactNode } from "react";

import { api } from "../../lib/api";

type MeetingType = components["schemas"]["MeetingType"];
type SchedulingLink = components["schemas"]["SchedulingLink"];

export interface SchedulingPodProps {
  ticketId: string;
  ticketTitle: string;
}

function derivedState(link: SchedulingLink): "active" | "viewed" | "scheduled" | "expired" | "revoked" {
  if (link.status === "revoked") return "revoked";
  if (link.status === "used") return "scheduled";
  if (link.expires_at !== null && link.expires_at !== undefined && new Date(link.expires_at) <= new Date()) {
    return "expired";
  }
  if (link.first_viewed_at !== null && link.first_viewed_at !== undefined) return "viewed";
  return "active";
}

const STATE_TONE: Record<string, "neutral" | "success" | "warning" | "danger"> = {
  active: "success",
  viewed: "neutral",
  scheduled: "success",
  expired: "warning",
  revoked: "danger",
};

export function SchedulingPod({ ticketId, ticketTitle }: SchedulingPodProps): ReactNode {
  const [types, setTypes] = useState<MeetingType[] | null>(null);
  const [links, setLinks] = useState<SchedulingLink[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [typeId, setTypeId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [freshToken, setFreshToken] = useState<{ url: string } | null>(null);
  const [copied, setCopied] = useState(false);

  async function reloadLinks(): Promise<void> {
    const { data, error: err } = await api.api.GET("/api/v1/scheduling-links", {
      params: { query: { ticket_id: ticketId } },
    });
    if (err !== undefined) {
      setError("Could not load scheduling links");
      setLinks([]);
      return;
    }
    setLinks(data ?? []);
  }

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [t, l] = await Promise.all([
        api.api.GET("/api/v1/meeting-types"),
        api.api.GET("/api/v1/scheduling-links", { params: { query: { ticket_id: ticketId } } }),
      ]);
      if (cancelled) return;
      if (t.data === undefined) setError("Could not load meeting types");
      else {
        const active = (t.data ?? []).filter((mt) => mt.is_active);
        setTypes(active);
        setTypeId(active[0]?.id ?? null);
      }
      setLinks(l.data === undefined ? [] : (l.data ?? []));
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ticketId]);

  async function createLink(): Promise<void> {
    if (typeId === null) return;
    setBusy(true);
    setError(null);
    const { data, error: err } = await api.api.POST("/api/v1/scheduling-links", {
      body: {
        meeting_type_id: typeId,
        ticket_id: ticketId,
        label: `Ticket ${ticketTitle}`.slice(0, 120),
      },
    });
    setBusy(false);
    if (err !== undefined || data === undefined) {
      setError("Could not create the scheduling link");
      return;
    }
    const bookingUrl = `${window.location.origin}/s/${data.token}`;
    setFreshToken({ url: bookingUrl });
    setCopied(false);
    void reloadLinks();
  }

  async function revoke(linkId: string): Promise<void> {
    setBusy(true);
    const { error: err } = await api.api.POST("/api/v1/scheduling-links/{id}/revoke", {
      params: { path: { id: linkId } },
    });
    setBusy(false);
    if (err !== undefined) {
      setError("Could not revoke the link");
      return;
    }
    void reloadLinks();
  }

  const selectedType = types?.find((t) => t.id === typeId) ?? null;

  return (
    <section
      className="grid content-start gap-2 rounded-[2px] border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid="pod-scheduling"
      aria-label="Scheduling"
    >
      <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        Scheduling
      </h2>
      {error !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{error}</p>
      )}
      {types === null || links === null ? (
        <Spinner />
      ) : (
        <>
          <div className="grid gap-1">
            <Label htmlFor="scheduling-type">Meeting type</Label>
            <div className="flex gap-2">
              <select
                id="scheduling-type"
                className="min-w-0 flex-1 rounded-[2px] border px-2 py-1 text-xs"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
                value={typeId ?? ""}
                onChange={(e) => setTypeId(e.target.value || null)}
                data-testid="scheduling-type"
              >
                {types.length === 0 && <option value="">No active meeting types</option>}
                {types.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name} · {t.duration_minutes} min
                  </option>
                ))}
              </select>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || typeId === null}
                onClick={() => void createLink()}
                data-testid="scheduling-create"
              >
                Create link
              </Button>
            </div>
            {selectedType !== null && selectedType.auto_email_enabled && (
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Creating sends the template auto-email to the client automatically (backend mail path).
              </p>
            )}
          </div>

          <ul className="grid gap-1" data-testid="scheduling-links">
            {links.map((link) => {
              const state = derivedState(link);
              return (
                <li
                  key={link.id}
                  className="grid gap-0.5 rounded-[2px] border p-2 text-xs"
                  style={{ borderColor: "var(--m-border)" }}
                  data-testid={`scheduling-link-${link.id}`}
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <StatusTag tone={STATE_TONE[state] ?? "neutral"}>{state}</StatusTag>
                    <span style={{ color: "var(--m-text-muted)" }}>
                      {link.view_count} view{link.view_count === 1 ? "" : "s"}
                    </span>
                    {link.scheduled_at !== null && link.scheduled_at !== undefined && (
                      <span style={{ color: "var(--m-status-success-fg)" }}>
                        booked {new Date(link.scheduled_at).toLocaleString()}
                      </span>
                    )}
                    <span className="ml-auto" style={{ color: "var(--m-text-muted)" }}>
                      created {new Date(link.created_at).toLocaleDateString()}
                    </span>
                    {state === "active" || state === "viewed" ? (
                      <Button size="sm" variant="ghost" disabled={busy} onClick={() => void revoke(link.id)}>
                        Revoke
                      </Button>
                    ) : null}
                  </div>
                </li>
              );
            })}
            {links.length === 0 && (
              <li className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                No scheduling links for this ticket yet.
              </li>
            )}
          </ul>
        </>
      )}

      {freshToken !== null && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center p-4"
          style={{ background: "var(--m-scrim)" }}
          role="dialog"
          aria-modal="true"
          aria-label="Shareable scheduling link"
          data-testid="scheduling-copy-once"
        >
          <div
            className="grid w-full max-w-md gap-3 rounded-[2px] border p-4"
            style={{ background: "var(--m-overlay-surface)", borderColor: "var(--m-border)" }}
          >
            <h3 className="text-sm font-semibold">Share this booking link</h3>
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Shown once — it cannot be retrieved again. Copy it now and attach it to your email to the client,
              or rely on the template auto-email if this meeting type has one enabled.
            </p>
            <div className="flex gap-2">
              <Input readOnly value={freshToken.url} onFocus={(e) => e.target.select()} data-testid="scheduling-token" />
              <Button
                size="sm"
                variant="primary"
                onClick={() => {
                  void navigator.clipboard.writeText(freshToken.url).then(() => setCopied(true));
                }}
                data-testid="scheduling-copy"
              >
                {copied ? "Copied" : "Copy"}
              </Button>
            </div>
            <div className="flex justify-end">
              <Button size="sm" onClick={() => setFreshToken(null)} data-testid="scheduling-copy-done">
                Done
              </Button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
