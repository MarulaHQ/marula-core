// /settings/toolboxes — the config home for toolboxes and dispatch defaults
// (Wave 32 lane 8, `p32-today-day-pack`).
//
// Purpose: three jobs in one place.
//   1. Toolboxes: create/edit a kit, set membership (which tools live in
//      which toolbox), and print a box tag whose QR or code scans back to
//      the kit (lane 5's renderer, lane 5's scan contract).
//   2. Dispatch defaults: the typed org mapping behind the day pack's
//      recommendations — a ticket type / ticket category / board maps to
//      items ("network ⇒ cable tester + punch-down") or to a default kit
//      ("dispatch the field kit with every network ticket").
//   3. Scan test: charge the bar with a toolbox tag and the editor selects
//      the kit, so a printed label proves itself before it goes on a box.
//
// Non-purpose: no checkout here (the stock room surface owns that), no
// label template design (settings → Labels & scanning).
"use client";

import {
  Badge,
  Button,
  Card,
  EmptyState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useMemo, useState } from "react";

import { LabelPrintButton } from "../../../components/labels/label-print-button";
import { ScanBar } from "../../../components/scan/scan-bar";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { api, useRequireAuth } from "../../../lib/api";
import {
  createPackRule,
  deletePackRule,
  fetchPackRules,
  qtyText,
  setPackRuleActive,
  type PackRuleView,
} from "../../../lib/day-pack";
import { friendlyMutationError, rejectionText } from "../../../lib/error-copy";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { canManageOnboarding } from "../../../lib/onboarding";
import { scanSummary } from "../../../lib/scan/routing";

type ToolKitView = components["schemas"]["ToolKitView"];
type KitDetailResponse = components["schemas"]["KitDetailResponse"];
type KitAvailabilityResponse = components["schemas"]["KitAvailabilityResponse"];
type StockItemView = components["schemas"]["StockItemView"];

interface ScopeOption {
  id: string;
  name: string;
}

const SCOPE_KIND_LABELS: Record<string, string> = {
  ticket_type: "Ticket type",
  ticket_category: "Ticket category",
  board: "Board",
};

export default function ToolboxesSettingsPage(): React.ReactNode {
  const auth = useRequireAuth();
  const isAdmin = canManageOnboarding(auth.me);

  const [kits, setKits] = useState<ToolKitView[] | null>(null);
  const [selectedId, setSelectedId] = useState<string>("");
  const [detail, setDetail] = useState<KitDetailResponse | null>(null);
  const [availability, setAvailability] = useState<KitAvailabilityResponse | null>(null);
  const [items, setItems] = useState<StockItemView[]>([]);
  const [boards, setBoards] = useState<ScopeOption[]>([]);
  const [types, setTypes] = useState<ScopeOption[]>([]);
  const [categories, setCategories] = useState<ScopeOption[]>([]);
  const [rules, setRules] = useState<PackRuleView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scanLine, setScanLine] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [newKitName, setNewKitName] = useState("");
  const [newKitCode, setNewKitCode] = useState("");
  const [memberItemId, setMemberItemId] = useState("");
  const [memberQty, setMemberQty] = useState("1");

  const [ruleScopeKind, setRuleScopeKind] = useState("ticket_category");
  const [ruleScopeId, setRuleScopeId] = useState("");
  const [ruleTargetKind, setRuleTargetKind] = useState("item");
  const [ruleItemId, setRuleItemId] = useState("");
  const [ruleKitId, setRuleKitId] = useState("");
  const [ruleQty, setRuleQty] = useState("1");
  const [ruleNote, setRuleNote] = useState("");

  const refreshKits = useCallback(async () => {
    const { data, error: err } = await api.api.GET("/api/v1/tool-kits", {
      params: { query: { limit: 200 } },
    });
    if (err !== undefined || data === undefined) {
      setError("The toolboxes could not be loaded.");
      return;
    }
    setKits(data);
    setSelectedId((current) => current || data[0]?.id || "");
  }, []);

  const refreshRules = useCallback(async () => {
    try {
      setRules(await fetchPackRules());
    } catch (err) {
      setError(friendlyMutationError(err).text);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refreshKits();
    void refreshRules();
    void (async () => {
      const [itemsRes, navRes, typesRes, categoriesRes] = await Promise.all([
        api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
        api.api.GET("/api/v1/boards/nav"),
        api.api.GET("/api/v1/taxonomy/types", { params: { query: {} } }),
        api.api.GET("/api/v1/taxonomy/categories", { params: { query: {} } }),
      ]);
      setItems(itemsRes.data ?? []);
      setBoards((navRes.data ?? []) as unknown as ScopeOption[]);
      setTypes((typesRes.data ?? []) as unknown as ScopeOption[]);
      setCategories((categoriesRes.data ?? []) as unknown as ScopeOption[]);
    })();
  }, [auth.status, refreshKits, refreshRules]);

  useEffect(() => {
    if (selectedId === "") {
      setDetail(null);
      setAvailability(null);
      return;
    }
    void (async () => {
      const [detailRes, availabilityRes] = await Promise.all([
        api.api.GET("/api/v1/tool-kits/{id}", { params: { path: { id: selectedId } } }),
        api.api.GET("/api/v1/tool-kits/{id}/availability", {
          params: { path: { id: selectedId }, query: {} },
        }),
      ]);
      setDetail(detailRes.data ?? null);
      setAvailability(availabilityRes.data ?? null);
    })();
  }, [selectedId]);

  const selected = useMemo(
    () => (kits ?? []).find((kit) => kit.id === selectedId) ?? null,
    [kits, selectedId],
  );

  const scopeOptions =
    ruleScopeKind === "board" ? boards : ruleScopeKind === "ticket_type" ? types : categories;

  const toolItems = useMemo(
    () => items.filter((item) => item.active && item.is_tool),
    [items],
  );
  const activeItems = useMemo(() => items.filter((item) => item.active), [items]);

  const readiness = useMemo(() => {
    const byId = new Map<string, { ready: boolean; available: number; required: number }>();
    for (const member of availability?.members ?? []) {
      byId.set(member.stock_item_id, {
        ready: member.ready,
        available: member.available_units,
        required: member.required_units,
      });
    }
    return byId;
  }, [availability]);

  async function createKit(): Promise<void> {
    if (newKitName.trim() === "") {
      setError("Name the toolbox first.");
      return;
    }
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
      describe: "create a toolbox",
      buildBody: (base) => ({
        name: newKitName.trim(),
        code: newKitCode.trim() === "" ? null : newKitCode.trim(),
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) => client.POST("/api/v1/tool-kits", { body: body as never }),
    });
    setBusy(false);
    if (result.ok === false) {
      setError(result.friendly ?? "The toolbox was not created.");
      return;
    }
    setNewKitName("");
    setNewKitCode("");
    await refreshKits();
  }

  async function patchKit(patch: {
    name?: string;
    code?: string | null;
    active?: boolean;
  }): Promise<void> {
    if (selected === null) return;
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
      describe: "update the toolbox",
      entityId: selected.id,
      buildBody: (base) => ({ ...patch, ...base }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.PATCH("/api/v1/tool-kits/{id}", {
          params: { path: { id: selected.id } },
          body: body as never,
        }),
    });
    setBusy(false);
    if (result.ok === false) {
      setError(result.friendly ?? "The toolbox was not updated.");
      return;
    }
    await refreshKits();
    setSelectedId(selected.id);
  }

  async function addMember(): Promise<void> {
    if (selected === null || memberItemId === "") {
      setError("Pick a tool to add to the toolbox.");
      return;
    }
    const qty = Number.parseFloat(memberQty);
    if (!Number.isFinite(qty) || qty <= 0) {
      setError("Member quantity must be positive.");
      return;
    }
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
      describe: "add a toolbox member",
      entityId: selected.id,
      buildBody: (base) => ({
        stock_item_id: memberItemId,
        qty_milli: Math.round(qty * 1000),
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/tool-kits/{id}/members", {
          params: { path: { id: selected.id } },
          body: body as never,
        }),
    });
    setBusy(false);
    if (result.ok === false) {
      setError(result.friendly ?? "The member was not added.");
      return;
    }
    setMemberItemId("");
    setSelectedId(selected.id);
  }

  async function removeMember(memberId: string): Promise<void> {
    if (selected === null) return;
    setBusy(true);
    setError(null);
    const { error: err } = await api.api.DELETE("/api/v1/tool-kits/{id}/members/{member_id}", {
      params: { path: { id: selected.id, member_id: memberId } },
    });
    setBusy(false);
    if (err !== undefined) {
      setError(rejectionText(err, "The member was not removed."));
      return;
    }
    setSelectedId(selected.id);
  }

  async function addRule(): Promise<void> {
    if (ruleScopeId === "") {
      setError(`Pick a ${SCOPE_KIND_LABELS[ruleScopeKind]?.toLowerCase() ?? "scope"}.`);
      return;
    }
    if (ruleTargetKind === "item" && ruleItemId === "") {
      setError("Pick the item this scope recommends.");
      return;
    }
    if (ruleTargetKind === "kit" && ruleKitId === "") {
      setError("Pick the kit this scope dispatches.");
      return;
    }
    const qty = Number.parseFloat(ruleQty);
    if (!Number.isFinite(qty) || qty <= 0) {
      setError("Quantity must be positive.");
      return;
    }
    setBusy(true);
    setError(null);
    const failure = await createPackRule({
      scope_kind: ruleScopeKind,
      scope_id: ruleScopeId,
      target_kind: ruleTargetKind,
      stock_item_id: ruleTargetKind === "item" ? ruleItemId : null,
      kit_id: ruleTargetKind === "kit" ? ruleKitId : null,
      qty_milli: Math.round(qty * 1000),
      note: ruleNote.trim() === "" ? null : ruleNote.trim(),
    });
    setBusy(false);
    if (failure !== null) {
      setError(failure);
      return;
    }
    setRuleItemId("");
    setRuleKitId("");
    setRuleNote("");
    await refreshRules();
  }

  async function toggleRule(rule: PackRuleView): Promise<void> {
    setError(null);
    const failure = await setPackRuleActive(rule.id, !rule.active);
    if (failure !== null) setError(failure);
    await refreshRules();
  }

  async function removeRule(rule: PackRuleView): Promise<void> {
    setError(null);
    const failure = await deletePackRule(rule.id);
    if (failure !== null) setError(failure);
    await refreshRules();
  }

  const targetLabel = (rule: PackRuleView): string => {
    if (rule.target_kind === "kit") {
      return `${rule.kit_name ?? "Kit"} (default kit)`;
    }
    const qty = qtyText(rule.qty_milli);
    return `${qty}× ${rule.item_name ?? "Item"}`;
  };

  return (
    <div>
      <SettingsHeader
        title="Toolboxes & day pack"
        description="Toolbox kits, their members, and the typed dispatch defaults behind /today's grab list. Print a box tag here; scanning it selects the kit."
      />

      <div className="grid gap-3">
        {error !== null ? (
          <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        ) : null}

        <Card className="p-3">
          <ScanBar
            surface="toolboxes"
            hint="Test a toolbox tag: scan its QR or type the kit code, then press Enter."
            onScan={({ resolution }) => {
              setScanLine(scanSummary(resolution));
              if (
                resolution.kind === "tool_kit" &&
                resolution.entity_id !== null &&
                resolution.entity_id !== undefined
              ) {
                setSelectedId(resolution.entity_id);
              }
            }}
            onRejected={(message) => setScanLine(message)}
          />
          {scanLine !== null ? (
            <p className="mt-2 text-xs" data-testid="toolboxes-scan-result" style={{ color: "var(--m-text-secondary)" }}>
              {scanLine}
            </p>
          ) : null}
        </Card>

        <div className="grid gap-3 lg:grid-cols-[320px_minmax(0,1fr)]">
          <Card className="content-start p-3">
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Toolboxes
            </h2>
            {kits === null ? (
              <div className="flex justify-center p-4">
                <Spinner />
              </div>
            ) : kits.length === 0 ? (
              <EmptyState message="No toolboxes yet. Create the field kit below." />
            ) : (
              <ul className="grid gap-1" role="list">
                {kits.map((kit) => (
                  <li key={kit.id}>
                    <button
                      type="button"
                      data-testid="toolbox-row"
                      onClick={() => setSelectedId(kit.id)}
                      className="flex w-full cursor-pointer items-center justify-between gap-2 rounded-[var(--m-radius-control)] px-2 py-1.5 text-left text-xs"
                      style={
                        kit.id === selectedId
                          ? { background: "var(--m-surface)", color: "var(--m-text)" }
                          : { color: "var(--m-text-secondary)" }
                      }
                    >
                      <span className="min-w-0 truncate">{kit.name}</span>
                      <span className="flex shrink-0 items-center gap-1.5">
                        {kit.code !== null && kit.code !== undefined && kit.code !== "" ? (
                          <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                            {kit.code}
                          </span>
                        ) : null}
                        {!kit.active ? <Badge tone="neutral">Archived</Badge> : null}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}

            {isAdmin ? (
              <div className="mt-3 grid gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
                <Label htmlFor="new-kit-name">New toolbox</Label>
                <Input
                  id="new-kit-name"
                  data-testid="toolbox-new-name"
                  value={newKitName}
                  placeholder="Network field kit"
                  onChange={(e) => setNewKitName(e.target.value)}
                />
                <Input
                  id="new-kit-code"
                  data-testid="toolbox-new-code"
                  value={newKitCode}
                  placeholder="Short code (KIT-NET-01)"
                  onChange={(e) => setNewKitCode(e.target.value)}
                />
                <Button size="sm" disabled={busy} onClick={() => void createKit()} data-testid="toolbox-create">
                  Create toolbox
                </Button>
              </div>
            ) : null}
          </Card>

          <Card className="content-start p-3">
            {selected === null ? (
              <EmptyState message="Pick a toolbox to edit its membership and print its tag." />
            ) : (
              <div className="grid gap-3">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="grid gap-0.5">
                    <span className="text-sm font-semibold">{selected.name}</span>
                    <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {detail?.members.length ?? 0} member tool
                      {(detail?.members.length ?? 0) === 1 ? "" : "s"}
                      {availability !== null ? (availability.ready ? " · ready to check out" : " · not ready at the moment") : ""}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <LabelPrintButton
                      purpose="tool_kit"
                      entityId={selected.id}
                      entityLabel={selected.name}
                    />
                    {isAdmin ? (
                      <>
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={busy}
                          data-testid="toolbox-toggle-active"
                          onClick={() => void patchKit({ active: !selected.active })}
                        >
                          {selected.active ? "Archive" : "Restore"}
                        </Button>
                      </>
                    ) : null}
                  </div>
                </div>

                {detail === null ? (
                  <div className="flex justify-center p-4">
                    <Spinner />
                  </div>
                ) : detail.members.length === 0 ? (
                  <EmptyState message="This toolbox has no member tools yet." />
                ) : (
                  <ul className="grid gap-0.5" role="list">
                    {detail.members.map((member) => {
                      const state = readiness.get(member.stock_item_id);
                      return (
                        <li
                          key={member.id}
                          data-testid="toolbox-member"
                          className="row-hover flex items-center justify-between gap-2 py-1.5 text-xs"
                        >
                          <span className="flex min-w-0 items-baseline gap-1.5">
                            <span className="truncate">{member.item_name}</span>
                            {member.item_sku !== null && member.item_sku !== undefined ? (
                              <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                                {member.item_sku}
                              </span>
                            ) : null}
                          </span>
                          <span className="flex shrink-0 items-center gap-2">
 <span className="text-[10px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                              {qtyText(member.qty_milli)}×
                            </span>
                            {state !== undefined ? (
                              <Badge tone={state.ready ? "success" : "warning"}>
                                {state.ready
                                  ? "In stock"
                                  : `${qtyText(state.available)} of ${qtyText(state.required)}`}
                              </Badge>
                            ) : null}
                            {isAdmin ? (
                              <button
                                type="button"
                                data-testid="toolbox-member-remove"
                                aria-label={`Remove ${member.item_name}`}
                                className="cursor-pointer text-[11px]"
                                style={{ color: "var(--m-text-muted)" }}
                                onClick={() => void removeMember(member.id)}
                              >
                                ×
                              </button>
                            ) : null}
                          </span>
                        </li>
                      );
                    })}
                  </ul>
                )}

                {isAdmin ? (
                  <div className="grid gap-2 border-t pt-3 sm:grid-cols-[minmax(0,1fr)_100px_auto] sm:items-end" style={{ borderColor: "var(--m-border)" }}>
                    <div className="grid gap-1">
                      <Label htmlFor="member-item">Add a tool</Label>
                      <Select value={memberItemId} onValueChange={setMemberItemId}>
                        <SelectTrigger id="member-item" data-testid="toolbox-member-item">
                          <SelectValue placeholder="Pick a tool" />
                        </SelectTrigger>
                        <SelectContent>
                          {toolItems.map((item) => (
                            <SelectItem key={item.id} value={item.id}>
                              {item.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="grid gap-1">
                      <Label htmlFor="member-qty">Qty</Label>
                      <Input
                        id="member-qty"
                        type="number"
                        min={1}
                        value={memberQty}
                        onChange={(e) => setMemberQty(e.target.value)}
                      />
                    </div>
                    <Button size="sm" disabled={busy} data-testid="toolbox-member-add" onClick={() => void addMember()}>
                      Add member
                    </Button>
                  </div>
                ) : null}
              </div>
            )}
          </Card>
        </div>

        <Card className="p-3">
          <h2 className="mb-1 text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
            Dispatch defaults
          </h2>
          <p className="mb-3 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Every scheduled ticket whose type, category, or board matches a rule shows the mapped
            items — or the mapped default kit — on the day pack. A pin on the ticket itself always
            wins over a rule.
          </p>

          {rules === null ? (
            <div className="flex justify-center p-4">
              <Spinner />
            </div>
          ) : rules.length === 0 ? (
            <EmptyState message="No dispatch defaults yet. Map a category to a tool or a kit below." />
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-xs" data-testid="dispatch-rules">
                <thead>
                  <tr style={{ color: "var(--m-text-muted)" }}>
                    <th className="py-1 text-left font-medium">Scope</th>
                    <th className="py-1 text-left font-medium">Recommends</th>
                    <th className="py-1 text-right font-medium">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {rules.map((rule) => (
                    <tr key={rule.id} data-testid="dispatch-rule-row" style={{ borderTop: "1px solid var(--m-border)" }}>
                      <td className="py-1.5">
                        <span className="text-[10px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                          {SCOPE_KIND_LABELS[rule.scope_kind] ?? rule.scope_kind}
                        </span>
                        <span className="ml-2">{rule.scope_name}</span>
                      </td>
                      <td className="py-1.5">
                        {targetLabel(rule)}
                        {!rule.active ? (
                          <span className="ml-2 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                            (off)
                          </span>
                        ) : null}
                      </td>
                      <td className="py-1.5 text-right">
                        {isAdmin ? (
                          <span className="inline-flex items-center gap-1.5">
                            <Button
                              variant="ghost"
                              size="sm"
                              className="!px-1.5"
                              data-testid="dispatch-rule-toggle"
                              onClick={() => void toggleRule(rule)}
                            >
                              {rule.active ? "Turn off" : "Turn on"}
                            </Button>
                            <button
                              type="button"
                              data-testid="dispatch-rule-remove"
                              aria-label="Remove rule"
                              className="cursor-pointer text-[11px]"
                              style={{ color: "var(--m-text-muted)" }}
                              onClick={() => void removeRule(rule)}
                            >
                              ×
                            </button>
                          </span>
                        ) : null}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          {isAdmin ? (
            <div className="mt-3 grid gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
              <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
                <div className="grid gap-1">
                  <Label htmlFor="rule-scope-kind">When</Label>
                  <Select
                    value={ruleScopeKind}
                    onValueChange={(value) => {
                      setRuleScopeKind(value);
                      setRuleScopeId("");
                    }}
                  >
                    <SelectTrigger id="rule-scope-kind" data-testid="rule-scope-kind">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="ticket_category">Ticket category is</SelectItem>
                      <SelectItem value="ticket_type">Ticket type is</SelectItem>
                      <SelectItem value="board">Board is</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="rule-scope">Scope</Label>
                  <Select value={ruleScopeId} onValueChange={setRuleScopeId}>
                    <SelectTrigger id="rule-scope" data-testid="rule-scope">
                      <SelectValue placeholder="Pick a scope" />
                    </SelectTrigger>
                    <SelectContent>
                      {scopeOptions.map((option) => (
                        <SelectItem key={option.id} value={option.id}>
                          {option.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="rule-target-kind">Then</Label>
                  <Select value={ruleTargetKind} onValueChange={setRuleTargetKind}>
                    <SelectTrigger id="rule-target-kind" data-testid="rule-target-kind">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="item">Recommend items</SelectItem>
                      <SelectItem value="kit">Dispatch a default kit</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1">
                  {ruleTargetKind === "kit" ? (
                    <>
                      <Label htmlFor="rule-kit">Kit</Label>
                      <Select value={ruleKitId} onValueChange={setRuleKitId}>
                        <SelectTrigger id="rule-kit" data-testid="rule-kit">
                          <SelectValue placeholder="Pick a kit" />
                        </SelectTrigger>
                        <SelectContent>
                          {(kits ?? []).map((kit) => (
                            <SelectItem key={kit.id} value={kit.id}>
                              {kit.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </>
                  ) : (
                    <>
                      <Label htmlFor="rule-item">Item</Label>
                      <Select value={ruleItemId} onValueChange={setRuleItemId}>
                        <SelectTrigger id="rule-item" data-testid="rule-item">
                          <SelectValue placeholder="Pick an item" />
                        </SelectTrigger>
                        <SelectContent>
                          {activeItems.map((item) => (
                            <SelectItem key={item.id} value={item.id}>
                              {item.name}
                              {item.is_tool ? " · tool" : ""}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </>
                  )}
                </div>
              </div>
              <div className="grid gap-2 sm:grid-cols-[100px_minmax(0,1fr)_auto] sm:items-end">
                <div className="grid gap-1">
                  <Label htmlFor="rule-qty">Qty</Label>
                  <Input
                    id="rule-qty"
                    type="number"
                    min={1}
                    value={ruleQty}
                    onChange={(e) => setRuleQty(e.target.value)}
                  />
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="rule-note">Note</Label>
                  <Input
                    id="rule-note"
                    value={ruleNote}
                    placeholder="Shown on the day pack line"
                    onChange={(e) => setRuleNote(e.target.value)}
                  />
                </div>
                <Button size="sm" disabled={busy} data-testid="rule-create" onClick={() => void addRule()}>
                  Add rule
                </Button>
              </div>
              {scopeOptions.length === 0 ? (
                <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  No {SCOPE_KIND_LABELS[ruleScopeKind]?.toLowerCase() ?? "scope"} rows exist yet.
                </p>
              ) : null}
            </div>
          ) : null}
        </Card>
      </div>
    </div>
  );
}
