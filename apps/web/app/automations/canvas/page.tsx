"use client";
// /automations/canvas — the power-tier workflow canvas.
//
// A route of its own so the canvas gets the full-width, full-height surface the
// design pack draws (the mockup's `.page.wf`), opened from the automations
// surface as a VIEW TOGGLE on the same rule (`?rule=<id>` promotes a simple
// rule into the canvas; without it, the first saved workflow opens). Editing
// here writes the rule row's `definition`; the simple-tier fields stay intact.
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import WorkflowCanvas from "./WorkflowCanvas";

function CanvasRoute() {
  const params = useSearchParams();
  const rule = params.get("rule") ?? undefined;
  return <WorkflowCanvas ruleId={rule} />;
}

export default function AutomationsCanvasPage() {
  return (
    <Suspense fallback={null}>
      <CanvasRoute />
    </Suspense>
  );
}
