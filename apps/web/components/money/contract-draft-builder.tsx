"use client";
/**
 * The contract builder's draft-aware parts: parties and term, pricing from
 * quantities, service levels marked Your words or Template, and the clause
 * library with its legal-review flag.
 *
 * Purpose: C03 and C04 are the contract's version of the check step and the
 * draft. The rule they exist to hold is that no legal wording is written by a
 * model: every clause here is a library row, and the only thing a draft may
 * change is one parameter, which turns the row into a variant.
 *
 * Non-purpose: no state, no save. The route owns the contract and the create
 * call; the library is the server's.
 */
import { Icon } from "@marula-core/ui/kit";
import { useState } from "react";

import type {
  ClauseRef,
  ContractDraft,
  ContractPriceLine,
  LineProvenance,
  Understanding,
} from "../../lib/money-authoring";
import { DecimalField } from "./decimal-field";
import { formatMoneyMinor, lineAmountMinor } from "./draft-math";
import { FieldValue, ProvenanceDot } from "./draft-provenance";

/** Months in one billing period, for a price that is quoted per period. */
const MONTHS_PER_PERIOD: Record<string, number> = {
  weekly: 12 / 52,
  monthly: 1,
  quarterly: 3,
  semiannual: 6,
  annual: 12,
};

/** The period's own noun: a price is "per month", never "per monthly". */
const PERIOD_NOUN: Record<string, string> = {
  weekly: "week",
  monthly: "month",
  quarterly: "quarter",
  semiannual: "half-year",
  annual: "year",
};

/** The types a billing frequency applies to, matching the create call. */
const RECURRING_TYPES = new Set(["recurring", "retainer"]);

export function ContractParties({
  draft,
  clientName,
}: {
  draft: ContractDraft;
  clientName: string | null;
}): React.JSX.Element {
  const includes = (draft.clause_refs ?? [])
    .filter((clause) => clause.status === "standard")
    .map((clause) => clause.title);
  return (
    <div className="grid gap-2.5" data-testid="contract-parties">
      <Row label="Client" value={clientName ?? draft.client_name ?? "Not matched"} />
      <Row label="Starts" value={draft.effective_from ?? "Not set"} />
      <Row
        label="Term"
        value={
          draft.term_months === null || draft.term_months === undefined
            ? "Not set"
            : `${draft.term_months} months`
        }
      />
      <Row
        label="Renews"
        value={
          draft.billing_frequency === null || draft.billing_frequency === undefined
            ? "One-off"
            : `Automatically, ${draft.billing_frequency}`
        }
      />
      {includes.length > 0 ? (
        <div className="flex flex-wrap gap-1.5">
          {includes.map((title) => (
            <span className="m-dsc-pill m-dsc-pill-muted" key={title}>
              {title}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="m-dsc-kv">
      <span>{label}</span>
      <span>{value}</span>
    </div>
  );
}

/** Pricing built from quantities and rates, never from a stated total. */
export function ContractPricing({
  pricing,
  currency,
  frequency,
  onChange,
  proposed,
}: {
  pricing: ContractPriceLine[];
  currency: string;
  frequency: string | null;
  onChange?: (index: number, patch: Partial<ContractPriceLine>) => void;
  /** Rows a change proposal would touch, tinted and tagged. */
  proposed?: Set<number>;
}): React.JSX.Element {
  const total = pricing.reduce(
    (sum, line) => sum + lineAmountMinor(line.quantity_milli, line.unit_price_minor),
    0,
  );
  const periodNoun = PERIOD_NOUN[frequency ?? ""] ?? "period";
  return (
    <div data-testid="contract-pricing">
      <table className="m-dsc-items">
        <thead>
          <tr>
            <th>Item</th>
            <th className="m-dsc-num">Qty</th>
            <th className="m-dsc-num">Rate</th>
            <th className="m-dsc-num">Per {periodNoun}</th>
          </tr>
        </thead>
        <tbody>
          {pricing.map((line, index) => (
            <tr
              key={`${line.label}-${index}`}
              data-proposed={proposed?.has(index) === true ? "true" : "false"}
            >
              <td>
                <span className="m-dsc-item-name">
                  <ProvenanceDot source={line.price_assumed === true ? "assumed" : "notes"} />
                  <span>{line.label}</span>
                  {proposed?.has(index) === true ? (
                    <span className="m-dsc-proposed-tag">
                      <Icon name="spark" size={12} /> Proposed
                    </span>
                  ) : null}
                </span>
              </td>
              <td className="m-dsc-num">
                {onChange !== undefined ? (
                  <DecimalField
                    label={`Quantity for ${line.label}`}
                    value={line.quantity_milli}
                    scale={1000}
                    width={74}
                    onCommit={(quantity_milli) =>
                      onChange(index, { quantity_milli, quantity_assumed: false })
                    }
                  />
                ) : (
                  <FieldValue assumed={line.quantity_assumed}>{line.quantity_milli / 1000}</FieldValue>
                )}
              </td>
              <td className="m-dsc-num">
                {onChange !== undefined ? (
                  <DecimalField
                    label={`Rate for ${line.label}`}
                    value={line.unit_price_minor}
                    scale={100}
                    width={96}
                    decimals={2}
                    onCommit={(unit_price_minor) =>
                      onChange(index, { unit_price_minor, price_assumed: false })
                    }
                  />
                ) : (
                  <FieldValue assumed={line.price_assumed}>
                    {formatMoneyMinor(line.unit_price_minor, currency)}
                  </FieldValue>
                )}
              </td>
              <td className="m-dsc-num">
                <b>{formatMoneyMinor(lineAmountMinor(line.quantity_milli, line.unit_price_minor), currency)}</b>
              </td>
            </tr>
          ))}
          <tr className="m-dsc-section-row">
            <td colSpan={3}>Total per {periodNoun}</td>
            <td className="m-dsc-num">
              <b>{formatMoneyMinor(total, currency)}</b>
            </td>
          </tr>
        </tbody>
      </table>
      <p className="m-dsc-hint" style={{ marginTop: 8 }}>
        Billed {frequency ?? "per period"} in advance.
        {pricing.length === 0
          ? " The price is set on the contract; the words did not break it into quantities."
          : ""}
      </p>
    </div>
  );
}

/**
 * Service levels: "Your words" where the draft filled the clause's parameter,
 * "Template" where the library's standard value stands.
 */
export function ContractServiceLevels({
  clauses,
}: {
  clauses: ClauseRef[];
}): React.JSX.Element | null {
  const rows = clauses.filter((clause) =>
    ["response", "payment"].includes(clause.key),
  );
  if (rows.length === 0) return null;
  return (
    <table className="m-dsc-items" data-testid="contract-levels">
      <thead>
        <tr>
          <th>Term</th>
          <th>Value</th>
          <th>Source</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((clause) => {
          const own = clause.parameter_value !== null && clause.parameter_value !== undefined;
          return (
            <tr key={clause.key}>
              <td>
                <b>{clause.title}</b>
              </td>
              <td>{own ? clause.parameter_value : clause.standard_value}</td>
              <td>
                <span
                  className={`m-dsc-pill ${own ? "m-dsc-pill-gold" : "m-dsc-pill-muted"}`}
                >
                  {own ? "Your words" : "Template"}
                </span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/** The clause library on the draft, with the one flag that matters. */
export function ClauseList({
  draft,
  library,
}: {
  draft: ContractDraft;
  library: { key: string; title: string; status: string; standard_value?: string | null }[];
}): React.JSX.Element {
  const [openClause, setOpenClause] = useState<string | null>(null);
  const refs = draft.clause_refs ?? [];
  const rows: ClauseRef[] =
    refs.length > 0
      ? refs
      : library.map((clause) => ({
          clause_id: clause.key,
          key: clause.key,
          title: clause.title,
          body: "",
          version: 1,
          status: clause.status === "variant" ? "variant" : "standard",
          review_required: false,
          parameter_label: null,
          parameter_value: null,
          standard_value: clause.standard_value ?? null,
        }));
  return (
    <div data-testid="contract-clauses">
      {rows.map((clause) => (
        <div key={clause.key}>
          <div className="m-dsc-clause">
            <ProvenanceDot source={clause.review_required ? "assumed" : "notes"} />
            <b>{clause.title}</b>
            {clause.parameter_value !== null && clause.parameter_value !== undefined ? (
              <span className="m-dsc-item-detail">{clause.parameter_value}</span>
            ) : null}
            <span
              className={`m-dsc-pill ${
                clause.status === "variant" ? "m-dsc-pill-variant" : "m-dsc-pill-standard"
              }`}
            >
              {clause.status === "variant" ? (
                <>
                  <Icon name="alert" size={12} /> Changed: needs review
                </>
              ) : (
                <>
                  <Icon name="check" size={12} /> Standard
                </>
              )}
            </span>
            {clause.body !== "" ? (
              <button
                type="button"
                className="m-dsc-option"
                aria-expanded={openClause === clause.key}
                onClick={() => setOpenClause((current) => (current === clause.key ? null : clause.key))}
              >
                Read
              </button>
            ) : null}
          </div>
          {openClause === clause.key ? (
            <p className="m-dsc-hint" style={{ paddingBottom: 8 }}>
              {clause.body}
            </p>
          ) : null}
        </div>
      ))}
      <p className="m-dsc-hint" style={{ marginTop: 8 }}>
        {rows.length} clauses from your library. No new wording was written.
      </p>
    </div>
  );
}

/**
 * The contract summary column: the value, the signer, and the one place a
 * clause that differs from the standard is called out — which stays even with
 * no grant, because a changed clause needs review whoever wrote it (C06).
 */
export function ContractSummary({
  draft,
  currency,
  library,
  checks,
  understanding,
}: {
  draft: ContractDraft;
  currency: string;
  library: ClauseRef[];
  checks: string[];
  understanding: Understanding | null;
}): React.JSX.Element {
  const signer = understanding?.questions.find((question) => /sign/i.test(question.text));
  const variants = (draft.clause_refs ?? []).filter((clause) => clause.review_required);
  const value =
    draft.value_minor ?? (draft.pricing ?? []).reduce(
      (sum, line) => sum + lineAmountMinor(line.quantity_milli, line.unit_price_minor),
      0,
    );
  // The value is per billing period, not per month: a quarterly 3,000 over 12
  // months is 12,000, not 36,000. A one-off, or a type the create call saves
  // with no frequency, has no period to multiply by at all.
  const termMonths = draft.term_months ?? null;
  const periodNoun = PERIOD_NOUN[draft.billing_frequency ?? ""] ?? "period";
  const periodMonths = MONTHS_PER_PERIOD[draft.billing_frequency ?? ""];
  const termTotal =
    termMonths !== null && periodMonths !== undefined && RECURRING_TYPES.has(draft.contract_type)
      ? Math.round((value * termMonths) / periodMonths)
      : null;
  return (
    <>
      <div>
        <p className="m-dsc-lab">Value</p>
        <div className="m-dsc-total" data-testid="contract-total">
          {formatMoneyMinor(value, currency)} <small>a {periodNoun}</small>
        </div>
        {termTotal !== null && termMonths !== null ? (
          <p className="m-dsc-hint" style={{ marginTop: 4 }}>
            {formatMoneyMinor(termTotal, currency)} over {termMonths} months
          </p>
        ) : null}
      </div>
      <div>
        <p className="m-dsc-lab">Signer</p>
        <div className="m-dsc-kv">
          <span>For the client</span>
          <span>{signer?.chosen ?? signer?.options[0] ?? "Choose it before sending"}</span>
        </div>
        <div className="m-dsc-kv">
          <span>For us</span>
          <span>You</span>
        </div>
      </div>
      {variants.length > 0 ? (
        <div data-testid="contract-review">
          <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
            Needs a look, {variants.length}
          </p>
          {variants.map((clause) => (
            <div className="m-dsc-check" key={clause.key}>
              <Icon name="alert" size={14} />
              <span>
                {clause.title} differs from the standard wording
                {clause.standard_value !== null && clause.standard_value !== undefined
                  ? ` (standard: ${clause.standard_value})`
                  : ""}
                . Legal review is advised.
              </span>
            </div>
          ))}
        </div>
      ) : null}
      {checks.length > 0 ? (
        <div>
          <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
            Also check
          </p>
          {checks.map((check) => (
            <div className="m-dsc-check" key={check}>
              <Icon name="alert" size={14} />
              <span>{check}</span>
            </div>
          ))}
        </div>
      ) : null}
      {library.length > 0 ? <p className="m-dsc-hint">{library.length} approved clauses available.</p> : null}
    </>
  );
}

/** Which lines a contract proposal touches, for the tinted pricing rows. */
export function proposedPricingIndexes(paths: string[]): Set<number> {
  const indexes = new Set<number>();
  for (const path of paths) {
    const match = /^pricing\[(\d+)\]/.exec(path);
    if (match !== null) indexes.add(Number(match[1]));
  }
  return indexes;
}

export type { LineProvenance };
