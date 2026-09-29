// CSV export for a saved report definition's run (R6 builder).
//
// Purpose: mirrors lib/reports.ts's CSV bypass — text/csv is a stream, not
// a JSON envelope, so it rides a plain fetch with the URL literal
// `satisfies`-anchored to the canonical contract. The run compiles
// server-side via the stored spec (GET /api/v1/report-definitions/{id}/run).
import type { MeridianPaths } from "@marula-core/sdk";
import { apiBaseUrl, loadTokens } from "./api";

const RUN_PATH = "/api/v1/report-definitions/{id}/run" satisfies keyof MeridianPaths;

export function builderCsvUrl(id: string, from: string, to: string): string {
  const params = new URLSearchParams();
  if (from !== "") params.set("from", from);
  if (to !== "") params.set("to", to);
  params.set("format", "csv");
  return `${apiBaseUrl()}${RUN_PATH.replace("{id}", id)}?${params.toString()}`;
}

/** Stream the definition's compiled report as text/csv and download it. */
export async function downloadBuilderCsv(
  id: string,
  name: string,
  from: string,
  to: string,
): Promise<void> {
  const tokens = loadTokens();
  const response = await fetch(builderCsvUrl(id, from, to), {
    headers: tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` },
  });
  if (!response.ok) throw new Error(`CSV export failed (${response.status})`);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${name.replace(/[^\w-]+/g, "-").toLowerCase()}.csv`;
  anchor.click();
  URL.revokeObjectURL(url);
}
