// CheatSheetDialog — the `?`-key full-registry dialog (Wave 29 B).
//
// The SECOND view over the SAME registry (lib/commands) — no second source
// of truth. The hold-Alt overlay is the per-surface slice; this dialog is
// the whole map: every command grouped by scope with its chord and scope
// badge. Read-only reference: it stays reachable even with the all_off
// profile, because turning shortcuts off should not hide their documentation.

"use client";

import { useEffect, useRef } from "react";
import { registryGroups } from "@/lib/commands/registry";
import { displayChord } from "@/lib/commands/chords";

/** Machine chord → OS-native label for the search palette row. */
function kbdLabel(spec: string): string {
  const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform ?? "");
  return displayChord(spec).replace("Mod", isMac ? "⌘" : "Ctrl");
}

export function CheatSheetDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const dialogRef = useRef<HTMLDivElement | null>(null);

  // Escape closes (native dialog semantics); alt+x clicks the same close
  // control below, exactly like a click on it would.
  useEffect(() => {
    if (open) dialogRef.current?.focus();
  }, [open]);

  if (!open) return null;
  const groups = registryGroups();

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: "var(--m-scrim)" }}
      data-testid="shortcut-cheatsheet-scrim"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcut-cheatsheet-title"
        data-testid="shortcut-cheatsheet"
        tabIndex={-1}
        className="max-h-[80vh] w-[min(92vw,720px)] overflow-y-auto rounded-[var(--m-radius-control)] border outline-none"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        <div
          className="sticky top-0 flex items-center justify-between gap-4 border-b px-5 py-3"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          <div>
            <h2 id="shortcut-cheatsheet-title" className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Keyboard shortcuts
            </h2>
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Hold Alt on any surface for the shortcuts that apply there. Press ? to reopen this cheat sheet.
            </p>
          </div>
          <button
            type="button"
            data-testid="shortcut-cheatsheet-close"
            data-shortcuts-close="1"
            onClick={onClose}
            className="shrink-0 rounded-md border px-2 py-1 text-xs"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
          >
            Close
          </button>
        </div>
        <div className="grid gap-6 px-5 py-4">
          {groups.map((group) => (
            <section key={group.title} aria-label={group.title}>
              <h3
                className="mb-2 text-xs font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                {group.title}
              </h3>
              <ul className="grid gap-1.5">
                {group.commands.map((command) => (
                  <li key={command.id} className="flex items-start justify-between gap-6">
                    <span className="flex min-w-0 flex-col">
                      <span className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                        {command.label}
                      </span>
                      <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        {command.description}
                      </span>
                    </span>
                    <span className="flex shrink-0 items-center gap-2">
                      <span
                        className="rounded-full border px-1.5 py-px text-xs"
                        style={{
                          borderColor: "var(--m-border)",
                          color: "var(--m-text-muted)",
                          background: "var(--m-bg-subtle)",
                        }}
                      >
                        {command.surfaces.length === 0 ? "Global" : command.surfaces.join(" · ")}
                      </span>
                      <kbd
                        className="rounded-md border px-1.5 py-0.5 font-mono text-xs leading-none"
                        style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
                      >
                        {kbdLabel(command.chord)}
                      </kbd>
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
