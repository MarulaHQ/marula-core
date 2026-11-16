// /contracts — the contract ledger and the Describe it authoring surface.
//
// DERIVATION-GUIDE: `/contracts` and `/contracts/[id]` **Derive** from TL and
// the KIT components, changing two things: the ledger carries a Status
// column, and renewals are BLUSH PILLS.
//
// The colour rule is the load-bearing one here, and it is absolute
// (`DESIGN-NOTES.md` §A2, and the lane's own handoff): **a contract nearing
// renewal is blush, never red.** Red in this product means one of three
// things — a genuine SLA breach, a P1, or a destructive action — and a term
// approaching its end is none of them. A term that has ALREADY elapsed is
// different: the contract has run past its end, which is a real commercial
// event, so that one is allowed red, and only once it is actually past.
//
// Purpose: know what was promised, for how long, and when it renews. The
// Describe it panel (pack C01–C06) docks at 440px and builds a draft the same
// way the quote panel does — with one rule it states out loud: **no legal
// wording is written by a model.** A draft names clauses from the approved
// library and may fill one parameter; a clause that differs from the standard
// is flagged for legal review, and that flag survives with no grant too.
//
// Non-purpose: no contract authoring on this page beyond the builder (which the
// record also carries); no invoice runs.
"use client";

import {
  Badge,
  Button,
  ColumnChooser,
  DataToolbar,
  Dialog,
  DialogContent,
  EmptyState,
  ExportControl,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceFooter,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
  DensitySwitch,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { rejectionText } from "../../lib/error-copy";
import { api, useRequireAuth } from "../../lib/api";
import { money } from "../../lib/format";
import { ErrorNote } from "../../components/knowledge/ledger";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import { formatPackDate, renewalSignal } from "../../lib/renewal-signals";
import {
  contractClauses,
  proposeContractChange,
  type ChangeOperation,
  type ClauseRef,
  type ContractDraft,
  type ContractPriceLine,
  type MoneySource,
  type Understanding,
} from "../../lib/money-authoring";
import {
  DescribeMoney,
  DescribePanel,
  NewMoneySplit,
  useMoneyAuthoring,
} from "../../components/money/describe-money";
import {
  ChangeProposalCard,
  QuoteContextColumn,
  type BuilderTab,
} from "../../components/money/quote-draft-builder";
import {
  ClauseList,
  ContractParties,
  ContractPricing,
  ContractServiceLevels,
  ContractSummary,
  proposedPricingIndexes,
} from "../../components/money/contract-draft-builder";
import { contractTotal, formatMoneyMinor } from "../../components/money/draft-math";

type Contract = components["schemas"]["Contract"];
type ContractType = components["schemas"]["ContractType"];
type ContractMutationResponse = components["schemas"]["ContractMutationResponse"];
type Client = components["schemas"]["Client"];

const TYPES: ContractType[] = [
  "recurring",
  "time_and_materials",
  "retainer",
  "fixed_fee",
  "per_unit",
  "per_incident",
];

const TYPE_LABEL: Record<string, string> = {
  recurring: "Recurring",
  time_and_materials: "Time and materials",
  retainer: "Retainer",
  fixed_fee: "Fixed fee",
  per_unit: "Per unit",
  per_incident: "Per incident",
};

const STATUS_LABEL: Record<string, string> = {
  draft: "Draft",
  active: "Active",
  ended: "Ended",
  cancelled: "Cancelled",
};

const VIEWS = [
  { id: "all", label: "All" },
  { id: "active", label: "Active" },
  { id: "renewing", label: "Renewing soon" },
];

const SURFACE = "contracts.list";

type ColumnId = "number" | "name" | "client" | "type" | "status" | "term" | "amount";

const COLUMNS: { id: ColumnId; label: string; defaultVisible: boolean }[] = [
  { id: "number", label: "Number", defaultVisible: true },
  { id: "name", label: "Name", defaultVisible: true },
  { id: "client", label: "Client", defaultVisible: true },
  { id: "type", label: "Type", defaultVisible: true },
  { id: "status", label: "Status", defaultVisible: true },
  { id: "term", label: "Term", defaultVisible: true },
  { id: "amount", label: "Per period", defaultVisible: true },
];

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

/**
 * Add months to a calendar date. Both ends stay in UTC — a local midnight
 * converts to the previous day west of UTC, which is a day early once
 * `toISOString` prints it — and the day is clamped, so 31 January plus one
 * month is 28 February rather than 3 March.
 */
function addMonths(iso: string, months: number): string {
  const date = new Date(`${iso}T00:00:00Z`);
  if (Number.isNaN(date.getTime())) return iso;
  const day = date.getUTCDate();
  date.setUTCDate(1);
  date.setUTCMonth(date.getUTCMonth() + months);
  const lastDay = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 0)).getUTCDate();
  date.setUTCDate(Math.min(day, lastDay));
  return date.toISOString().slice(0, 10);
}

export default function ContractsPage(): ReactNode {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();
  const granted = useMoneyAuthoring("contract_authoring");

  const [contracts, setContracts] = useState<Contract[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [library, setLibrary] = useState<ClauseRef[]>([]);

  const [view, setView] = useState("all");
  const [typeFilter, setTypeFilter] = useState<ContractType | "all">("all");
  const [clientFilter, setClientFilter] = useState<string>("all");
  const [query, setQuery] = useState("");

  // ---- builder -----------------------------------------------------------
  const [createOpen, setCreateOpen] = useState(false);
  const [number, setNumber] = useState("");
  const [name, setName] = useState("");
  const [clientId, setClientId] = useState<string>("");
  const [type, setType] = useState<ContractType>("recurring");
  const [currency, setCurrency] = useState("USD");
  const [effectiveFrom, setEffectiveFrom] = useState(today());
  const [effectiveUntil, setEffectiveUntil] = useState("");
  const [amount, setAmount] = useState("");
  const [frequency, setFrequency] = useState<string>("monthly");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  // The draft's memory.
  const [pricing, setPricing] = useState<ContractPriceLine[]>([]);
  const [clauseRefs, setClauseRefs] = useState<ClauseRef[]>([]);
  const [sources, setSources] = useState<MoneySource[]>([]);
  const [checks, setChecks] = useState<string[]>([]);
  const [understanding, setUnderstanding] = useState<Understanding | null>(null);
  const [termMonths, setTermMonths] = useState<number | null>(null);
  const [fromNotes, setFromNotes] = useState(false);
  const [builderTab, setBuilderTab] = useState<BuilderTab>("summary");

  // ---- changes by describing --------------------------------------------
  const [changeInstruction, setChangeInstruction] = useState("");
  const [changeBusy, setChangeBusy] = useState(false);
  const [changeError, setChangeError] = useState<string | null>(null);
  const [changeSentence, setChangeSentence] = useState<string | null>(null);
  const [changeOperations, setChangeOperations] = useState<ChangeOperation[]>([]);
  const [changeDraft, setChangeDraft] = useState<ContractDraft | null>(null);
  const [changeTotal, setChangeTotal] = useState<number | null>(null);
  /** The draft the proposal was described against; a newer draft makes it stale. */
  const [changeBase, setChangeBase] = useState<string | null>(null);
  const changeRequest = useRef(0);
  const [proposed, setProposed] = useState<Set<number>>(new Set());

  // ---- describe ----------------------------------------------------------
  const [describeOpen, setDescribeOpen] = useState(false);
  const [renewal, setRenewal] = useState<{
    text: string;
    using: { label: string; icon: "file" | "ticket" | "chart" }[];
  } | null>(null);
  const renewHandled = useRef(false);

  const now = Date.now();

  const refresh = useCallback(async () => {
    const [c, cl] = await Promise.all([
      api.api.GET("/api/v1/contracts"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (c.error !== undefined || c.data === undefined) {
      setLoadError("Failed to load contracts.");
      setContracts([]);
    } else {
      setContracts(c.data);
      setLoadError(null);
    }
    if (cl.error === undefined && cl.data !== undefined) setClients(cl.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // The clause library is read once: it is the only source of clause text, and
  // the builder shows it with no grant too (where its own column has no marks).
  useEffect(() => {
    if (auth.status !== "authed") return;
    void contractClauses()
      .then((rows) => setLibrary(rows))
      .catch(() => undefined);
  }, [auth.status]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((c) => [c.id, c.name]));
    return (id: string) => map.get(id) ?? "—";
  }, [clients]);

  const posture = useMemo(() => {
    const list = contracts ?? [];
    return {
      active: list.filter((c) => c.status === "active").length,
      draft: list.filter((c) => c.status === "draft").length,
      renewing: list.filter((c) => {
        const s = renewalSignal(c.effective_until, now);
        return s !== null && s.days >= 0;
      }).length,
      elapsed: list.filter((c) => {
        const s = renewalSignal(c.effective_until, now);
        return s !== null && s.days < 0;
      }).length,
      book: moneyOf(list),
    };
  }, [contracts, now]);

  const filtered = useMemo(() => {
    if (contracts === null) return null;
    const q = query.trim().toLowerCase();
    return contracts.filter((c) => {
      if (view === "active" && c.status !== "active") return false;
      if (view === "renewing") {
        const s = renewalSignal(c.effective_until, now);
        if (s === null || s.days < 0) return false;
      }
      if (typeFilter !== "all" && c.contract_type !== typeFilter) return false;
      if (clientFilter !== "all" && c.client_id !== clientFilter) return false;
      if (q === "") return true;
      return (
        c.number.toLowerCase().includes(q) ||
        c.name.toLowerCase().includes(q) ||
        clientName(c.client_id).toLowerCase().includes(q)
      );
    });
  }, [contracts, view, typeFilter, clientFilter, query, clientName, now]);

  const saved = prefs.view(SURFACE);
  const [visibleColumns, setVisibleColumns] = useState<ColumnId[]>(COLUMNS.map((c) => c.id));
  useEffect(() => {
    if (!prefs.ready) return;
    const savedIds = saved.columns.map((c) => c.key as ColumnId);
    if (savedIds.length === 0) return;
    const known = COLUMNS.filter((c) => savedIds.includes(c.id));
    if (known.length > 0) setVisibleColumns(known.map((c) => c.id));
  }, [prefs.ready, saved.columns]);

  const visible = useMemo(
    () => COLUMNS.filter((c) => visibleColumns.includes(c.id)),
    [visibleColumns],
  );

  const onColumnsChange = (next: { key: string; visible: boolean }[]) => {
    const ordered = next
      .filter((c) => c.visible)
      .map((c) => c.key as ColumnId)
      .filter((id) => COLUMNS.some((c) => c.id === id));
    setVisibleColumns(ordered);
    void prefs.save(SURFACE, {
      ...saved,
      columns: next.map((c) => ({ key: c.key, visible: c.visible })),
    });
  };

  const createDisabled =
    creating || number.trim() === "" || name.trim() === "" || clientId === "";

  const pricingTotal = contractTotal(pricing, amount.trim() === "" ? null : Math.round(Number(amount) * 100));

  /** Forget the proposal: its draft, its rows and the base it was read from. */
  function resetChange(): void {
    setChangeInstruction("");
    setChangeOperations([]);
    setChangeSentence(null);
    setChangeDraft(null);
    setChangeTotal(null);
    setChangeBase(null);
    setProposed(new Set());
  }

  function resetDraft(): void {
    setPricing([]);
    setClauseRefs([]);
    setSources([]);
    setChecks([]);
    setUnderstanding(null);
    setTermMonths(null);
    setFromNotes(false);
    resetChange();
    setBuilderTab("summary");
  }

  /** The renewal path (C05): prefill the panel from the contract it starts at. */
  useEffect(() => {
    if (renewHandled.current || contracts === null) return;
    if (typeof window === "undefined") return;
    const id = new URLSearchParams(window.location.search).get("renew");
    if (id === null || id === "") return;
    const contract = contracts.find((c) => c.id === id);
    if (contract === undefined) return;
    renewHandled.current = true;
    const start = contract.effective_until ?? today();
    setRenewal({
      text: `Renew ${contract.name} for ${clientName(contract.client_id)} from ${formatPackDate(start) ?? start} on the same terms.`,
      using: [
        { label: `Contract ${contract.number}`, icon: "file" },
        { label: "12 months of tickets", icon: "ticket" },
        { label: "Usage and invoices", icon: "chart" },
      ],
    });
    setDescribeOpen(true);
  }, [contracts, clientName]);

  const createContract = async () => {
    setCreating(true);
    setCreateError(null);
    const monthly = pricingTotal > 0 ? pricingTotal : null;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateContractRequest"],
      ContractMutationResponse
    >({
      describe: "Create contract",
      entityId: `pending-contract:${number}`,
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        number: number.trim(),
        name: name.trim(),
        client_id: clientId,
        contract_type: type,
        currency: currency.trim() === "" ? "USD" : currency.trim(),
        effective_from: effectiveFrom,
        effective_until: effectiveUntil === "" ? null : effectiveUntil,
        amount_minor: monthly,
        billing_frequency:
          type === "recurring" || type === "retainer"
            ? (frequency as components["schemas"]["BillingFrequency"])
            : null,
      }),
      request: (client, body) => client.POST("/api/v1/contracts", { body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setCreating(false);
    if (!result.ok) {
      setCreateError(rejectionText(result, "The contract was rejected. Nothing was saved."));
      return;
    }
    setCreateOpen(false);
    setNumber("");
    setName("");
    setClientId("");
    setAmount("");
    setEffectiveUntil("");
    resetDraft();
    void refresh();
  };

  /**
   * The one draft both the change call and the summary read. The dates are the
   * authority for the term: a date the person edited after a draft applied must
   * not be replaced by the term the draft arrived with.
   */
  function currentDraft(): ContractDraft {
    return {
      name,
      contract_type: type,
      billing_frequency: frequency,
      value_minor: pricingTotal > 0 ? pricingTotal : null,
      currency,
      term_months: effectiveUntil === "" ? termMonths : monthsBetween(effectiveFrom, effectiveUntil),
      effective_from: effectiveFrom,
      notes: null,
      clauses: clauseRefs.map((clause) => clause.title),
      clause_refs: clauseRefs,
      pricing,
      client_id: clientId === "" ? null : clientId,
      client_name: clientId === "" ? null : clientName(clientId),
      assumptions: [],
      understanding,
      checks,
    };
  }

  // A proposal is only good for the draft it was described against. The
  // fingerprint is the draft's own content, so editing a rate, a date or a
  // clause after reading a proposal makes that proposal stale.
  const draftKey = JSON.stringify(currentDraft());
  const changeStale = changeDraft !== null && changeBase !== null && changeBase !== draftKey;
  const draftKeyRef = useRef(draftKey);
  useEffect(() => {
    draftKeyRef.current = draftKey;
  }, [draftKey]);

  function applyContract(draft: ContractDraft): void {
    setName(draft.name);
    if (TYPES.includes(draft.contract_type as ContractType)) setType(draft.contract_type as ContractType);
    if (draft.billing_frequency !== null && draft.billing_frequency !== undefined) {
      setFrequency(draft.billing_frequency);
    }
    if (draft.currency !== "") setCurrency(draft.currency);
    if (draft.client_id !== null && draft.client_id !== undefined) setClientId(draft.client_id);
    if (draft.effective_from !== null && draft.effective_from !== undefined) {
      setEffectiveFrom(draft.effective_from);
    }
    setTermMonths(draft.term_months ?? null);
    if (draft.term_months !== null && draft.term_months !== undefined) {
      setEffectiveUntil(addMonths(draft.effective_from ?? today(), draft.term_months));
    }
    setPricing(draft.pricing ?? []);
    setClauseRefs(draft.clause_refs ?? []);
    setAmount(
      draft.value_minor === null || draft.value_minor === undefined
        ? ""
        : (draft.value_minor / 100).toFixed(2),
    );
    setChecks(draft.checks ?? []);
    setUnderstanding(draft.understanding ?? null);
    setProposed(new Set());
  }

  async function proposeChange(): Promise<void> {
    // The draft this sentence is read against. A later request, or an edit
    // while this one runs, makes the answer unusable.
    const requestId = ++changeRequest.current;
    const base = draftKey;
    setChangeBusy(true);
    setChangeError(null);
    const outcome = await proposeContractChange(currentDraft(), changeInstruction.trim());
    if (changeRequest.current !== requestId) return;
    setChangeBusy(false);
    if (!outcome.ok) {
      setChangeError(outcome.message);
      return;
    }
    if (!outcome.result.valid || outcome.result.draft === null || outcome.result.draft === undefined) {
      setChangeError(
        outcome.result.issues[0]?.message ?? "I could not read a change in that sentence.",
      );
      return;
    }
    if (draftKeyRef.current !== base) {
      setChangeError("The draft changed while I read that. Describe the change again.");
      return;
    }
    setChangeSentence(outcome.result.sentence);
    setChangeOperations(outcome.result.operations);
    setChangeDraft(outcome.result.draft);
    setChangeTotal(outcome.result.resulting_total_minor);
    setChangeBase(base);
    setProposed(proposedPricingIndexes(outcome.result.operations.map((o) => o.path)));
  }

  function applyChange(): void {
    // Applying a proposal made against an older draft would undo every edit
    // since: refuse rather than silently replace them.
    if (changeDraft === null || changeDraft === undefined || changeStale) return;
    applyContract(changeDraft);
    resetChange();
  }

  function openBlank(): void {
    resetDraft();
    // A blank contract starts from the initial values, not from whatever a
    // cancelled draft left behind: an old amount would survive as an
    // unexplained manual price in a brand-new contract.
    setNumber("");
    setName("");
    setClientId("");
    setType("recurring");
    setCurrency("USD");
    setEffectiveFrom(today());
    setEffectiveUntil("");
    setAmount("");
    setFrequency("monthly");
    setCreateError(null);
    setCreateOpen(true);
  }

  // ---- keyboard ----------------------------------------------------------
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key.toLowerCase() !== "d") return;
      if (!granted) return;
      const target = event.target as HTMLElement | null;
      if (target !== null && /^(input|textarea|select)$/i.test(target.tagName)) return;
      if (createOpen) {
        setBuilderTab("describe");
        document.getElementById("change-composer")?.focus();
        return;
      }
      setDescribeOpen(true);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [createOpen, granted]);

  if (auth.status !== "authed") return null;

  const cell = (id: ColumnId) => visibleColumns.includes(id);

  return (
    <div className="m-dsc-shell">
      <div className="m-dsc-main">
        <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
          <SurfaceHeader
            title="Contracts"
            meta={
              <div className="flex flex-wrap items-center gap-2">
                <StatPill label="active" count={posture.active} active={view === "active"} dot="var(--m-canopy)" onToggle={() => setView(view === "active" ? "all" : "active")} />
                <StatPill
                  label="renewing soon"
                  count={posture.renewing}
                  active={view === "renewing"}
                  dot="var(--m-blush)"
                  onToggle={() => setView(view === "renewing" ? "all" : "renewing")}
                />
                <StatPill label="draft" count={posture.draft} active={false} dot="var(--m-input)" onToggle={() => undefined} />
                {posture.elapsed > 0 ? (
                  <StatPill label="term elapsed" count={posture.elapsed} active={false} dot="var(--m-danger)" onToggle={() => undefined} />
                ) : null}
              </div>
            }
            actions={
              <>
                <DescribeMoney
                  kind="contract"
                  available={granted}
                  onOpen={() => setDescribeOpen(true)}
                  testId="contracts-describe"
                />
                <NewMoneySplit
                  label="New contract"
                  menuLabel="More ways to start a contract"
                  testId="new-contract"
                  showDescribe={granted}
                  onBlank={openBlank}
                  onDescribe={() => setDescribeOpen(true)}
                />
              </>
            }
          />

          <div style={{ "--m-card-padding": "0px" } as React.CSSProperties}>
            <DataToolbar
              views={<ViewSwitch views={VIEWS} activeId={view} onChange={setView} label="Contract views" />}
              filters={
                <>
                  <div className="w-52">
                    <Input
                      aria-label="Search contracts"
                      placeholder="Search number, name or client…"
                      value={query}
                      onChange={(e) => setQuery(e.target.value)}
                    />
                  </div>
                  <div className="w-40">
                    <Select value={typeFilter} onValueChange={(v) => setTypeFilter(v as ContractType | "all")}>
                      <SelectTrigger aria-label="Filter by type">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All types</SelectItem>
                        {TYPES.map((t) => (
                          <SelectItem key={t} value={t}>
                            {TYPE_LABEL[t]}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="w-44">
                    <Select value={clientFilter} onValueChange={setClientFilter}>
                      <SelectTrigger aria-label="Filter by client">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All clients</SelectItem>
                        {clients.map((c) => (
                          <SelectItem key={c.id} value={c.id}>
                            {c.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </>
              }
              columns={
                <ColumnChooser
                  columns={COLUMNS.map((c) => ({
                    key: c.id,
                    label: c.label,
                    visible: visibleColumns.includes(c.id),
                  }))}
                  onChange={onColumnsChange}
                />
              }
              export={
                <ExportControl
                  columns={visible.map((c) => c.label)}
                  rowCount={filtered === null ? 0 : filtered.length}
                  onExport={() => {}}
                />
              }
            />
          </div>

          <div
            className="overflow-hidden rounded-2xl border"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          >
            {loadError !== null ? (
              <div className="p-4">
                <ErrorNote>{loadError}</ErrorNote>
              </div>
            ) : filtered === null ? (
              <div className="p-4">
                <Spinner />
              </div>
            ) : filtered.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No contracts match this view. A contract is what an invoice run bills against." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    {cell("number") ? <Th>Number</Th> : null}
                    {cell("name") ? <Th>Name</Th> : null}
                    {cell("client") ? <Th>Client</Th> : null}
                    {cell("type") ? <Th>Type</Th> : null}
                    {cell("status") ? <Th>Status</Th> : null}
                    {cell("term") ? <Th>Term</Th> : null}
                    {cell("amount") ? <Th className="text-right">Per period</Th> : null}
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((contract, i) => {
                    const signal = renewalSignal(contract.effective_until, now);
                    return (
                      <Tr
                        key={contract.id}
                        className="m-entity-row"
                        style={{ animationDelay: `${Math.min(i * 20, 160)}ms` }}
                        data-testid={`contract-row-${contract.id}`}
                      >
                        {cell("number") ? (
                          <Td>
                            <Link
                              href={`/contracts/${contract.id}`}
                              className="text-[13px] underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]"
                            >
                              {contract.number}
                            </Link>
                          </Td>
                        ) : null}
                        {cell("name") ? <Td className="font-medium">{contract.name}</Td> : null}
                        {cell("client") ? (
                          <Td>
                            <Link href={`/clients/${contract.client_id}`} className="text-[13px] underline underline-offset-2">
                              {clientName(contract.client_id)}
                            </Link>
                          </Td>
                        ) : null}
                        {cell("type") ? (
                          <Td>
                            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                              {TYPE_LABEL[contract.contract_type]}
                            </span>
                          </Td>
                        ) : null}
                        {cell("status") ? (
                          <Td>
                            <Badge tone={statusTone(contract.status)}>
                              {STATUS_LABEL[contract.status] ?? contract.status}
                            </Badge>
                          </Td>
                        ) : null}
                        {cell("term") ? (
                          <Td>
                            <span className="flex flex-wrap items-center gap-2">
                              <span className="text-[13px] tabular-nums">
                                {formatPackDate(contract.effective_from) ?? "—"}
                                {" → "}
                                {contract.effective_until != null && contract.effective_until !== ""
                                  ? formatPackDate(contract.effective_until)
                                  : "open"}
                              </span>
                              {signal !== null ? (
                                <Badge tone={signal.tone === "danger" ? "danger" : "due"}>
                                  <span className="inline-flex items-center gap-1">
                                    <Icon name="clock" size={13} />
                                    {signal.label}
                                  </span>
                                </Badge>
                              ) : null}
                            </span>
                          </Td>
                        ) : null}
                        {cell("amount") ? (
                          <Td className="text-right text-[13px] font-semibold tabular-nums">
                            {contract.amount_minor != null ? (
                              <>
                                {money(contract.amount_minor, contract.currency)}
                                <span className="ml-1 font-normal" style={{ color: "var(--m-text-muted)" }}>
                                  / {contract.billing_frequency ?? "period"}
                                </span>
                              </>
                            ) : (
                              <span style={{ color: "var(--m-text-muted)" }}>—</span>
                            )}
                          </Td>
                        ) : null}
                      </Tr>
                    );
                  })}
                </tbody>
              </Table>
            )}
          </div>

          <SurfaceFooter
            shown={filtered?.length ?? 0}
            total={
              filtered !== null && contracts !== null && filtered.length !== contracts.length
                ? contracts.length
                : undefined
            }
            noun="contract"
            density={<DensitySwitch value={prefs.density} onChange={(step) => void prefs.setDensity(step)} variant="icons" label="Row density" />}
            trailing={<span className="tabular-nums">Booked per period: {posture.book}</span>}
          />
        </div>
      </div>

      {granted ? (
        <DescribePanel
          kind="contract"
          open={describeOpen}
          onClose={() => setDescribeOpen(false)}
          clients={clients.map((client) => ({ id: client.id, name: client.name }))}
          prefilled={renewal}
          onBuild={(draft, nextSources, nextKind) => {
            if (nextKind !== "contract" || "lines" in draft) return;
            applyContract(draft);
            setSources(nextSources);
            setFromNotes(true);
            setBuilderTab("summary");
            setCreateOpen(true);
          }}
          onReady={() => setDescribeOpen(true)}
          onStartBlank={(nextSources) => {
            openBlank();
            setSources(nextSources);
            setFromNotes(true);
            setBuilderTab(nextSources.length > 0 ? "notes" : "summary");
          }}
          testId="contracts-describe-panel"
        />
      ) : null}

      <Dialog
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) setCreateError(null);
        }}
      >
        <DialogContent
          title="Contract builder"
          description="A contract is the commercial agreement an invoice run bills against. Draft contracts preview and are refused by invoice runs."
        >
          <div className="grid max-h-[84vh] gap-4 overflow-auto p-4" data-testid="contract-builder">
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="ct-number">Number</Label>
                <Input id="ct-number" value={number} onChange={(e) => setNumber(e.target.value)} placeholder="CTR-2026-04" />
              </div>
              <div>
                <Label htmlFor="ct-name">Name</Label>
                <Input id="ct-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Managed Gold" />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="ct-client">Client</Label>
                <Select value={clientId} onValueChange={setClientId}>
                  <SelectTrigger id="ct-client" className="w-full">
                    <SelectValue placeholder="Choose a client" />
                  </SelectTrigger>
                  <SelectContent>
                    {clients.map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label htmlFor="ct-type">Type</Label>
                <Select value={type} onValueChange={(v) => setType(v as ContractType)}>
                  <SelectTrigger id="ct-type" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {TYPES.map((t) => (
                      <SelectItem key={t} value={t}>
                        {TYPE_LABEL[t]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="ct-from">Effective from</Label>
                <Input id="ct-from" type="date" value={effectiveFrom} onChange={(e) => setEffectiveFrom(e.target.value)} />
              </div>
              <div>
                <Label htmlFor="ct-until">Effective until</Label>
                <Input id="ct-until" type="date" value={effectiveUntil} onChange={(e) => setEffectiveUntil(e.target.value)} />
                <FieldHint>Empty means open-ended: there is nothing to renew.</FieldHint>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="ct-amount">Per-period price</Label>
                <Input
                  id="ct-amount"
                  type="number"
                  min={0}
                  step="0.01"
                  value={pricing.length > 0 ? (pricingTotal / 100).toFixed(2) : amount}
                  onChange={(e) => {
                    setPricing([]);
                    setAmount(e.target.value);
                  }}
                  placeholder="Optional"
                  data-testid="ct-amount"
                />
                <FieldHint>
                  {pricing.length > 0
                    ? "Built from the quantities and rates below."
                    : "Type a price, or let a described draft build it from quantities."}
                </FieldHint>
              </div>
              <div>
                <Label htmlFor="ct-freq">Billing frequency</Label>
                <Select value={frequency} onValueChange={setFrequency}>
                  <SelectTrigger id="ct-freq" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {["weekly", "monthly", "quarterly", "semiannual", "annual"].map((f) => (
                      <SelectItem key={f} value={f}>
                        {f}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FieldHint>Applies to recurring and retainer contracts.</FieldHint>
              </div>
            </div>

            <div className="m-dsc-builder">
              <div className="m-dsc-builder-main">
                {fromNotes && checks.length > 0 ? (
                  <div className="m-dsc-legend">
                    <span>
                      <span className="m-dsc-dot" data-source="notes" aria-hidden="true" /> From your words
                    </span>
                    <span>
                      <span className="m-dsc-dot" data-source="assumed" aria-hidden="true" /> Assumed, please check
                    </span>
                    <span className="m-dsc-sp" />
                    <button
                      type="button"
                      className="m-dsc-option"
                      onClick={() => setBuilderTab("summary")}
                      data-testid="contract-checks-count"
                    >
                      {checks.length} to check
                    </button>
                  </div>
                ) : null}

                {pricing.length > 0 ? (
                  <div className="m-dsc-panel-card">
                    <div className="m-dsc-panel-head">
                      <h3>Pricing</h3>
                    </div>
                    <div className="p-3">
                      <ContractPricing
                        pricing={pricing}
                        currency={currency}
                        frequency={frequency}
                        proposed={proposed}
                        onChange={(index, patch) =>
                          setPricing((cur) => cur.map((line, i) => (i === index ? { ...line, ...patch } : line)))
                        }
                      />
                    </div>
                  </div>
                ) : null}

                {clauseRefs.length > 0 || library.length > 0 ? (
                  <div className="m-dsc-panel-card" data-testid="contract-clause-card">
                    <div className="m-dsc-panel-head">
                      <h3>Clauses</h3>
                      <span className="flex-1" />
                      <span className="m-dsc-hint">From your approved library</span>
                    </div>
                    <div className="p-3">
                      <ClauseList draft={currentDraft()} library={library} />
                    </div>
                  </div>
                ) : null}

                {pricing.length === 0 && clauseRefs.length === 0 ? (
                  <div className="m-dsc-panel-card p-4">
                    <p className="m-dsc-hint">
                      This contract is being built by hand. Describe it to fill the parties, the
                      pricing and the clauses from your approved library.
                    </p>
                  </div>
                ) : null}

                {fromNotes ? (
                  <div className="m-dsc-bar" data-testid="contract-change-bar">
                    <span className="flex flex-1 items-center gap-2" style={{ fontSize: 14 }}>
                      <span className="m-dsc-pill m-dsc-pill-muted">D</span>
                      Change something, or add a clause value
                    </span>
                    <button
                      type="button"
                      className="m-dsc-option"
                      onClick={() => setBuilderTab("describe")}
                    >
                      Describe a change
                    </button>
                  </div>
                ) : null}
              </div>

              <QuoteContextColumn
                tab={fromNotes ? builderTab : "summary"}
                onTab={setBuilderTab}
                showTabs={fromNotes}
                summary={
                  <ContractSummary
                    draft={currentDraft()}
                    currency={currency}
                    library={library}
                    checks={checks}
                    understanding={understanding}
                  />
                }
                notes={
                  <ContractNotes sources={sources} pricing={pricing} currency={currency} />
                }
                describe={
                  <ChangeProposalCard
                    instruction={changeInstruction}
                    sentence={changeSentence}
                    operations={changeOperations}
                    resultingTotalMinor={changeTotal}
                    currency={currency}
                    busy={changeBusy}
                    stale={changeStale}
                    error={changeError}
                    onInstruction={setChangeInstruction}
                    onSubmit={() => void proposeChange()}
                    onApply={applyChange}
                    onDismiss={resetChange}
                  />
                }
              />
            </div>

            {createError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {createError}
              </p>
            ) : null}
            <div className="flex items-center gap-2">
              <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <span className="flex-1" />
              <Button onClick={() => void createContract()} disabled={createDisabled} data-testid="contract-save">
                {creating ? <Spinner /> : null} Create contract
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );

}

function ContractNotes({
  sources,
  pricing,
  currency,
}: {
  sources: MoneySource[];
  pricing: ContractPriceLine[];
  currency: string;
}): ReactNode {
  if (sources.length === 0) {
    return (
      <p className="m-dsc-hint">
        This contract was built by hand. There are no words behind its clauses.
      </p>
    );
  }
  return (
    <div className="grid gap-3" data-testid="contract-source-notes">
      <div className="m-dsc-chips">
        {sources.map((source) => (
          <span className="m-dsc-chip m-dsc-chip-neutral" key={source.id ?? source.name}>
            <Icon name={source.kind === "transcript" ? "mic" : "file"} size={13} />
            {source.name ?? source.kind}
          </span>
        ))}
      </div>
      {sources.map((source) => (
        <div key={source.id ?? source.name}>
          <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
            {source.name ?? source.kind}
          </p>
          <p className="m-dsc-notes">{source.text}</p>
        </div>
      ))}
      {pricing.length > 0 ? (
        <p className="m-dsc-hint">
          The pricing came from quantities in these words: {pricing.map((line) => line.label).join(", ")}.
          The total is {formatMoneyMinor(contractTotal(pricing, null), currency)} a period.
        </p>
      ) : null}
    </div>
  );
}

/** Header stat pill, as on the clients and projects ledgers. */
function StatPill({
  label,
  count,
  active,
  dot,
  onToggle,
}: {
  label: string;
  count: number;
  active: boolean;
  dot: string;
  onToggle: () => void;
}): ReactNode {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onToggle}
      data-testid={`contracts-stat-${label.replaceAll(" ", "-")}`}
      className="inline-flex cursor-pointer items-center gap-2 rounded-full border px-3 py-1.5 text-[13px] font-semibold"
      style={{
        background: active ? "var(--m-accent)" : "transparent",
        color: active ? "var(--m-on-accent)" : "var(--m-text)",
        borderColor: active ? "transparent" : "var(--m-border)",
      }}
    >
      <span className="h-2 w-2 rounded-full" style={{ background: dot }} aria-hidden="true" />
      {label}
      <span className="tabular-nums">{count}</span>
    </button>
  );
}

function moneyOf(list: Contract[]): string {
  const priced = list
    .filter((c) => c.status === "active" && c.amount_minor != null)
    .map((c) => ({ currency: c.currency, amount: c.amount_minor as number }));
  if (priced.length === 0) return "—";
  const currencies = [...new Set(priced.map((p) => p.currency))];
  if (currencies.length > 1) return "mixed currencies";
  const total = priced.reduce((acc, p) => acc + p.amount, 0);
  return money(total, currencies[0] as string);
}

function statusTone(status: string): "success" | "warning" | "danger" | "neutral" {
  switch (status) {
    case "active":
      return "success";
    case "draft":
      return "warning";
    case "cancelled":
      return "danger";
    default:
      return "neutral";
  }
}

function monthsBetween(from: string, until: string): number | null {
  const start = new Date(`${from}T00:00:00`).getTime();
  const end = new Date(`${until}T00:00:00`).getTime();
  if (Number.isNaN(start) || Number.isNaN(end)) return null;
  return Math.max(1, Math.round((end - start) / (86_400_000 * 30.44)));
}
