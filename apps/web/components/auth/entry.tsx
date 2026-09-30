// The entry surfaces: everything before the product.
//
// Purpose: the shared parts of the sign-in family, both wizards, and the
// welcome-back screen. The pack draws these as one system (DESIGN-NOTES-auth
// §2), so they are built once here rather than nine times.
//
// THE SCOPED EXCEPTION IS ENFORCED HERE, NOT JUST DOCUMENTED. The indigo and
// rust prints, the cream sticker outline on the fruit, and the serif headline
// exist in `entry.css` under `.entry-root`, and nowhere else. They are not
// tokens, no other file may reference those hexes, and nothing in this file
// may be imported by a surface that is not an entry screen. The product
// palette returns the moment someone is signed in.
//
// WHICH SCREEN GETS WHICH PATTERN, in one table so nobody guesses:
//   indigo — sign-in, the second step, a password, company sign-in, welcome back
//   rust   — a link on its way, a verification, an invitation
//   band   — the two wizards wear indigo as a 72px header, plain body
"use client";

import "./entry.css";
import { useEffect, useRef, useState } from "react";

export type EntryBackground = "indigo" | "rust" | "band" | "plain";

/** The wordmark, always on a solid pill so it never sits on a pattern. */
export function EntryWordmark() {
  return (
    <span className="entry-wordmark">
      <MarulaMark />
      marula
    </span>
  );
}

function MarulaMark() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden>
      <circle cx="12" cy="13" r="8" fill="var(--m-gold)" />
      <path d="M12 5V2" stroke="var(--m-canopy)" strokeWidth="1.8" strokeLinecap="round" />
      <path d="M12 3c2.5-3.5 8-4 10-2-2 3-7 4.5-10 2Z" fill="var(--m-canopy)" />
    </svg>
  );
}

/** A flat fruit, with a cream sticker outline so it holds on either pattern. */
export function EntryFruit({ half = false }: { half?: boolean }) {
  return half ? (
    <svg className="entry-fruit entry-fruit--half" viewBox="0 0 64 64" aria-hidden>
      <circle cx="32" cy="34" r="22" fill="var(--m-cream)" stroke="var(--m-ink)" strokeWidth="2.5" />
      <circle cx="32" cy="34" r="11" fill="var(--m-blush)" />
      <circle cx="32" cy="34" r="4" fill="var(--m-ink)" />
    </svg>
  ) : (
    <svg className="entry-fruit" viewBox="0 0 64 64" aria-hidden>
      <path
        d="M32 17V10"
        stroke="var(--m-bark)"
        strokeWidth="2.2"
        strokeLinecap="round"
        fill="none"
      />
      <path
        d="M32 11c6-9 19-10 26-5-6 8-18 11-26 5Z"
        fill="var(--m-canopy)"
        stroke="var(--m-cream)"
        strokeWidth="1.8"
        strokeLinejoin="round"
      />
      <circle cx="32" cy="38" r="22" fill="var(--m-gold)" stroke="var(--m-cream)" strokeWidth="2.5" />
      <ellipse cx="24" cy="31" rx="6" ry="4" fill="var(--m-cream)" opacity=".75" />
    </svg>
  );
}

/** The header. `band` puts it on the pattern; otherwise it sits on the canvas. */
export function EntryHeader({
  background,
  right,
}: {
  background: EntryBackground;
  right?: React.ReactNode;
}) {
  return (
    <header className={`entry-header${background === "band" ? " entry-header--band entry-root--pattern" : ""}`}>
      <EntryWordmark />
      <span className="entry-header__spacer" />
      {right}
    </header>
  );
}

export function EntryFooter() {
  return (
    <footer className="entry-footer">
      <div className="entry-footer__pill">
        <a href="/terms">Terms</a>
        <a href="/privacy">Privacy</a>
        <a href="mailto:support@brightline.example">Help</a>
      </div>
    </footer>
  );
}

/**
 * The whole sign-in frame: pattern, header, centred card, footer. The card is
 * the only place text sits, plus the wordmark and footer pills.
 */
export function EntryShell({
  background,
  children,
  fruit = "both",
  right,
}: {
  background: EntryBackground;
  children: React.ReactNode;
  fruit?: "both" | "none" | "half";
  right?: React.ReactNode;
}) {
  const rootClass =
    background === "indigo"
      ? "entry-root entry-root--pattern"
      : background === "rust"
        ? "entry-root entry-root--rust"
        : "entry-root";
  return (
    <div className={rootClass}>
      <EntryHeader background={background} right={right} />
      <main className="entry-main">
        <div className="entry-card-wrap">
          {fruit !== "none" && <EntryFruit />}
          {fruit === "both" && <EntryFruit half />}
          {children}
        </div>
      </main>
      <EntryFooter />
    </div>
  );
}

/** The card every entry screen sits in. */
export function EntryCard({
  children,
  testId,
}: {
  children?: React.ReactNode;
  testId?: string;
}) {
  return (
    <div className="entry-card" data-testid={testId}>
      {children}
    </div>
  );
}

/** The headline, in the brand's serif. The second half of the exception. */
export function EntryHeadline({ children }: { children: React.ReactNode }) {
  return <h1 className="entry-headline">{children}</h1>;
}

/** A shield disc, for the screens that protect. */
export function EntryShield() {
  return (
    <span className="entry-shield">
      <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinejoin="round" aria-hidden>
        <path d="M12 3 5 6v6c0 4.5 3 8 7 9 4-1 7-4.5 7-9V6l-7-3Z" />
        <path d="m9 12 2 2 4-4" />
      </svg>
    </span>
  );
}

/** The quiet note: an icon and one sentence about what happens next. */
export function EntryNote({ children }: { children: React.ReactNode }) {
  return (
    <p className="entry-note">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" aria-hidden style={{ flexShrink: 0, marginTop: 1 }}>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11v5M12 8h.01" />
      </svg>
      <span>{children}</span>
    </p>
  );
}

export function EntryIcon({ name, size = 18 }: { name: string; size?: number }) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.75,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
  switch (name) {
    case "check":
      return (
        <svg {...common} strokeWidth={2.5}>
          <path d="M20 6 9 17l-5-5" />
        </svg>
      );
    case "x":
      return (
        <svg {...common}>
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      );
    case "clock":
      return (
        <svg {...common}>
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v5l3 2" />
        </svg>
      );
    case "alert":
      return (
        <svg {...common}>
          <path d="M12 4 3 19h18L12 4Z" />
          <path d="M12 10v4M12 17h.01" />
        </svg>
      );
    case "mail":
      return (
        <svg {...common}>
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <path d="m3 7 9 6 9-6" />
        </svg>
      );
    case "shield":
      return (
        <svg {...common}>
          <path d="M12 3 5 6v6c0 4.5 3 8 7 9 4-1 7-4.5 7-9V6l-7-3Z" />
        </svg>
      );
    case "plus":
      return (
        <svg {...common}>
          <path d="M12 5v14M5 12h14" />
        </svg>
      );
    case "down":
      return (
        <svg {...common}>
          <path d="m6 9 6 6 6-6" />
        </svg>
      );
    case "qr":
      return (
        <svg {...common}>
          <rect x="4" y="4" width="6" height="6" />
          <rect x="14" y="4" width="6" height="6" />
          <rect x="4" y="14" width="6" height="6" />
        </svg>
      );
    default:
      return null;
  }
}

/**
 * The second step: six boxes with the active one outlined, and a real input
 * behind them. The boxes are a display of the value, not six separate fields,
 * so paste and backspace behave the way a person expects.
 */
export function OtpBoxes({
  value,
  onChange,
  length = 6,
  small = false,
  label,
  id,
  onComplete,
}: {
  value: string;
  onChange: (next: string) => void;
  length?: number;
  small?: boolean;
  label: string;
  id: string;
  onComplete?: (code: string) => void;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const digits = value.padEnd(length, " ").split("");

  return (
    <>
      <div className={`entry-otp${small ? " entry-otp--small" : ""}`} aria-hidden>
        {digits.map((d, i) => (
          <i
            key={i}
            className={`entry-otp__box${i === value.length ? " entry-otp__box--active" : ""}`}
          >
            {d.trim()}
          </i>
        ))}
      </div>
      <input
        ref={inputRef}
        id={id}
        className="entry-otp-input"
        value={value}
        onChange={(e) => {
          const next = e.target.value.replace(/\D/g, "").slice(0, length);
          onChange(next);
          if (next.length === length) onComplete?.(next);
        }}
        onKeyDown={(e) => {
          if (e.key === "Backspace" && value.length === 0) return;
          if (e.key === "Enter") onComplete?.(value);
        }}
        inputMode="numeric"
        autoComplete="one-time-code"
        maxLength={length}
        aria-label={label}
        data-testid={`${id}-input`}
        autoFocus
      />
    </>
  );
}

/** The password rules, ticking off as they are met. */
export function PasswordRules({
  password,
  touched,
}: {
  password: string;
  /** Rules only report failure once the field has been touched, so an empty
   *  form is not a wall of red. */
  touched: boolean;
}) {
  const rules: { met: boolean; text: string; test: (p: string) => boolean }[] = [
    { met: password.length >= 12, text: "At least 12 characters", test: (p) => p.length >= 12 },
    {
      met: password.length > 0 && !/^(password|12345|qwerty|letmein)/i.test(password),
      text: "Not a common password",
      test: (p) => p.length > 0 && !/^(password|12345|qwerty|letmein)/i.test(p),
    },
  ];
  return (
    <div className="entry-rules" aria-live="polite">
      {rules.map((r) => (
        <div
          key={r.text}
          className={`entry-rule${r.met ? " entry-rule--met" : touched && !r.met ? " entry-rule--failed" : ""}`}
        >
          <EntryIcon name={r.met ? "check" : touched ? "x" : "clock"} size={16} />
          <span>{r.text}</span>
        </div>
      ))}
    </div>
  );
}

export type StepState = "done" | "current" | "skipped" | "todo";

/** The wizard's step rail, with every step's true state. */
export function WizardRail({
  heading,
  steps,
  states,
  current,
  onSelect,
  doneCount,
  total,
  footnote,
}: {
  heading: string;
  steps: readonly string[];
  states: readonly StepState[];
  current: string;
  onSelect: (key: string) => void;
  doneCount: number;
  total: number;
  footnote: string;
}) {
  return (
    <div className="entry-steps">
      <div className="entry-steps__h">{heading}</div>
      {steps.map((label, i) => {
        const state = states[i] ?? "todo";
        return (
          <button
            key={label}
            type="button"
            className={`entry-step entry-step--${state}`}
            aria-current={current === label ? "step" : undefined}
            onClick={() => onSelect(label)}
            data-testid={`wizard-step-${i + 1}`}
          >
            <span className="entry-step__n">
              {state === "done" ? <EntryIcon name="check" size={14} /> : i + 1}
            </span>
            <span className="entry-step__label">{label}</span>
            {state === "skipped" && <span className="entry-step__skipped">Skipped</span>}
          </button>
        );
      })}
      <div className="entry-progress" aria-hidden>
        <i style={{ width: `${Math.round((doneCount / total) * 100)}%` }} />
      </div>
      <p className="entry-explain" style={{ marginTop: 8 }}>
        {doneCount} of {total} done. {footnote}
      </p>
    </div>
  );
}

/** The wizard's body: the card in front, faint cards behind for what remains. */
export function WizardStack({ children }: { children: React.ReactNode }) {
  return (
    <div className="entry-wizard">
      <div className="entry-stack">
        <div className="entry-wcard">{children}</div>
      </div>
    </div>
  );
}

/**
 * THE TWO "LATER" BUTTONS, AND WHY THEY ARE NOT THE SAME BUTTON.
 *
 *   Skip for now        Moves on. Records nothing. The step stays here.
 *   Never show again    Removes the guide. Records the choice, and it sticks.
 *
 * They are deliberately different weights, different places, and different
 * consequences (DESIGN-NOTES-auth.md §4). A skipped step is drawn as a dashed
 * circle and resumes exactly where it was left; the guide never nags back into
 * existence. `Never show again` lives in the guide's menu, behind a confirm
 * that lists its consequences in three lines.
 */
export function SkipButton({
  onClick,
  busy,
  label = "Skip for now",
}: {
  onClick: () => void;
  busy?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      className="entry-btn entry-btn--secondary entry-btn--small"
      onClick={onClick}
      disabled={busy}
      data-testid="wizard-skip-for-now"
    >
      {label}
    </button>
  );
}

/** The line under the actions that says what skipping does and does not do. */
export function SkipExplain({ extra }: { extra?: string }) {
  return (
    <p className="entry-explain">
      <b>Skip for now</b> records nothing and keeps this step here. To stop the guide for good, use{" "}
      <b>{extra ?? "Setup guide, Never show this setup again"}</b> at the top.
    </p>
  );
}

/** The guide's menu, in the header band. */
export function GuideMenu({ label, onNever }: { label: string; onNever: () => void }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent) {
      if (wrapRef.current && !wrapRef.current.contains(event.target as Node)) setOpen(false);
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div style={{ position: "relative" }} ref={wrapRef}>
      <button
        type="button"
        className="entry-band-action"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        data-testid="wizard-guide-menu"
      >
        {label}
        <EntryIcon name="down" size={16} />
      </button>
      {open && (
        <div className="entry-menu" role="menu">
          <button
            type="button"
            onClick={() => setOpen(false)}
            data-testid="wizard-close-for-now"
          >
            <b>Close for now</b>
            <span>Records nothing. You pick up exactly where you stopped.</span>
          </button>
          <button type="button" onClick={onNever} data-testid="wizard-never-show-again">
            <b>Never show this setup again</b>
            <span>Removes the guide from your home screen. You can bring it back from Settings.</span>
          </button>
        </div>
      )}
    </div>
  );
}

/**
 * The confirm behind "Never show this setup again". It opens by saying it is
 * not the same as skipping a step, contrasts the two, and lists three
 * consequences — because the difference between "later" and "never" is exactly
 * what a person cannot see, so it is spelled out rather than implied.
 */
export function NeverShowDialog({
  skippedCount,
  onKeep,
  onConfirm,
  busy,
}: {
  skippedCount: number;
  onKeep: () => void;
  onConfirm: () => void;
  busy?: boolean;
}) {
  return (
    <div className="entry-scrim" role="presentation" onClick={onKeep}>
      <div
        className="entry-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="never-show-title"
        onClick={(e) => e.stopPropagation()}
        data-testid="never-show-dialog"
      >
        <div className="entry-dialog__hd">
          <h2 id="never-show-title">Stop showing the setup guide?</h2>
          <p>This is different from skipping a step.</p>
        </div>
        <div className="entry-dialog__mb">
          <div className="entry-note">
            <EntryIcon name="clock" size={18} />
            <span>
              <b>Skip for now</b> keeps the guide and records nothing. <b>Never show again</b>{" "}
              removes it, and this choice sticks.
            </span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <p className="entry-consequence">
              <EntryIcon name="check" size={18} />
              <span>The guide leaves your home screen.</span>
            </p>
            <p className="entry-consequence">
              <EntryIcon name="check" size={18} />
              <span>
                The {skippedCount} skipped {skippedCount === 1 ? "step stays" : "steps stay"}{" "}
                undone. Nothing is switched on for you.
              </span>
            </p>
            <p className="entry-consequence">
              <EntryIcon name="check" size={18} />
              <span>You can bring it back from Settings, Workspace, Setup guide.</span>
            </p>
          </div>
        </div>
        <div className="entry-dialog__mf">
          <span className="entry-hint">You can change your mind.</span>
          <button type="button" className="entry-btn entry-btn--secondary" onClick={onKeep}>
            Keep the guide
          </button>
          <button
            type="button"
            className="entry-btn"
            onClick={onConfirm}
            disabled={busy}
            data-testid="never-show-confirm"
          >
            {busy ? "Removing…" : "Never show again"}
          </button>
        </div>
      </div>
    </div>
  );
}
