// Shared disabled-state panel for settings sections owned by other waves.
//
// Purpose: render an honest, keyboard-reachable "not yet" surface for
// sections whose backends land in sibling lanes, so the IA is complete
// without faking functionality.
//
// Non-purpose: never used for shipped features.
"use client";

import { Card } from "@meridian/ui";

export function SettingsPendingPanel({
  title,
  owner,
}: {
  title: string;
  /** Which build wave owns the real editor. */
  owner: string;
}) {
  return (
    <div className="grid gap-6 max-w-2xl">
      <header>
        <h1
          className="text-lg font-semibold"
          style={{ color: "var(--m-text)" }}
        >
          {title}
        </h1>
      </header>
      <Card className="p-8 grid gap-2 justify-items-start">
        <p className="text-sm font-medium" style={{ color: "var(--m-text)" }}>
          Editor coming from another wave
        </p>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          This section&apos;s backend lands with the {owner}. The navigation
          entry is intentionally disabled until then — no configuration here
          changes behavior today.
        </p>
      </Card>
    </div>
  );
}
