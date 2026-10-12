// /settings/fx — the FX rate book + the ops.v1_fx_reconciliation view
// (Wave 12 lane 10 backends, migrations 0065–0067).
//
// Purpose: the org's FX surfaces in one place: the append-only rate book
// (record a rate = one new row, corrections add rows and never rewrite),
// and the FX reconciliation read model — the deliverable to the books:
// original amount, rate + source/date, base equivalent, sync target. The
// additivity contract is rendered literally: reconciliation rows are
// GROUPED BY kind and totals are only ever taken within one kind — never
// summed across kinds.
//
// Non-purpose: no currency conversion math here (the core resolves rates);
// realized/unrealized gain-loss stays in the accounting system.
"use client";

import {
  Badge,
  Button,
  Card,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { money } from "../../../lib/format";

type FxRateEntry = components["schemas"]["FxRateEntry"];
type FxReconciliationRow = components["schemas"]["FxReconciliationRow"];

function isoDay(offsetDays: number): string {
  const d = new Date();
  d.setUTCHours(0, 0, 0, 0);
  d.setUTCDate(d.getUTCDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}

export default function FxPage(): ReactNode {
  const auth = useRequireAuth();
  const [rates, setRates] = useState<FxRateEntry[] | null>(null);
  const [recon, setRecon] = useState<FxReconciliationRow[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [recordError, setRecordError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // record-rate form
  const [baseCurrency, setBaseCurrency] = useState<string | null>(null);
  const [quoteCurrency, setQuoteCurrency] = useState("");
  const [asOf, setAsOf] = useState(isoDay(0));
  const [rate, setRate] = useState("");

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const [rateRes, reconRes, orgRes] = await Promise.all([
        api.api.GET("/api/v1/fx/rates"),
        api.api.GET("/api/v1/fx/reconciliation", { params: { query: { limit: 200 } } }),
        api.api.GET("/api/v1/org"),
      ]);
      if (rateRes.data === undefined || reconRes.data === undefined) {
        setLoadError("Could not load the FX surfaces — try reloading.");
        return;
      }
      setRates(rateRes.data ?? []);
      setRecon(reconRes.data ?? []);
      if (orgRes.data !== undefined) setBaseCurrency(orgRes.data.base_currency);
    })();
  }, [auth.status]);

  const base = baseCurrency ?? "USD";

  const bookByPair = useMemo(() => {
    // Append-only book: show the latest entry per (base, quote) pair,
    // newest as_of first.
    const latest = new Map<string, FxRateEntry>();
    for (const r of rates ?? []) {
      const key = `${r.base_currency}/${r.quote_currency}`;
      const prev = latest.get(key);
      if (prev === undefined || r.as_of_date > prev.as_of_date || (r.as_of_date === prev.as_of_date && r.captured_at > prev.captured_at)) {
        latest.set(key, r);
      }
    }
    return [...latest.values()].sort((a, b) => a.base_currency.localeCompare(b.base_currency) || a.quote_currency.localeCompare(b.quote_currency));
  }, [rates]);

  const reconByKind = useMemo(() => {
    // Additivity contract (FX7): group rows by kind; totals are per-kind
    // only and never summed across kinds.
    const groups = new Map<string, FxReconciliationRow[]>();
    for (const row of recon ?? []) {
      const list = groups.get(row.kind) ?? [];
      list.push(row);
      groups.set(row.kind, list);
    }
    return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [recon]);

  async function recordRate(): Promise<void> {
    setRecordError(null);
    const qc = quoteCurrency.trim().toUpperCase();
    const asOfTrim = asOf.trim();
    const rateTrim = rate.trim();
    if (qc === "" || asOfTrim === "" || rateTrim === "") {
      setRecordError("Quote currency, date, and rate are all required.");
      return;
    }
    const rateNum = Number(rateTrim);
    if (Number.isNaN(rateNum) || rateNum <= 0) {
      setRecordError("Rate must be a positive decimal (1 base unit buys N quote units).");
      return;
    }
    setBusy(true);
    const { error: apiError } = await api.api.POST("/api/v1/fx/rates", {
      body: {
        base_currency: (baseCurrency ?? "USD").toUpperCase(),
        quote_currency: qc,
        as_of_date: asOfTrim,
        rate: rateTrim,
      } as components["schemas"]["RecordRateRequest"],
    });
    setBusy(false);
    if (apiError !== undefined) {
      const detail = (apiError as { error?: { message?: string } }).error?.message;
      setRecordError(detail ?? "The server rejected the rate.");
      return;
    }
    setQuoteCurrency("");
    setRate("");
    const { data } = await api.api.GET("/api/v1/fx/rates");
    setRates(data ?? []);
  }

  if (rates === null && loadError === null) {
    return <Spinner />;
  }
  if (loadError !== null) {
    return <EmptyState message={loadError} />;
  }

  return (
    <div className="grid gap-6 max-w-4xl">
      <SettingsHeader
        title="FX rates"
        description={`The org's rate book against the base currency (${base ?? "—"}) plus the reconciliation view of every conversion the core recorded.`}
      />

      <Card className="grid gap-3 p-5">
        <h2 className="text-sm font-semibold">Record a rate</h2>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-4">
          <div className="grid gap-1">
            <Label htmlFor="fx-base">Base</Label>
            <Input id="fx-base" value={base ?? "…"} disabled aria-readonly />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="fx-quote">Quote currency</Label>
            <Input
              id="fx-quote"
              value={quoteCurrency}
              maxLength={3}
              onChange={(e) => setQuoteCurrency(e.target.value.toUpperCase())}
              placeholder="EUR"
              data-testid="fx-quote"
            />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="fx-asof">As of</Label>
            <Input id="fx-asof" type="date" value={asOf} onChange={(e) => setAsOf(e.target.value)} />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="fx-rate">Rate</Label>
            <Input
              id="fx-rate"
              inputMode="decimal"
              value={rate}
              onChange={(e) => setRate(e.target.value)}
              placeholder="1.0855"
              data-testid="fx-rate"
            />
            <FieldHint>1 {base ?? "base"} buys this many quote units.</FieldHint>
          </div>
        </div>
        {recordError !== null && (
          <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="fx-record-error">
            {recordError}
          </p>
        )}
        <div>
          <Button disabled={busy} onClick={() => void recordRate()} data-testid="fx-record">
            Record rate
          </Button>
        </div>
      </Card>

      <Card className="overflow-x-auto p-0">
        <h2 className="p-4 pb-0 text-sm font-semibold">Rate book</h2>
        <p className="px-4 pb-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          Append-only: the latest entry per pair is shown; corrections add rows.
        </p>
        <Table data-testid="fx-rate-book">
          <thead>
            <tr>
              <Th>Pair</Th>
              <Th>Rate</Th>
              <Th>As of</Th>
              <Th>Source</Th>
              <Th>Captured</Th>
            </tr>
          </thead>
          <tbody>
            {bookByPair.map((r) => (
              <Tr key={`${r.base_currency}/${r.quote_currency}/${r.as_of_date}`}>
                <Td className="font-medium">{r.base_currency}/{r.quote_currency}</Td>
                <Td>{r.rate}</Td>
                <Td>{r.as_of_date}</Td>
                <Td>
                  <Badge tone={r.source === "manual" ? "neutral" : "success"}>{r.source}</Badge>
                </Td>
                <Td>{new Date(r.captured_at).toLocaleString()}</Td>
              </Tr>
            ))}
            {bookByPair.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No rates recorded yet — record the first one above." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

      <Card className="p-0">
        <h2 className="p-4 pb-0 text-sm font-semibold">FX reconciliation</h2>
        <p className="px-4 pt-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          ops.v1_fx_reconciliation — the deliverable to the books. Totals are
          per kind and never summed across kinds (the additivity contract).
        </p>
        {recon !== null && recon.length === 0 && (
          <div className="p-4">
            <EmptyState message="No FX facts recorded yet — this fills as foreign-currency money moves through the system." />
          </div>
        )}
        <div className="grid gap-4 p-4" data-testid="fx-reconciliation">
          {reconByKind.map(([kind, rows]) => {
            const perKindTotal = rows.reduce((acc, r) => acc + (r.base_amount_minor ?? 0), 0);
            const currencies = [...new Set(rows.map((r) => r.original_currency ?? "—"))];
            return (
              <div key={kind} className="grid gap-1">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge tone="neutral">{kind}</Badge>
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {rows.length} fact{rows.length === 1 ? "" : "s"} · original currencies: {currencies.join(", ")}
                  </span>
                </div>
                <div className="overflow-x-auto">
                  <Table>
                    <thead>
                      <tr>
                        <Th>Fact at</Th>
                        <Th>Original</Th>
                        <Th>Rate</Th>
                        <Th>Source</Th>
                        <Th>Base equivalent</Th>
                        <Th>Ticket</Th>
                      </tr>
                    </thead>
                    <tbody>
                      {rows.slice(0, 20).map((r) => (
                        <Tr key={r.fact_id}>
                          <Td>{r.fact_at !== null && r.fact_at !== undefined ? new Date(r.fact_at).toLocaleString() : "—"}</Td>
                          <Td>
                            {r.original_amount_minor !== null && r.original_currency !== null
                              ? money(r.original_amount_minor ?? 0, r.original_currency ?? "USD")
                              : "—"}
                          </Td>
                          <Td>
                            {r.fx_rate_used !== null ? r.fx_rate_used : "—"} ({r.fx_rate_date})
                          </Td>
                          <Td>{r.fx_source ?? "—"}</Td>
                          <Td>{r.base_amount_minor !== null ? money(r.base_amount_minor ?? 0, base ?? "USD") : "—"}</Td>
                          <Td>
                            <a
                              href={`/tickets/${r.ticket_id}`}
                              className="underline underline-offset-2"
                              style={{ color: "var(--m-accent)" }}
                            >
                              open →
                            </a>
                          </Td>
                        </Tr>
                      ))}
                    </tbody>
                  </Table>
                </div>
                <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  {kind} base total:{" "}
                  <span data-testid={`fx-total-${kind}`}>{money(perKindTotal, base ?? "USD")}</span>
                  {rows.length > 20 && ` (showing 20 of ${rows.length})`}
                </p>
              </div>
            );
          })}
        </div>
      </Card>
    </div>
  );
}
