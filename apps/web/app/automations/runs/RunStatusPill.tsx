"use client";
// The run ledger's status pill (Wave 43 lane 18).
//
// One small wrapper over the kit's `Pill`, so a run status and a step status
// can carry the kit's own tone names (`progress`, `waiting`, `success`,
// `attention`, `neutral`) — the `Badge` wrapper in `@marula-core/ui` maps to a
// narrower set and cannot say "waiting". The colour never travels alone: the
// dot says "this is a status", and the word says which (docs/theme.md §6).
import { Pill } from "@marula-core/ui/kit";
import type { ReactNode } from "react";
import type { PillTone } from "../../../lib/workflow/run-labels";

export function DotPill({ tone, children }: { tone: PillTone; children: ReactNode }) {
  return (
    <Pill tone={tone} dot>
      {children}
    </Pill>
  );
}
