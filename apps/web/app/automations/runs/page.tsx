"use client";
// /automations/runs — the run ledger (Wave 43 lane 18).
//
// A route of its own so the ledger reads as a full data surface, opened from
// the automations header, the rail's Automation section, or a workflow row's
// "Runs" link. `?rule=<id>` narrows it to one workflow's history, which is the
// "run history per rule" entry point.
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import RunLedger from "./RunLedger";

function LedgerRoute() {
  const params = useSearchParams();
  const rule = params.get("rule") ?? undefined;
  return <RunLedger ruleId={rule} />;
}

export default function AutomationsRunsPage() {
  return (
    <Suspense fallback={null}>
      <LedgerRoute />
    </Suspense>
  );
}
