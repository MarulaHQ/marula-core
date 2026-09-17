// /s/[token] — the public scheduling-link booking page.
//
// Purpose: the customer-facing half of the ticket scheduling-link flow
// (Wave 11 lane 4 backend + lane 6 send UI). The tech mints a link from a
// ticket and hands out `{origin}/s/{token}`; this page resolves the token
// through the unauthenticated public endpoints (view + book), shows the
// meeting shape and the free-interval preview, and books a slot. A
// completed booking lands back on the originating ticket. Typed 409s
// (revoked / used / expired / slot-taken) render as friendly states —
// never raw errors. Stamps the tech-visible link lifecycle (first view).
// Non-purpose: no staff surfaces, no auth, no branding beyond Workbench
// tokens.
//
// Wave 17 lane 8 UX pass: the raw <select> booking picker becomes the
// date-grouped slot-pill pattern (date rows, one tap per slot, explicit
// selection summary before confirm); brand lockup; entrance motion.
"use client";

import type { components } from "@meridian/sdk";
import { Button, Input, Label, Spinner, StatusTag } from "@meridian/ui";
import { useParams } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../../lib/api";
import { applyTheme } from "@meridian/ui";

type PublicLinkView = components["schemas"]["PublicLinkView"];
type SlotInterval = components["schemas"]["SlotInterval"];

const SLOT_CHUNK_MINUTES = 30;

/** Typed error envelope bodies arrive untyped here (responses carry no
 * content schema); the codes map to friendly public copy. */
function friendlyLinkError(error: unknown): string {
  const body = error as { error?: { code?: number; message?: string } } | undefined;
  const code = body?.error?.code;
  if (code === 409) {
    const message = body?.error?.message ?? "";
    if (message.toLowerCase().includes("already been used")) return "This link has already been booked. If this is your ticket, no further action is needed.";
    if (message.toLowerCase().includes("revoked")) return "This scheduling link was revoked by the sender.";
    if (message.toLowerCase().includes("expired")) return "This scheduling link has expired. Please ask for a fresh one.";
    return "This scheduling link is no longer bookable.";
  }
  if (code === 429) return "Too many attempts — please wait a minute and try again.";
  return "This scheduling link is not available.";
}

function fmtTime(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function fmtDay(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const local = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const delta = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() - local.getTime()) / 86_400_000,
  );
  if (delta === 0) return "Today";
  if (delta === 1) return "Tomorrow";
  return d.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

export default function PublicBookingPage(): ReactNode {
  const params = useParams<{ token: string }>();
  const token = params?.token ?? "";

  useEffect(() => {
    // Public page: always render in the customer theme (light), never
    // inherit a staff member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  const [view, setView] = useState<PublicLinkView | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "error" | "booked">("loading");
  const [errorText, setErrorText] = useState<string | null>(null);
  const [bookerName, setBookerName] = useState("");
  const [bookerEmail, setBookerEmail] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (token === "") return;
    let cancelled = false;
    api.api
      .GET("/api/v1/public/scheduling-links/{token}", { params: { path: { token } } })
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error !== undefined || data === undefined) {
          setState("error");
          setErrorText(friendlyLinkError(error));
          return;
        }
        setView(data);
        setState("ready");
      })
      .catch(() => {
        if (!cancelled) {
          setState("error");
          setErrorText("This scheduling link is not available.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [token]);

  /** Slices each free interval into bookable 30-minute choices, grouped
   * by day. */
  const choices = useMemo(() => {
    if (view === null) return [];
    const all: { start: string; label: string }[] = [];
    for (const slot of view.slots ?? []) {
      const startMs = new Date(slot.starts_at).getTime();
      const endMs = new Date(slot.ends_at).getTime();
      for (let t = startMs; t + SLOT_CHUNK_MINUTES * 60_000 <= endMs; t += SLOT_CHUNK_MINUTES * 60_000) {
        all.push({
          start: new Date(t).toISOString(),
          label: new Date(t).toLocaleString(undefined, {
            weekday: "short",
            month: "short",
            day: "numeric",
            hour: "numeric",
            minute: "2-digit",
          }),
        });
      }
    }
    all.sort((a, b) => (a.start < b.start ? -1 : 1));
    return all.slice(0, 60);
  }, [view]);

  const byDay = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const c of choices) {
      const key = fmtDay(c.start);
      map.set(key, [...(map.get(key) ?? []), c.start]);
    }
    return [...map.entries()];
  }, [choices]);

  async function book(): Promise<void> {
    if (selected === null) return;
    setBusy(true);
    setErrorText(null);
    const { error } = await api.api.POST("/api/v1/public/scheduling-links/{token}/book", {
      params: { path: { token } },
      body: {
        starts_at: selected,
        booker_name: bookerName.trim() === "" ? null : bookerName.trim(),
        booker_email: bookerEmail.trim() === "" ? null : bookerEmail.trim(),
      },
    });
    setBusy(false);
    if (error !== undefined) {
      setErrorText("That time is no longer free — please pick another slot.");
      return;
    }
    setState("booked");
  }

  if (state === "loading") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner aria-label="Loading booking page" />
      </main>
    );
  }

  if (state === "error") {
    return (
      <PublicShell>
        <div className="flex">
          <StatusTag tone="warning">Unavailable</StatusTag>
        </div>
        <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
          {errorText}
        </p>
      </PublicShell>
    );
  }

  if (state === "booked" || (view !== null && view.is_expired)) {
    return (
      <PublicShell>
        <div className="flex">
          <StatusTag tone="success">Booked</StatusTag>
        </div>
        <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
          {state === "booked"
            ? "Your meeting is booked. A confirmation is on its way, and it is linked to your ticket."
            : "This scheduling link has expired. Please ask your contact for a fresh one."}
        </p>
      </PublicShell>
    );
  }

  return (
    <PublicShell>
      <div className="grid gap-1">
        <h1 className="font-heading text-lg font-semibold tracking-tight">{view?.meeting_type_name}</h1>
        <p className="text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          {view?.duration_minutes} minutes
          {view?.label !== null && view?.label !== undefined ? ` · ${view.label}` : null}
        </p>
        {view?.meeting_type_description !== null && view?.meeting_type_description !== undefined && (
          <p className="text-xs leading-5" style={{ color: "var(--m-text-secondary)" }}>
            {view.meeting_type_description}
          </p>
        )}
      </div>

      <div className="grid gap-3 max-h-72 overflow-y-auto pr-1" data-testid="public-slot-grid">
        {choices.length === 0 ? (
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            No open times in the booking window yet — check back soon.
          </p>
        ) : (
          byDay.map(([day, times]) => (
            <div key={day}>
              <h3
                className="mb-1.5 text-xs font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                {day}
              </h3>
              <ul className="flex flex-wrap gap-2">
                {times.map((start) => {
                  const active = selected === start;
                  return (
                    <li key={start}>
                      <button
                        type="button"
                        onClick={() => setSelected(start)}
                        aria-pressed={active}
                        className="rounded-[0.75rem] border px-2.5 py-1.5 text-xs tabular-nums transition-colors duration-100 hover:bg-[var(--m-surface-hover)]"
                        style={{
                          borderColor: active ? "var(--m-accent)" : "var(--m-border)",
                          color: active ? "var(--m-accent)" : "var(--m-text-secondary)",
                          background: "var(--m-surface)",
                        }}
                      >
                        {fmtTime(start)}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))
        )}
      </div>

      <div className="grid gap-2 sm:grid-cols-2">
        <div className="grid gap-1">
          <Label htmlFor="book-name">Your name (optional)</Label>
          <Input id="book-name" value={bookerName} onChange={(e) => setBookerName(e.target.value)} />
        </div>
        <div className="grid gap-1">
          <Label htmlFor="book-email">Your email (optional)</Label>
          <Input id="book-email" type="email" value={bookerEmail} onChange={(e) => setBookerEmail(e.target.value)} />
        </div>
      </div>

      {errorText !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
          {errorText}
        </p>
      )}

      {selected !== null && (
        <p className="m-rise text-sm" data-testid="public-slot-summary" role="status">
          <span className="font-medium">Selected:</span>{" "}
          {fmtDay(selected)}, {fmtTime(selected)}
        </p>
      )}

      <div className="flex justify-end">
        <Button
          variant="primary"
          disabled={busy || selected === null}
          onClick={() => void book()}
        >
          {busy ? "Booking…" : selected !== null ? `Book ${fmtTime(selected)}` : "Pick a time to book"}
        </Button>
      </div>
    </PublicShell>
  );
}

function PublicShell({ children }: { children: ReactNode }): ReactNode {
  return (
    <main
      className="flex min-h-full items-center justify-center p-6"
      data-testid="public-booking"
    >
      <div className="m-rise w-full max-w-md">
        <div className="mb-3 text-center">
          <span className="font-heading text-lg font-semibold tracking-tight">Meridian</span>
        </div>
        <div
          className="grid gap-4 rounded-[0.75rem] border p-5"
          style={{ background: "var(--m-surface)", borderColor: "var(--m-border)" }}
        >
          {children}
        </div>
      </div>
    </main>
  );
}
