"use client";
// /automations/runs/[id] — one run and its trace (Wave 43 lane 18).
//
// The id is the path segment; the surface itself is the client component in
// RunDetail.tsx.
import { useParams } from "next/navigation";
import RunDetail from "./RunDetail";

export default function AutomationsRunDetailPage() {
  const params = useParams<{ id: string }>();
  return <RunDetail runId={params.id} />;
}
