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
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useState, type ReactNode } from "react";

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
  serialStatusTone,
} from "../../../lib/stock-format";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type ToolSerialWhereaboutsView = components["schemas"]["ToolSerialWhereaboutsView"];
type ToolAvailabilityView = components["schemas"]["ToolAvailabilityView"];
type ToolKitView = components["schemas"]["ToolKitView"];
type KitDetailResponse = components["schemas"]["KitDetailResponse"];
type KitAvailabilityResponse = components["schemas"]["KitAvailabilityResponse"];

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

  return (
    <div className="mx-auto grid w-full max-w-350 gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Tool checkout
          </h1>
          <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-secondary)" }}>
            Who holds each tool unit now, and what sits in stock per location.
          </p>
        </div>
        <div className="flex flex-wrap gap-1">
          <Button variant="secondary" onClick={() => setKitCreateOpen(true)} data-testid="tools-kit-new">
            New kit
          </Button>
          <Button onClick={() => setCheckoutOpen(true)} data-testid="tools-checkout-open">
            Check out tools
          </Button>
        </div>
      </header>

      <div className="max-w-xl" data-testid="tools-scan">
        <ScanBar
          surface="tools"
          onScan={(event) => void toolsScan(event)}
          hint="Scan a tool unit to check it out, or to check it in when it is already out."
        />
      </div>

      {directoryDenied && (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }} data-testid="tools-directory-note">
          The staff directory needs a manager role. The holder picker lists you only.
        </p>
      )}
      {actionError !== null && (
        <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="tools-error">
          {actionError}
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

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {(
          [
            { label: "Available", value: totals.available, hint: "in stock, checkout-eligible", testId: "tools-stat-available" },
            { label: "Checked out", value: totals.checkedOut, hint: "with a holder now", testId: "tools-stat-checked-out" },
            { label: "On loan", value: totals.loaned, hint: "with a client", testId: "tools-stat-loaned" },
            { label: "Out", value: totals.out, hint: "shipped or consumed", testId: "tools-stat-out" },
          ] as const
        ).map((tile, index) => (
          <Card key={tile.label} className="m-rise p-4" style={{ animationDelay: `${Math.min(index, 6) * 40}ms` }}>
            <p className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              {tile.label}
            </p>
            <p className="mt-1 font-mono text-xl font-semibold tabular-nums" style={{ color: "var(--m-text)" }} data-testid={tile.testId}>
              {tile.value}
            </p>
            <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
              {tile.hint}
            </p>
          </Card>
        ))}
      </div>

      <Card className="m-rise overflow-x-auto p-0">
        <div className="p-4 pb-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Who has what
          </h2>
        </div>
        <Table data-testid="tools-table">
          <thead>
            <tr>
              <Th>Serial</Th>
              <Th>Tool</Th>
              <Th>Status</Th>
              <Th>Holder</Th>
              <Th>Location</Th>
              <Th>Due</Th>
              <Th className="text-right">Actions</Th>
            </tr>
          </thead>
          <tbody>
            {whereabouts.map((row) => {
              const due = row.due_at ?? row.expected_return_at ?? null;
              return (
                <Tr key={row.id} className="row-hover" data-testid={`tools-row-${row.id}`}>
                  <Td className="font-mono text-xs">{row.serial_number}</Td>
                  <Td>
                    <span style={{ color: "var(--m-text)" }}>{row.item_name}</span>
                  </Td>
                  <Td>
                    <div className="flex flex-wrap items-center gap-1">
                      <StatusTag tone={serialStatusTone(row.status)}>{serialStatusLabel(row.status)}</StatusTag>
                      {row.overdue && <StatusTag tone="danger">overdue</StatusTag>}
                    </div>
                  </Td>
                  <Td className="text-xs">{row.holder_user_id == null ? "—" : holderLabel(row.holder_user_id)}</Td>
                  <Td className="text-xs">{row.location_name ?? "—"}</Td>
                  <Td className="text-xs">
                    {due === null ? (
                      <span style={{ color: "var(--m-text-muted)" }}>—</span>
                    ) : (
                      duePhrase(due)
                    )}
                  </Td>
                  <Td>
                    <div className="flex justify-end">
                      {row.checkout_id != null && (
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={() => setCheckinId(row.checkout_id ?? null)}
                          data-testid={`tools-checkin-${row.id}`}
                        >
                          Check in
                        </Button>
                      )}
                    </div>
                  </Td>
                </Tr>
              );
            })}
            {whereabouts.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No tool units yet. Flag a serialized stock item as a tool to see it here." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

      <Card className="m-rise overflow-x-auto p-0">
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
                <Td className="font-mono text-xs">{kit.code ?? "—"}</Td>
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
    </div>
  );
}
