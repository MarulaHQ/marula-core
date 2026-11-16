"use client";
/**
 * ExpiryTable — one table for every expiring thing, whatever wrote the date.
 *
 * Purpose: the rollup the brief asks for, drawn the way the pack draws a
 * ledger: words and a date, a blush row when the date is close, and red only
 * when it has already passed (a real breach). The provenance column exists
 * because a typed warranty is not a registry lookup, and the two must not read
 * the same.
 *
 * Non-purpose: no dates computed in the browser beyond "how many days", and no
 * colour-only status — every row says the number of days in words.
 */

import Link from "next/link";
import type { ExpiryRow } from "./knowledge-api";
import { HEAD_CELL_CLASS, TableCard } from "./ledger";

const KIND_LABEL: Record<string, string> = {
  domain_registrar: "Domain registration",
  domain_tls: "TLS certificate",
  document_review: "Document review",
  asset_warranty: "Warranty",
  secret_rotation: "Credential rotation",
};

/** Whole days between today (local midnight) and the due date. */
export function daysUntil(dueOn: string): number {
  const due = new Date(`${dueOn}T00:00:00`);
  const today = new Date();
  const start = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  return Math.round((due.getTime() - start.getTime()) / 86_400_000);
}

export function dueWords(dueOn: string): { text: string; tone: "danger" | "warning" | "neutral" } {
  const days = daysUntil(dueOn);
  if (days < 0) return { text: `Expired ${Math.abs(days)} days ago`, tone: "danger" };
  if (days === 0) return { text: "Due today", tone: "warning" };
  if (days <= 30) return { text: `Due in ${days} days`, tone: "warning" };
  return { text: `Due in ${days} days`, tone: "neutral" };
}

function tonePair(tone: "danger" | "warning" | "neutral"): { background?: string; color: string } {
  if (tone === "danger") {
    return {
      background: "var(--m-status-danger-bg)",
      color: "var(--m-status-danger-fg)",
    };
  }
  if (tone === "warning") {
    return { background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" };
  }
  return { color: "var(--m-text-secondary)" };
}

export function ExpiryTable({
  rows,
  showClient = true,
}: {
  rows: ExpiryRow[];
  showClient?: boolean;
}): React.JSX.Element {
  return (
    <TableCard label="Expiring items">
      <table className="w-full border-collapse text-[13px]">
        <thead>
          <tr style={{ background: "var(--m-surface-2)" }}>
            <th className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
              Expires
            </th>
            <th className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
              What
            </th>
            {showClient ? (
              <th className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                Client
              </th>
            ) : null}
            <th className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
              Owner
            </th>
            <th className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
              Date
            </th>
            <th className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
              Source
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const words = dueWords(row.due_on);
            const palette = tonePair(words.tone);
            return (
              <tr
                key={`${row.kind}:${row.source_id ?? row.title}`}
                style={{
                  borderBottom: "1px solid var(--m-border)",
                  background: words.tone === "danger" ? palette.background : undefined,
                  color: palette.color,
                }}
                data-testid={`expiry-${row.kind}`}
              >
                <td className="px-3 py-2 font-semibold whitespace-nowrap">{words.text}</td>
                <td className="px-3 py-2">
                  <span className="font-semibold">{row.title}</span>
                  <span className="ml-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    {KIND_LABEL[row.kind] ?? row.kind}
                  </span>
                </td>
                {showClient ? (
                  <td className="px-3 py-2">
                    <Link
                      href={`/kb/clients/${row.client_id}`}
                      className="underline underline-offset-2"
                      style={{ color: "inherit" }}
                    >
                      {row.client_name}
                    </Link>
                  </td>
                ) : null}
                <td className="px-3 py-2">{row.owner ?? "No owner"}</td>
                <td className="px-3 py-2 font-mono text-[12px] tabular-nums">{row.due_on}</td>
                <td className="px-3 py-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  {row.provenance}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {rows.length === 0 ? (
        <p className="px-3 py-6 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Nothing expires in this window. Domains, certificates, document reviews, warranties and
          credential rotation all report here when they are added.
        </p>
      ) : null}
    </TableCard>
  );
}
