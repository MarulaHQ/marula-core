// /s/[token] — a scheduling link, opened by somebody outside the service desk.
//
// Purpose: the customer-facing half of a scheduling link. The tech mints a
// link from a ticket and hands out `{origin}/s/{token}`; this page resolves it
// through the unauthenticated public endpoint, shows what the meeting is, and
// books a time.
//
// THE DERIVATION, AND ITS ONE RULE. DERIVATION-GUIDE.md lines 90-91 send this
// route and /share to the KIT with no screen, which leaves one instruction:
// no chrome, no portal header, no staff nav. So there is no portal header and
// no staff nav here — only the meeting itself, on a plain card, with the
// service desk's name on it and nothing else. The only brand on the page is the
// desk's, because the person holding this link has never heard of Marula and
// the service desk is who they are dealing with.
//
// THE EXPIRY IS A FACT, NOT A FEAR. It is drawn in plain words, at the bottom,
// with the date and the time. The ONE coloured element anywhere on these
// surfaces is a blush pill when the link is inside a day of expiring — and even
// then the pill carries the words "expires in 6 hours", never a countdown that
// nags. An expiry a person cannot act on is noise; an expiry with a date is
// something to plan around.
//
// Non-purpose: no account, no ticket data, no staff context, and nothing about
// the ticket this link came from beyond the meeting's own shape.
"use client";

import type { components } from "@marula-core/sdk";
import { useParams } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Spinner } from "@marula-core/ui";
import { applyEntryTheme } from "../../../lib/entry-theme";
import { api } from "../../../lib/api";
import { portalOrgName } from "../../../lib/portal-branding";

type PublicLinkView = components["schemas"]["PublicLinkView"];

const SLOT_CHUNK_MINUTES = 30;

/** Typed error envelope bodies arrive untyped here (responses carry no
 *  content schema); the codes map to friendly public copy. */
function friendlyLinkError(error: unknown): string {
  const body = error as { error?: { code?: number; message?: string } } | undefined;
  const code = body?.error?.code;
  if (code === 409) {
    const message = body?.error?.message ?? "";
    const text = message.toLowerCase();
    if (text.includes("already been used")) {
      return "This link has already been booked. If this is your ticket, nothing more is needed.";
    }
    if (text.includes("revoked")) return "The sender withdrew this link. Ask them for a new one.";
    if (text.includes("expired")) return "This link has expired. Ask for a fresh one and it will work.";
    return "This link is no longer bookable.";
  }
  if (code === 429) return "Too many attempts. Wait a minute and try again.";
  return "This link is not available.";
}

function fmtTime(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
}

function fmtDay(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const delta = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() -
      new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime()) /
      86_400_000,
  );
  if (delta === 0) return "Today";
  if (delta === 1) return "Tomorrow";
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
}

/** "6 hours", "2 days" — the shape of a time left, in words. */
function untilText(iso: string, now: Date = new Date()): string {
  const ms = new Date(iso).getTime() - now.getTime();
  if (ms <= 0) return "expired";
  const hours = Math.floor(ms / 3_600_000);
  if (hours < 1) return `${Math.max(1, Math.floor(ms / 60_000))} minutes`;
  if (hours < 48) return `${hours} ${hours === 1 ? "hour" : "hours"}`;
  return `${Math.round(hours / 24)} days`;
}

function ExpiryPill({ expiresAt }: { expiresAt: string }) {
  const left = untilText(expiresAt);
  // The one coloured element on this surface, and only inside a day.
  const soon = !left.endsWith("days") && !left.endsWith("expired");
  return (
    <span className={soon ? "share-pill share-pill--blush" : "share-pill"}>
      {soon ? `Expires in ${left}` : `Expires ${new Date(expiresAt).toLocaleString()}`}
    </span>
  );
}

export default function PublicBookingPage(): ReactNode {
  const params = useParams<{ token: string }>();
  const token = params?.token ?? "";
  const desk = portalOrgName() ?? "Your service desk";

  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyEntryTheme();
  }, []);

  const [view, setView] = useState<PublicLinkView | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "error" | "booked">("loading");
  const [errorText, setErrorText] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
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
          setErrorText("This link is not available.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [token]);

  /** Slices each free interval into bookable half-hour choices, by day. */
  const byDay = useMemo(() => {
    if (view === null) return [];
    const all: string[] = [];
    for (const slot of view.slots ?? []) {
      const startMs = new Date(slot.starts_at).getTime();
      const endMs = new Date(slot.ends_at).getTime();
      for (let t = startMs; t + SLOT_CHUNK_MINUTES * 60_000 <= endMs; t += SLOT_CHUNK_MINUTES * 60_000) {
        all.push(new Date(t).toISOString());
      }
    }
    all.sort((a, b) => (a < b ? -1 : 1));
    const groups = new Map<string, string[]>();
    for (const iso of all.slice(0, 60)) {
      const key = fmtDay(iso);
      groups.set(key, [...(groups.get(key) ?? []), iso]);
    }
    return [...groups.entries()];
  }, [view]);

  async function book(): Promise<void> {
    if (selected === null) return;
    setBusy(true);
    setErrorText(null);
    const { error } = await api.api.POST("/api/v1/public/scheduling-links/{token}/book", {
      params: { path: { token } },
      body: {
        starts_at: selected,
        booker_name: name.trim() === "" ? null : name.trim(),
        booker_email: email.trim() === "" ? null : email.trim(),
      },
    });
    setBusy(false);
    if (error !== undefined) {
      setErrorText("That time has just been taken. Pick another and try again.");
      return;
    }
    setState("booked");
  }

  if (state === "loading") {
    return (
      <div className="share-root">
        <Spinner aria-label="Opening the link" />
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="share-root">
        <div className="share-card">
          <h1 className="share-card__h">This link isn&apos;t available.</h1>
          <p className="share-card__p">{errorText}</p>
          <p className="share-card__p" style={{ color: "var(--m-text-secondary)" }}>
            Ask {desk} for a new one and it will arrive in a moment.
          </p>
        </div>
      </div>
    );
  }

  if (state === "booked" || view === null) {
    return (
      <div className="share-root">
        <div className="share-card">
          <span
            aria-hidden
            style={{
              display: "flex",
              width: 48,
              height: 48,
              alignItems: "center",
              justifyContent: "center",
              borderRadius: 999,
              background: "var(--m-status-success-bg)",
              color: "var(--m-status-success-fg)",
            }}
          >
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" aria-hidden>
              <path d="M20 6 9 17l-5-5" />
            </svg>
          </span>
          <h1 className="share-card__h">Booked.</h1>
          <p className="share-card__p">
            {selected !== null && `${fmtDay(selected)} at ${fmtTime(selected)}`}. {desk} has it, and
            you&apos;ll get a confirmation by email.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="share-root">
      <div className="share-card" data-testid="public-booking">
        {/* The meeting, read-only. This is the whole page: what it is, how long,
            and who is running it. */}
        <div>
          <span className="share-desk">{desk}</span>
          <h1 className="share-card__h">Book a time with us.</h1>
          <p className="share-card__p">
            {view.duration_minutes} minutes. Pick a time that suits you, and we&apos;ll send a
            confirmation and a joining link by email.
          </p>
        </div>

        {view.is_expired === true ? (
          <div className="share-note">
            <span>
              This link has expired. Ask {desk} for a fresh one &mdash; it takes them a moment.
            </span>
          </div>
        ) : byDay.length === 0 ? (
          <div className="share-note">
            <span>
              Nothing is free in the next {view.booking_window_days} days. Ask {desk} for a time
              that works.
            </span>
          </div>
        ) : (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void book();
            }}
            style={{ display: "flex", flexDirection: "column", gap: 18 }}
          >
            {byDay.map(([day, times]) => (
              <div key={day} className="share-day">
                <span className="share-day__label">{day}</span>
                <div className="share-choices" role="radiogroup" aria-label={day}>
                  {times.map((iso) => (
                    <button
                      key={iso}
                      type="button"
                      role="radio"
                      aria-checked={selected === iso}
                      className="share-choice"
                      onClick={() => setSelected(iso)}
                      data-testid={`share-slot-${iso}`}
                    >
                      {fmtTime(iso)}
                    </button>
                  ))}
                </div>
              </div>
            ))}

            <div className="share-fields">
              <div className="entry-field">
                <label className="entry-label" htmlFor="share-name">
                  Your name
                </label>
                <input
                  id="share-name"
                  className="entry-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  autoComplete="name"
                />
              </div>
              <div className="entry-field">
                <label className="entry-label" htmlFor="share-email">
                  Your email
                </label>
                <input
                  id="share-email"
                  className="entry-input"
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  autoComplete="email"
                />
              </div>
            </div>
            <p className="entry-hint" style={{ margin: 0 }}>
              Optional, but it is how the confirmation reaches you.
            </p>

            {errorText !== null && (
              <p className="entry-error" role="alert">
                {errorText}
              </p>
            )}

            <button
              type="submit"
              className="entry-btn"
              disabled={busy || selected === null}
              data-testid="share-book"
            >
              {busy ? "Booking…" : "Book this time"}
            </button>
          </form>
        )}

        {/* The expiry, last, in plain words, as a fact to plan around. */}
        {view.expires_at !== null && view.expires_at !== undefined && (
          <div className="share-expiry">
            {view.is_expired === true ? (
              <span>Expired {new Date(view.expires_at).toLocaleString()}</span>
            ) : (
              <ExpiryPill expiresAt={view.expires_at} />
            )}
            <span style={{ color: "var(--m-text-secondary)" }}>
              This link stops working then. After that, ask {desk} for another.
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
