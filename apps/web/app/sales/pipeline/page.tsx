// /sales/pipeline — the sales pipeline board (Wave 12 lane 2 backends,
// migration 0060).
//
// Purpose: the org's sales pipeline as a board: one column per active
// pipeline stage (open stages first, then won/lost), deal cards with
// client + amount, stage moves through POST /pipeline/deals/{id}/stage
// (one audited transition), and the deal composer (client + stage + name +
// amount). Reads hydrate once from the SDK; mutations ride the IF1–IF3
// envelope (optimistic render, authoritative reconcile from the mutation
// response).
//
// Non-purpose: no stage administration beyond reordering visibility (stage
// create lives with the composer; quotes join deals via quote_id on the
// deal detail). Currency entry is minor units (cents).
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";

type PipelineDeal = components["schemas"]["PipelineDeal"];
type PipelineStage = components["schemas"]["PipelineStage"];
type DealStatus = components["schemas"]["DealStatus"];

const STATUS_TONE: Record<DealStatus, "neutral" | "success" | "danger"> = {
  open: "neutral",
  won: "success",
  lost: "danger",
};

export default function PipelinePage(): ReactNode {
  const auth = useRequireAuth();
  const [stages, setStages] = useState<PipelineStage[] | null>(null);
  const [deals, setDeals] = useState<PipelineDeal[] | null>(null);
  const [clients, setClients] = useState<components["schemas"]["Client"][] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);

  const [createOpen, setCreateOpen] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newClientId, setNewClientId] = useState<string>("");
  const [newStageId, setNewStageId] = useState<string>("");
  const [newAmount, setNewAmount] = useState("");
  const [newCurrency, setNewCurrency] = useState("USD");
  const [createBusy, setCreateBusy] = useState(false);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const [stageRes, dealRes, clientRes] = await Promise.all([
        api.api.GET("/api/v1/pipeline/stages"),
        api.api.GET("/api/v1/pipeline/deals"),
        api.api.GET("/api/v1/clients"),
      ]);
      if (stageRes.data === undefined || dealRes.data === undefined) {
        setLoadError("Could not load the pipeline — try reloading.");
        return;
      }
      setStages(stageRes.data ?? []);
      setDeals(dealRes.data ?? []);
      setClients(clientRes.data ?? []);
      const firstOpen = (stageRes.data ?? []).find((s) => s.stage_type === "open");
      setNewStageId(firstOpen?.id ?? "");
    })();
  }, [auth.status]);

  const orderedStages = useMemo(() => {
    const list = stages ?? [];
    const open = list.filter((s) => s.stage_type === "open").sort((a, b) => a.position - b.position);
    const won = list.filter((s) => s.stage_type === "won");
    const lost = list.filter((s) => s.stage_type === "lost");
    return [...open, ...won, ...lost];
  }, [stages]);

  const moveDeal = useCallback(
    async (deal: PipelineDeal, toStageId: string) => {
      if (toStageId === deal.stage_id) return;
      setMoveError(null);
      const result = await runEnvelopeMutation({
        describe: "Move deal stage",
        entityId: deal.id,
        buildBody: (base) => ({ to_stage_id: toStageId, ...base }),
        optimistic: () => {
          setDeals((cur) =>
            (cur ?? []).map((d) =>
              d.id === deal.id
                ? { ...d, stage_id: toStageId, updated_at: new Date().toISOString() }
                : d,
            ),
          );
        },
        revert: () => {
          setDeals((cur) =>
            (cur ?? []).map((d) => (d.id === deal.id ? { ...d, stage_id: deal.stage_id } : d)),
          );
        },
        request: (client, body) =>
          client.POST("/api/v1/pipeline/deals/{id}/stage", {
            params: { path: { id: deal.id } },
            body: body as components["schemas"]["MoveStageRequest"],
          }),
      });
      if (result.ok === false) {
        setMoveError(result.message ?? "The stage move was rejected.");
      }
    },
    [],
  );

  const createDeal = async (): Promise<void> => {
    setCreateError(null);
    const name = newName.trim();
    if (name === "" || newClientId === "" || newStageId === "") {
      setCreateError("Name, client, and stage are required.");
      return;
    }
    const amountMajor = Number(newAmount);
    const tempId = `pending-${Date.now()}`;
    setCreateBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Create deal",
      buildBody: (base) => ({
        name,
        client_id: newClientId,
        stage_id: newStageId,
        currency: newCurrency,
        amount_minor: newAmount !== "" && !Number.isNaN(amountMajor) ? Math.round(amountMajor * 100) : undefined,
        ...base,
      }),
      optimistic: () => {
        setDeals((cur) => [
          ...(cur ?? []),
          {
            id: tempId,
            org_id: "",
            client_id: newClientId,
            stage_id: newStageId,
            name,
            amount_minor: Number.isNaN(amountMajor) ? 0 : Math.round(amountMajor * 100),
            currency: newCurrency,
            status: "open" as DealStatus,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            closed_at: null,
            created_by: null,
            owner_user_id: null,
            quote_id: null,
            expected_close_on: null,
            notes: null,
          },
        ]);
      },
      revert: () => {
        setDeals((cur) => (cur ?? []).filter((d) => d.id !== tempId));
      },
      request: (client, body) =>
        client.POST("/api/v1/pipeline/deals", {
          body: body as components["schemas"]["CreateDealRequest"],
        }),
    });
    setCreateBusy(false);
    if (result.ok === false) {
      setCreateError(result.message ?? "Could not create the deal.");
      return;
    }
    setCreateOpen(false);
    setNewName("");
    setNewAmount("");
    void (async () => {
      const { data } = await api.api.GET("/api/v1/pipeline/deals");
      setDeals(data ?? []);
    })();
  };

  if (stages === null && loadError === null) {
    return <Spinner />;
  }
  if (loadError !== null) {
    return (
      <EmptyState message={loadError} />
    );
  }

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Sales pipeline
          </h1>
          <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Deals move through pipeline stages; won and lost are terminal.
          </p>
        </div>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger asChild>
            <Button data-testid="pipeline-new-deal">New deal</Button>
          </DialogTrigger>
          <DialogContent title="New deal">
            <Card className="grid gap-3 p-4">
              <h2 className="text-sm font-semibold">New deal</h2>
              <div className="grid gap-1">
                <Label htmlFor="deal-name">Name</Label>
                <Input id="deal-name" value={newName} onChange={(e) => setNewName(e.target.value)} data-testid="pipeline-deal-name" />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="deal-client">Client</Label>
                <Select value={newClientId} onValueChange={(v) => setNewClientId(v)}>
                  <SelectTrigger id="deal-client" data-testid="pipeline-deal-client">
                    <SelectValue placeholder="Choose client" />
                  </SelectTrigger>
                  <SelectContent>
                    {(clients ?? []).map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="deal-stage">Stage</Label>
                <Select value={newStageId} onValueChange={(v) => setNewStageId(v)}>
                  <SelectTrigger id="deal-stage" data-testid="pipeline-deal-stage">
                    <SelectValue placeholder="Choose stage" />
                  </SelectTrigger>
                  <SelectContent>
                    {orderedStagesForCreate(stages).map((s) => (
                      <SelectItem key={s.id} value={s.id}>
                        {s.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid grid-cols-[1fr_120px] gap-2">
                <div className="grid gap-1">
                  <Label htmlFor="deal-amount">Amount</Label>
                  <Input
                    id="deal-amount"
                    inputMode="decimal"
                    placeholder="0.00"
                    value={newAmount}
                    onChange={(e) => setNewAmount(e.target.value)}
                  />
                  <FieldHint>Major units; stored as minor units.</FieldHint>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="deal-currency">Currency</Label>
                  <Input id="deal-currency" value={newCurrency} onChange={(e) => setNewCurrency(e.target.value.toUpperCase())} maxLength={3} />
                </div>
              </div>
              {createError !== null && (
                <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="pipeline-create-error">
                  {createError}
                </p>
              )}
              <DialogActions>
                <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                  Cancel
                </Button>
                <Button disabled={createBusy} onClick={() => void createDeal()} data-testid="pipeline-deal-save">
                  Create
                </Button>
              </DialogActions>
            </Card>
          </DialogContent>
        </Dialog>
      </header>

      {moveError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="pipeline-move-error">
          {moveError}
        </p>
      )}

      <div
        className="grid gap-3"
        style={{
          gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))",
        }}
      >
        {orderedStages.map((stage) => {
          const stageDeals = (deals ?? []).filter((d) => d.stage_id === stage.id);
          return (
            <section
              key={stage.id}
              className="grid content-start gap-2 rounded-[4px] border p-2"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              data-testid={`pipeline-stage-${stage.name}`}
              aria-label={`Stage ${stage.name}`}
            >
              <div className="flex items-center justify-between gap-2">
                <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  {stage.name}
                </h2>
                <StatusTag tone={STATUS_TONE[stage.stage_type]}>{stage.stage_type}</StatusTag>
              </div>
              {stageDeals.map((deal) => (
                <article
                  key={deal.id}
                  className="grid gap-1 rounded-[3px] border p-2 text-xs"
                  style={{ borderColor: "var(--m-border)" }}
                  data-testid={`pipeline-deal-${deal.id}`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="min-w-0 flex-1 truncate font-medium" title={deal.name}>
                      {deal.name}
                    </span>
                    <span style={{ color: "var(--m-text)" }}>{money(deal.amount_minor, deal.currency)}</span>
                  </div>
                  {deal.status !== "open" && (
                    <StatusTag tone={STATUS_TONE[deal.status]}>{deal.status}</StatusTag>
                  )}
                  <div className="flex items-center gap-1">
                    <Select
                      value={deal.stage_id}
                      onValueChange={(v) => void moveDeal(deal, v)}
                      disabled={deal.status !== "open"}
                    >
                      <SelectTrigger aria-label={`Move ${deal.name}`} data-testid={`pipeline-move-${deal.id}`}>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {orderedStages.map((s) => (
                          <SelectItem key={s.id} value={s.id}>
                            {s.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <Link
                    href={`/clients/${deal.client_id}`}
                    className="underline underline-offset-2"
                    style={{ color: "var(--m-accent)" }}
                  >
                    Client record →
                  </Link>
                </article>
              ))}
              {stageDeals.length === 0 && (
                <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  No deals.
                </p>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}

function orderedStagesForCreate(stages: PipelineStage[] | null): PipelineStage[] {
  return (stages ?? []).slice().sort((a, b) => a.position - b.position);
}
