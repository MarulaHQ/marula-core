// /stock/tools — the tool checkout board (Wave 32 lane 4).
//
// Purpose: who has which tool right now, what is available per location, and
// the two writes that move a tool: check out (a kit or an item list) and
// check in (per-line condition). Kits are composed here; their readiness is
// read before a checkout is sent.
//
// Non-purpose: no client loans (loans register), no stock quantities or
// purchases (the stock board and PO flows), no serialized device edits.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
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
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { LedgerSurface, type LedgerColumn } from "../../../components/assets-stock/ledger-surface";
import { LedgerRow } from "../../../components/assets-stock/ledger-row";
import { csvFileName, downloadCsv } from "../../../components/assets-stock/csv";
import {
  DueSoonMark,
  NeutralMark,
  OverdueMark,
  ReturnedMark,
} from "../../../components/assets-stock/ledger-marks";
import { CheckInDialog } from "../../../components/stock/check-in-dialog";
import { CheckoutDialog, type HolderOption } from "../../../components/stock/checkout-dialog";
import { ScanBar, type ScanEvent } from "../../../components/scan/scan-bar";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import {
  duePhrase,
  formatQtyMilli,
  parseQtyToMilli,
  serialStatusLabel,
} from "../../../lib/stock-format";
import type { FilterClause } from "@marula-core/ui";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type ToolSerialWhereaboutsView = components["schemas"]["ToolSerialWhereaboutsView"];
type ToolAvailabilityView = components["schemas"]["ToolAvailabilityView"];
type ToolKitView = components["schemas"]["ToolKitView"];
type KitDetailResponse = components["schemas"]["KitDetailResponse"];
type KitAvailabilityResponse = components["schemas"]["KitAvailabilityResponse"];

const TOOL_VIEWS = [
  { id: "tools", label: "Tools" },
  { id: "ledger", label: "Ledger" },
  { id: "loans", label: "Loans" },
];

const TOOL_COLUMNS: LedgerColumn[] = [
  { key: "serial", label: "Serial", required: true },
  { key: "tool", label: "Tool" },
  { key: "status", label: "Status" },
  { key: "holder", label: "Holder" },
  { key: "location", label: "Location" },
  { key: "due", label: "Due" },
  { key: "actions", label: "Actions", align: "right" },
];

const TOOL_FILTERS = [
  { key: "tool", label: "Tool" },
  { key: "serial", label: "Serial" },
  { key: "status", label: "Status" },
  { key: "holder", label: "Holder" },
  { key: "location", label: "Location" },
];

/** Due inside this window is "due soon" (blush); past it is overdue (red). */
const TOOL_DUE_SOON_DAYS = 3;

function toolDaysUntil(dueAt: string, now: Date): number {
  const at = new Date(dueAt.length === 10 ? `${dueAt}T23:59:59` : dueAt);
  if (Number.isNaN(at.getTime())) return Number.POSITIVE_INFINITY;
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startOfDue = new Date(at.getFullYear(), at.getMonth(), at.getDate());
  return Math.round((startOfDue.getTime() - startOfToday.getTime()) / 86_400_000);
}

/**
 * A tool unit's state carries the same severity discipline as the loans view:
 * overdue is RED because a tool past its return is a genuine breach of the
 * agreement, due soon is BLUSH, and every state says its name with an icon so
 * colour is never the only signal.
 */
function ToolDueState({ row }: { row: ToolSerialWhereaboutsView }): ReactNode {
  if (row.status === "in_stock") return <ReturnedMark label="In stock" />;
  if (row.overdue === true) return <OverdueMark label="Overdue" testId={`tools-overdue-${row.id}`} />;
  if (row.status === "checked_out" || row.status === "loaned") {
    return (
      <NeutralMark
        label={serialStatusLabel(row.status)}
        testId={`tools-status-${row.id}`}
      />
    );
  }
  return <NeutralMark label={serialStatusLabel(row.status)} />;
}

function ToolDuePhrase({ due }: { due: string }): ReactNode {
  const now = new Date();
  const days = toolDaysUntil(due, now);
  if (days < 0) return <OverdueMark label={`Overdue ${Math.abs(days)}d`} />;
  if (days <= TOOL_DUE_SOON_DAYS) {
    return <DueSoonMark label={days === 0 ? "Due today" : duePhrase(due, now)} />;
  }
  return <span style={{ color: "var(--m-text-secondary)" }}>{duePhrase(due, now)}</span>;
}

function idPrefix(id: string): string {
  return `${id.slice(0, 8)}…`;
}

export default function StockToolsPage(): ReactNode {
  const auth = useRequireAuth();
  const me = auth.me;

  const [items, setItems] = useState<StockItemView[] | null>(null);
  const [locations, setLocations] = useState<StockLocationView[]>([]);
  const [whereabouts, setWhereabouts] = useState<ToolSerialWhereaboutsView[]>([]);
  const [availability, setAvailability] = useState<ToolAvailabilityView[]>([]);
  const [kits, setKits] = useState<ToolKitView[]>([]);
  const [holders, setHolders] = useState<HolderOption[]>([]);
  const [directoryDenied, setDirectoryDenied] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [checkoutOpen, setCheckoutOpen] = useState(false);
  const [checkinId, setCheckinId] = useState<string | null>(null);
  const [checkoutSeed, setCheckoutSeed] = useState<{
    itemId: string;
    serialId?: string;
    serialLabel?: string;
  } | null>(null);
  const [scanNotice, setScanNotice] = useState<string | null>(null);

  const [kitCreateOpen, setKitCreateOpen] = useState(false);
  const [kitForm, setKitForm] = useState({ name: "", code: "", note: "" });
  const [kitFormError, setKitFormError] = useState<string | null>(null);
  const [kitFormBusy, setKitFormBusy] = useState(false);

  const [manageKit, setManageKit] = useState<ToolKitView | null>(null);
  const [kitDetail, setKitDetail] = useState<KitDetailResponse | null>(null);
  const [kitAvailability, setKitAvailability] = useState<KitAvailabilityResponse | null>(null);
  const [memberItemId, setMemberItemId] = useState("");
  const [memberQty, setMemberQty] = useState("1");
  const [memberError, setMemberError] = useState<string | null>(null);
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [overdueOnly, setOverdueOnly] = useState(false);
  const [availableOnly, setAvailableOnly] = useState(false);
  const router = useRouter();
  const [memberBusy, setMemberBusy] = useState(false);

  async function refresh(): Promise<void> {
    const [itemsRes, locationsRes, whereaboutsRes, availabilityRes, kitsRes] = await Promise.all([
      api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/stock-serials", { params: { query: { tool_only: true, limit: 500 } } }),
      api.api.GET("/api/v1/tools/availability", { params: { query: { limit: 500 } } }),
      api.api.GET("/api/v1/tool-kits", { params: { query: { limit: 200 } } }),
    ]);
    if (itemsRes.data === undefined) {
      setLoadError("Could not load tools — try reloading.");
      return;
    }
    setItems(itemsRes.data ?? []);
    setLocations(locationsRes.data ?? []);
    setWhereabouts(whereaboutsRes.data ?? []);
    setAvailability(availabilityRes.data ?? []);
    setKits(kitsRes.data ?? []);
    const usersRes = await api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } });
    if (usersRes.data !== undefined) {
      setHolders(usersRes.data.users.map((user) => ({ id: user.id, label: user.display_name })));
      setDirectoryDenied(false);
    } else {
      // Manager+ directory read refused: a holder must still be pickable,
      // and the caller is always a valid holder of their own checkout.
      setDirectoryDenied(true);
      setHolders(me === null ? [] : [{ id: me.user_id, label: me.display_name }]);
    }
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, me?.user_id]);

  /**
   * Scan-to-checkout / scan-to-check-in: an in-stock unit seeds the
   * checkout row; a unit that is already out finds its open checkout and
   * opens the check-in. A non-tool scan is refused with a sentence.
   */
  async function toolsScan(event: ScanEvent): Promise<void> {
    const resolution = event.resolution;
    setScanNotice(null);
    if (!resolution.is_tool) {
      setScanNotice(`“${resolution.display_name}” is not a tool — nothing to check here.`);
      return;
    }
    if (resolution.serial_status === "checked_out" || resolution.serial_status === "loaned") {
      const { data } = await api.api.GET("/api/v1/tool-checkouts", {
        params: { query: { status: "open", limit: 200 } },
      });
      const open = (data ?? []).find((row) =>
        row.lines.some((line) => line.serial_id === resolution.serial_id),
      );
      if (open === undefined) {
        setScanNotice(
          `“${resolution.secondary ?? resolution.display_name}” reads out with no open checkout — check the ledger.`,
        );
        return;
      }
      setCheckinId(open.checkout.id);
      return;
    }
    if (resolution.serial_id == null) {
      setScanNotice("That tool code did not name a unit — scan the unit's label.");
      return;
    }
    setCheckoutSeed({
      itemId: resolution.stock_item_id ?? resolution.entity_id ?? "",
      serialId: resolution.serial_id,
      serialLabel: resolution.secondary ?? "",
    });
    setCheckoutOpen(true);
  }

  const holderLabel = useMemo(() => {
    const map = new Map(holders.map((holder) => [holder.id, holder.label]));
    return (id: string | null | undefined): string => (id == null ? "—" : (map.get(id) ?? idPrefix(id)));
  }, [holders]);

  const serialLabels = useMemo(() => {
    const map: Record<string, string> = {};
    for (const row of whereabouts) map[row.id] = row.serial_number;
    return map;
  }, [whereabouts]);

  const itemLabels = useMemo(() => {
    const map: Record<string, string> = {};
    for (const item of items ?? []) map[item.id] = item.name;
    return map;
  }, [items]);

  const totals = useMemo(() => {
    return availability.reduce(
      (sum, row) => ({
        available: sum.available + row.available_units,
        checkedOut: sum.checkedOut + row.checked_out_units,
        loaned: sum.loaned + row.loaned_units,
        out: sum.out + row.out_units,
      }),
      { available: 0, checkedOut: 0, loaned: 0, out: 0 },
    );
  }, [availability]);

  const toolItems = useMemo(
    () => (items ?? []).filter((item) => item.is_tool && item.active),
    [items],
  );

  async function loadKitDetail(kitId: string): Promise<void> {
    const [detailRes, availabilityRes] = await Promise.all([
      api.api.GET("/api/v1/tool-kits/{id}", { params: { path: { id: kitId } } }),
      api.api.GET("/api/v1/tool-kits/{id}/availability", { params: { path: { id: kitId }, query: {} } }),
    ]);
    setKitDetail(detailRes.data ?? null);
    setKitAvailability(availabilityRes.data ?? null);
  }

  function openManage(kit: ToolKitView): void {
    setManageKit(kit);
    setKitDetail(null);
    setKitAvailability(null);
    setMemberItemId("");
    setMemberQty("1");
    setMemberError(null);
    void loadKitDetail(kit.id);
  }

  async function createKit(): Promise<void> {
    setKitFormError(null);
    const name = kitForm.name.trim();
    if (name === "") {
      setKitFormError("Name is required.");
      return;
    }
    setKitFormBusy(true);
    const tempId = `pending-${Date.now()}`;
    const result = await runEnvelopeMutation({
      describe: "Create tool kit",
      buildBody: (base) => ({
        name,
        code: kitForm.code.trim() === "" ? null : kitForm.code.trim(),
        note: kitForm.note.trim() === "" ? null : kitForm.note,
        ...base,
      }),
      optimistic: () =>
        setKits((cur) => [
          ...cur,
          {
            id: tempId,
            org_id: "",
            name,
            code: kitForm.code.trim() === "" ? null : kitForm.code.trim(),
            note: kitForm.note.trim() === "" ? null : kitForm.note,
            active: true,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            created_by: null,
          },
        ]),
      revert: () => setKits((cur) => cur.filter((kit) => kit.id !== tempId)),
      request: (client, body) =>
        client.POST("/api/v1/tool-kits", { body: body as components["schemas"]["CreateKitRequest"] }),
      versions: (data) => [[data.id, data.mutation.version]],
    });
    setKitFormBusy(false);
    if (result.ok === false) {
      setKitFormError(rejectionText(result, "Could not create the kit."));
      return;
    }
    setKitCreateOpen(false);
    setKitForm({ name: "", code: "", note: "" });
    setNotice(`${name} created. Add member tools before checking it out.`);
    await refresh();
  }

  async function addMember(): Promise<void> {
    const kit = manageKit;
    if (kit === null) return;
    setMemberError(null);
    if (memberItemId === "") {
      setMemberError("Pick a tool to add.");
      return;
    }
    const qtyMilli = parseQtyToMilli(memberQty) ?? 1000;
    if (qtyMilli <= 0) {
      setMemberError("Quantity must be greater than zero.");
      return;
    }
    setMemberBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Add kit member",
      entityId: kit.id,
      buildBody: (base) => ({ stock_item_id: memberItemId, qty_milli: qtyMilli, ...base }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/tool-kits/{id}/members", {
          params: { path: { id: kit.id } },
          body: body as components["schemas"]["AddKitMemberRequest"],
        }),
    });
    setMemberBusy(false);
    if (result.ok === false) {
      setMemberError(rejectionText(result, "Could not add the member."));
      return;
    }
    setMemberItemId("");
    setMemberQty("1");
    await loadKitDetail(kit.id);
  }

  async function removeMember(kitId: string, memberId: string): Promise<void> {
    setMemberError(null);
    setMemberBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Remove kit member",
      entityId: kitId,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {},
      revert: () => {},
      request: (client) =>
        client.DELETE("/api/v1/tool-kits/{id}/members/{member_id}", {
          params: { path: { id: kitId, member_id: memberId } },
        }),
    });
    setMemberBusy(false);
    if (result.ok === false) {
      setMemberError(rejectionText(result, "Could not remove the member."));
      return;
    }
    await loadKitDetail(kitId);
  }

  if (items === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const overdueCount = whereabouts.filter((row) => row.overdue === true).length;
  const rows = whereabouts.filter((row) => {
    if (overdueOnly && row.overdue !== true) return false;
    if (availableOnly && row.status !== "in_stock") return false;
    for (const clause of clauses) {
      const value = (
        {
          tool: row.item_name,
          serial: row.serial_number,
          status: serialStatusLabel(row.status),
          holder: row.holder_user_id == null ? "" : holderLabel(row.holder_user_id),
          location: row.location_name ?? "",
        }[clause.field] ?? ""
      ).toLowerCase();
      const want = (clause.value ?? "").toLowerCase();
      if (clause.op === "is_set" && value === "") return false;
      if (clause.op === "is_not_set" && value !== "") return false;
      if (clause.op === "contains" && !value.includes(want)) return false;
      if (clause.op === "eq" && value !== want) return false;
      if (clause.op === "ne" && value === want) return false;
    }
    return true;
  });

  return (
    <>
    <LedgerSurface
      surface="stock-tools"
      testId="tools-ledger"
      title="Tool checkout"
      subtitle="Who holds each tool unit now, and what sits in stock per location."
      shortcuts={[
        {
          key: "overdue",
          label: "Overdue",
          count: overdueCount,
          tone: "red",
          active: overdueOnly,
          onClick: () => setOverdueOnly((v) => !v),
        },
        {
          key: "available",
          label: "Available",
          count: totals.available,
          tone: "gold",
          active: availableOnly,
          onClick: () => setAvailableOnly((v) => !v),
        },
      ]}
      headerAction={
        <>
          <Button variant="secondary" onClick={() => setKitCreateOpen(true)} data-testid="tools-kit-new">
            New kit
          </Button>
          <Button onClick={() => setCheckoutOpen(true)} data-testid="tools-checkout-open">
            Check out tools
          </Button>
        </>
      }
      views={TOOL_VIEWS}
      activeView="tools"
      onViewChange={(id) => {
        if (id === "ledger") router.push("/stock");
        if (id === "loans") router.push("/stock/loans");
      }}
      columns={TOOL_COLUMNS}
      filterFields={TOOL_FILTERS}
      filterClauses={clauses}
      onFilterChange={setClauses}
      rowCount={rows.length}
      rowsShown={rows.length}
      onExport={(request) => {
        const stamp = new Date().toISOString().slice(0, 10);
        downloadCsv(
          csvFileName("tools", stamp),
          request.columns,
          rows.map((row) => ({
            serial: row.serial_number,
            tool: row.item_name,
            status: serialStatusLabel(row.status),
            holder: holderLabel(row.holder_user_id),
            location: row.location_name ?? "",
            due: duePhrase(row.due_at ?? row.expected_return_at ?? ""),
          })),
        );
      }}
      selectedIds={selected}
      onToggleRow={(id, isSelected) =>
        setSelected((cur) => (isSelected ? [...cur, id] : cur.filter((row) => row !== id)))
      }
      onToggleAll={(isSelected) => setSelected(isSelected ? rows.map((r) => r.id) : [])}
      allSelected={rows.length > 0 && selected.length === rows.length}
      someSelected={selected.length > 0 && selected.length < rows.length}
      bulkActions={[
        {
          key: "check-in",
          label: "Check in",
          disabled: selected.length !== 1,
          title: "Check the selected unit back in",
          onClick: () => setCheckinId(selected[0] ?? null),
        },
      ]}
      onClearSelection={() => setSelected([])}
      emptyMessage="No tool units yet. Flag a serialized stock item as a tool to see it here."
      errorMessage={actionError}
      childrenAfter={
        <>
          {directoryDenied && (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }} data-testid="tools-directory-note">
              The staff directory needs a manager role. The holder picker lists you only.
            </p>
          )}
          {notice !== null && (
            <p className="text-xs" role="status" style={{ color: "var(--m-status-success-fg)" }} data-testid="tools-notice">
              {notice}
            </p>
          )}
          {scanNotice !== null && (
            <p className="text-xs" role="status" style={{ color: "var(--m-status-warning-fg)" }} data-testid="tools-scan-notice">
              {scanNotice}
            </p>
          )}
          <div className="max-w-xl" data-testid="tools-scan">
            <ScanBar
              surface="tools"
              onScan={(event) => void toolsScan(event)}
              hint="Scan a tool unit to check it out, or to check it in when it is already out."
            />
          </div>
          <Card className="m-rise overflow-x-auto p-0" data-testid="tools-kits">
            <div className="flex flex-wrap items-center justify-between gap-2 p-4 pb-2">
              <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
                Kits
              </h2>
              <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                a kit checks out every member unit at once
              </span>
            </div>
            <Table data-testid="tools-kits-table">
              <thead>
                <tr>
                  <Th>Name</Th>
                  <Th>Code</Th>
                  <Th>Note</Th>
                  <Th>State</Th>
                  <Th className="text-right">Actions</Th>
                </tr>
              </thead>
              <tbody>
                {kits.map((kit) => (
                  <Tr key={kit.id} className="row-hover" data-testid={`tools-kit-row-${kit.id}`}>
                    <Td>
                      <span className="font-medium" style={{ color: "var(--m-text)" }}>
                        {kit.name}
                      </span>
                    </Td>
                    <Td className="text-xs">{kit.code ?? "—"}</Td>
                    <Td className="text-xs">{kit.note ?? "—"}</Td>
                    <Td>
                      <StatusTag tone={kit.active ? "success" : "neutral"}>
                        {kit.active ? "active" : "archived"}
                      </StatusTag>
                    </Td>
                    <Td>
                      <div className="flex justify-end">
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={() => openManage(kit)}
                          data-testid={`tools-kit-manage-${kit.id}`}
                        >
                          Members
                        </Button>
                      </div>
                    </Td>
                  </Tr>
                ))}
                {kits.length === 0 && (
                  <Tr>
                    <Td>
                      <EmptyState message="No kits yet. Create one and add member tools." />
                    </Td>
                  </Tr>
                )}
              </tbody>
            </Table>
          </Card>
        </>
      }
    >
      {(columns) =>
        rows.map((row) => {
          const due = row.due_at ?? row.expected_return_at ?? null;
          return (
            <LedgerRow
              key={row.id}
              id={row.id}
              selected={selected.includes(row.id)}
              selectable
              onToggle={(rowId, isSelected) =>
                setSelected((cur) => (isSelected ? [...cur, rowId] : cur.filter((id) => id !== rowId)))
              }
              columns={columns}
              selectLabel={`Select tool unit ${row.serial_number}`}
              testId={`tools-row-${row.id}`}
              cells={{
                serial: <span className="text-xs">{row.serial_number}</span>,
                tool: (
                  <span style={{ color: "var(--m-text)" }}>{row.item_name}</span>
                ),
                status: <ToolDueState row={row} />,
                holder: (
                  <span>{row.holder_user_id == null ? "—" : holderLabel(row.holder_user_id)}</span>
                ),
                location: <span>{row.location_name ?? "—"}</span>,
                due: due == null ? <span style={{ color: "var(--m-text-muted)" }}>—</span> : <ToolDuePhrase due={due} />,
                actions:
                  row.checkout_id != null ? (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setCheckinId(row.checkout_id ?? null)}
                      data-testid={`tools-checkin-${row.id}`}
                    >
                      Check in
                    </Button>
                  ) : (
                    <span style={{ color: "var(--m-text-muted)" }}>—</span>
                  ),
              }}
            />
          );
        })
      }
    </LedgerSurface>

      <CheckoutDialog
        open={checkoutOpen}
        onOpenChange={(open) => {
          setCheckoutOpen(open);
          if (!open) setCheckoutSeed(null);
        }}
        items={items ?? []}
        locations={locations}
        kits={kits}
        holders={holders}
        defaultHolderId={me?.user_id ?? ""}
        initialItemId={checkoutSeed?.itemId}
        initialSerialId={checkoutSeed?.serialId}
        initialSerialLabel={checkoutSeed?.serialLabel}
        onCheckedOut={() => {
          setNotice("Tools checked out.");
          setCheckoutSeed(null);
          void refresh();
        }}
      />

      <CheckInDialog
        open={checkinId !== null}
        onOpenChange={(open) => {
          if (!open) setCheckinId(null);
        }}
        checkoutId={checkinId}
        locations={locations}
        serialLabels={serialLabels}
        itemLabels={itemLabels}
        onCheckedIn={() => {
          setNotice("Tools checked in.");
          void refresh();
        }}
      />

      <Dialog open={kitCreateOpen} onOpenChange={setKitCreateOpen}>
        <DialogContent title="New kit" data-testid="tools-kit-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            <div className="grid gap-1">
              <Label htmlFor="kit-name">Name</Label>
              <Input
                id="kit-name"
                value={kitForm.name}
                onChange={(e) => setKitForm({ ...kitForm, name: e.target.value })}
                data-testid="tools-kit-name"
              />
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="kit-code">Code</Label>
                <Input
                  id="kit-code"
                  value={kitForm.code}
                  onChange={(e) => setKitForm({ ...kitForm, code: e.target.value })}
                  data-testid="tools-kit-code"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="kit-note">Note</Label>
                <Input
                  id="kit-note"
                  value={kitForm.note}
                  onChange={(e) => setKitForm({ ...kitForm, note: e.target.value })}
                  data-testid="tools-kit-note"
                />
              </div>
            </div>
            {kitFormError !== null && <FieldError data-testid="tools-kit-error">{kitFormError}</FieldError>}
            <DialogActions>
              <Button variant="secondary" onClick={() => setKitCreateOpen(false)}>
                Cancel
              </Button>
              <Button disabled={kitFormBusy} onClick={() => void createKit()} data-testid="tools-kit-save">
                {kitFormBusy ? "Creating…" : "Create kit"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog
        open={manageKit !== null}
        onOpenChange={(open) => {
          if (!open) setManageKit(null);
        }}
      >
        <DialogContent title={manageKit === null ? "Kit members" : `${manageKit.name} members`} data-testid="tools-kit-members-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            {kitAvailability !== null && (
              <Card className="grid gap-1 p-3" data-testid="tools-kit-availability">
                <div className="flex flex-wrap items-center gap-2">
                  <StatusTag tone={kitAvailability.ready ? "success" : "danger"}>
                    {kitAvailability.ready ? "ready" : "not ready"}
                  </StatusTag>
                  <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                    {kitAvailability.ready
                      ? "Every member has enough in-stock units."
                      : "Members are short somewhere in stock."}
                  </span>
                </div>
                {kitAvailability.members.map((member) => (
                  <p key={member.stock_item_id} className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                    {member.item_name}: {member.available_units} of {member.required_units}
                  </p>
                ))}
              </Card>
            )}

            <div className="grid gap-2">
              {(kitDetail?.members ?? []).map((member) => (
                <div
                  key={member.id}
                  className="flex flex-wrap items-center justify-between gap-2 border p-2"
                  style={{ borderColor: "var(--m-border)" }}
                  data-testid={`tools-kit-member-row-${member.id}`}
                >
                  <div>
                    <p className="text-sm" style={{ color: "var(--m-text)" }}>
                      {member.item_name}
                    </p>
                    <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                      {formatQtyMilli(member.qty_milli, "unit")}
                      {member.item_sku != null && member.item_sku !== "" ? ` · ${member.item_sku}` : ""}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={memberBusy || manageKit === null}
                    onClick={() => manageKit !== null && void removeMember(manageKit.id, member.id)}
                    data-testid={`tools-kit-member-remove-${member.id}`}
                  >
                    Remove
                  </Button>
                </div>
              ))}
              {kitDetail !== null && kitDetail.members.length === 0 && (
                <EmptyState message="This kit has no members yet. Add a tool below." />
              )}
            </div>

            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <div className="grid gap-1 sm:col-span-2">
                <Label htmlFor="kit-member-item">Add tool</Label>
                <Select value={memberItemId} onValueChange={setMemberItemId}>
                  <SelectTrigger id="kit-member-item" data-testid="tools-kit-member-item">
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
                <Label htmlFor="kit-member-qty">Units</Label>
                <Input
                  id="kit-member-qty"
                  inputMode="decimal"
                  value={memberQty}
                  onChange={(e) => setMemberQty(e.target.value)}
                  data-testid="tools-kit-member-qty"
                />
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="secondary"
                disabled={memberBusy || manageKit === null}
                onClick={() => void addMember()}
                data-testid="tools-kit-member-add"
              >
                Add member
              </Button>
              <FieldHint>Only flagged tools can join a kit</FieldHint>
            </div>
            {memberError !== null && <FieldError data-testid="tools-kit-member-error">{memberError}</FieldError>}
            <DialogActions>
              <Button variant="secondary" onClick={() => setManageKit(null)}>
                Close
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
