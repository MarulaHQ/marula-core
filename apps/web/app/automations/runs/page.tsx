"use client";
// /automations/runs — the run ledger (Wave 43 lane 18).
//
// A route of its own so the ledger reads as a full data surface, opened from
// the automations header, the rail's Automation section, or a workflow row's
// "Runs" link. BOTH filters ride the URL — `?rule=<id>` for one workflow's
// history, `?status=<status>` for one status — so a narrowed view is shareable
// and Back behaves. Neither is re-read after mount, so a change cannot refetch
// twice.
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import RunLedger from "./RunLedger";

function LedgerRoute() {
  const params = useSearchParams();
  const rule = params.get("rule") ?? undefined;
  const status = params.get("status") ?? undefined;
  return <RunLedger ruleId={rule} initialStatus={status} />;
}

export default function AutomationsRunsPage() {
  return (
    <Suspense fallback={null}>
      <LedgerRoute />
    </Suspense>
  );
}
