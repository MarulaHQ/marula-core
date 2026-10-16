// ShortcutOverlay — the hold-Alt overlay (Wave 29 B).
//
// A transient, non-interactive layer listing every command valid on the
// CURRENT surface: typed command labels + keys + a scope badge. Appears
// while Alt is held, dismisses on release. It is decoration for the
// keyboard, not a modal: pointer-events off, focus never moves. Motion
// rides the law (opacity only, inert under reduced motion).

"use client";

import { displayChord, modIsMeta } from "@/lib/commands/chords";
import type { CommandDef } from "@/lib/commands/types";

interface MetaRow {
  chord: string;
  label: string;
  display: string;
}

interface Row {
  id: string;
  chord: string;
  label: string;
  badge: string;
}

/** Scope badge: "Global" for everywhere rows, else the surface family name. */
function scopeBadge(command: CommandDef): string {
  const first = command.surfaces[0] ?? "";
  if (first === "") return "Global";
  const family = first.split("/").filter(Boolean)[0] ?? "Surface";
  return family.charAt(0).toUpperCase() + family.slice(1);
}

export function collectRows(
  commands: ReadonlyArray<CommandDef>,
  metas: ReadonlyArray<MetaRow>,
): ReadonlyArray<Row> {
  // ⌘/Ctrl reads better than "Mod" for the palette row: the search button
  // in the header labels itself ⌘K on darwin, Ctrl+K elsewhere.
  const kbd = (spec: string): string => {
    return displayChord(spec).replace("Mod", modIsMeta() ? "⌘" : "Ctrl");
  };
  return [
    ...metas.map((m) => ({
      id: `meta-${m.chord}`,
      chord: m.display,
      label: m.label,
      badge: "Layer",
    })),
    ...commands.map((c) => ({
      id: c.id,
      chord: kbd(c.chord),
      label: c.label,
      badge: scopeBadge(c),
    })),
  ];
}

export function ShortcutOverlay({
  commands,
  metaRows,
}: {
  commands: ReadonlyArray<CommandDef>;
  metaRows: ReadonlyArray<MetaRow>;
}) {
  const rows = collectRows(commands, metaRows);
  return (
    <div
      data-testid="shortcut-overlay"
      aria-hidden="true"
      className="pointer-events-none fixed inset-x-0 bottom-24 z-40 flex justify-center"
    >
      <div
        className="max-h-[60vh] w-fit max-w-[min(92vw,520px)] overflow-y-auto rounded-[var(--m-radius-control)] border px-4 py-3"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", boxShadow: "var(--shadow)" }}
      >
        <p className="mb-2 text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Keyboard shortcuts
        </p>
        <ul className="grid gap-1.5" data-testid="shortcut-overlay-rows">
          {rows.map((row) => (
            <li key={row.id} className="flex items-center justify-between gap-6">
              <span className="flex min-w-0 items-center gap-2">
                <span className="truncate text-[13px]" style={{ color: "var(--m-text)" }}>
                  {row.label}
                </span>
                <span
                  className="shrink-0 rounded-full border px-1.5 py-px text-xs"
                  style={{
                    borderColor: "var(--m-border)",
                    color: "var(--m-text-muted)",
                    background: "var(--m-bg-subtle)",
                  }}
                >
                  {row.badge}
                </span>
              </span>
              <kbd
                className="shrink-0 rounded-md border px-1.5 py-0.5 font-mono text-xs leading-none"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
              >
                {row.chord}
              </kbd>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
