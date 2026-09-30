// /purchasing/[id] — one purchase order end to end (Wave 32 lane 4).
//
// Purpose: the order's header and lines, the draft-only line edits, the
// receive dialog that posts into the stock ledger (location, per-line
// quantities, serials, carrier/tracking, explicit over-receipt
// acknowledgement), the receipt history, the vendor bill, and the
// one-click deeplink buttons built from the vendor's portal links.
//
// Non-purpose: no PO creation (the browser's New order dialog) and no vendor
// record editing (/purchasing/vendors).
"use client";

import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
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
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { OrderStatusTag, ReceiptStateTag } from "../../../components/purchasing/status";
import { MovementDialog, type MovementKind } from "../../../components/stock/movement-dialog";
import { ScanBar, type ScanEvent } from "../../../components/scan/scan-bar";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import { money } from "../../../lib/format";
import { formatQtyMilli, parseQtyToMilli } from "../../../lib/stock-format";

type PurchaseOrderDetail = components["schemas"]["PurchaseOrderDetail"];
type PurchaseOrderLineView = components["schemas"]["PurchaseOrderLineView"];
type PurchaseOrderStatus = components["schemas"]["PurchaseOrderStatus"];
type PurchaseReceiptView = components["schemas"]["PurchaseReceiptView"];
type PurchaseOrderDeeplinks = components["schemas"]["PurchaseOrderDeeplinks"];
type DeeplinkEntry = components["schemas"]["DeeplinkEntry"];
type StockLocationView = components["schemas"]["StockLocationView"];
type StockItemView = components["schemas"]["StockItemView"];
type ReceiveLineInput = components["schemas"]["ReceiveLineInput"];
type ExternalLinkKind = components["schemas"]["ExternalLinkKind"];

const LINK_KINDS: ExternalLinkKind[] = ["order_portal", "tracking", "invoice", "other"];

const LINK_KIND_LABELS: Record<ExternalLinkKind, string> = {
  order_portal: "Order portal",
  tracking: "Tracking",
  invoice: "Invoice",
  other: "Other",
};

interface ReceiveState {
  locationId: string;
  qtys: Record<string, string>;
  serials: Record<string, string>;
  carrier: string;
  trackingRef: string;
  note: string;
  allowOver: boolean;
}

interface LineForm {
  description: string;
  qty: string;
  unitCost: string;
}

interface LinkForm {
  url: string;
  label: string;
  kind: ExternalLinkKind;
  lineId: string;
  carrier: string;
}

function emptyLinkForm(): LinkForm {
  return { url: "", label: "", kind: "order_portal", lineId: "", carrier: "" };
}

/** Server message first: over-receipt refusals name the offending lines. */
function receiveFailureText(result: { message?: string; reason?: string }): string {
  const message = result.message;
  if (
    typeof message === "string" &&
    message.trim() !== "" &&
    !message.trim().startsWith("{") &&
    !message.includes("\n")
  ) {
    return message;
  }
  return rejectionText(result, "Could not record the receipt.");
}

export default function PurchaseOrderPage(): ReactNode {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const orderId = params.id;

  const [detail, setDetail] = useState<PurchaseOrderDetail | null>(null);
  const [locations, setLocations] = useState<StockLocationView[]>([]);
  const [stockItems, setStockItems] = useState<StockItemView[]>([]);
  const [deeplinks, setDeeplinks] = useState<PurchaseOrderDeeplinks | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const [receiveOpen, setReceiveOpen] = useState(false);
  const [receive, setReceive] = useState<ReceiveState>({
    locationId: "",
    qtys: {},
    serials: {},
    carrier: "",
    trackingRef: "",
    note: "",
    allowOver: false,
  });
  const [receiveBusy, setReceiveBusy] = useState(false);
  const [receiveError, setReceiveError] = useState<string | null>(null);
  const [receiveAllowVisible, setReceiveAllowVisible] = useState(false);
  const [receiveScanNotice, setReceiveScanNotice] = useState<string | null>(null);
  const [receiveScanCounts, setReceiveScanCounts] = useState<Record<string, number>>({});
  const [receivedLines, setReceivedLines] = useState<{ itemId: string; name: string }[]>([]);
  const [ticketOptions, setTicketOptions] = useState<{ id: string; label: string }[]>([]);
  const [dispatch, setDispatch] = useState<{
    open: boolean;
    itemId?: string;
    kind?: MovementKind;
  }>({
    open: false,
  });

  const [editingLine, setEditingLine] = useState<PurchaseOrderLineView | null>(null);
  const [lineForm, setLineForm] = useState<LineForm>({ description: "", qty: "", unitCost: "" });
  const [lineError, setLineError] = useState<string | null>(null);

  const [cancelOpen, setCancelOpen] = useState(false);
  const [cancelReason, setCancelReason] = useState("");

  const [linkFormOpen, setLinkFormOpen] = useState(false);
  const [linkForm, setLinkForm] = useState<LinkForm>(emptyLinkForm);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [linkBusy, setLinkBusy] = useState(false);

  const [billAmount, setBillAmount] = useState("");
  const [billNumber, setBillNumber] = useState("");
  const [billFormOpen, setBillFormOpen] = useState(false);
  const [billError, setBillError] = useState<string | null>(null);
  const [billBusy, setBillBusy] = useState(false);

  async function refresh(): Promise<void> {
    const [detailRes, locationsRes, deeplinksRes] = await Promise.all([
      api.api.GET("/api/v1/purchase-orders/{id}", { params: { path: { id: orderId } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/purchase-orders/{id}/deeplinks", { params: { path: { id: orderId } } }),
    ]);
    if (detailRes.data === undefined) {
      setLoadError("Could not load this purchase order — try reloading.");
      return;
    }
    setDetail(detailRes.data);
    setLocations(locationsRes.data ?? []);
    setDeeplinks(deeplinksRes.data ?? null);
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    void (async () => {
      const [itemsRes, ticketsRes] = await Promise.all([
        api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
        api.api.GET("/api/v1/tickets", { params: { query: { limit: 200 } } }),
      ]);
      setStockItems(itemsRes.data ?? []);
      setTicketOptions(
        ((ticketsRes.data ?? []) as components["schemas"]["TicketWithFields"][])
          .filter((ticket) => ticket.category !== "closed")
          .map((ticket) => ({ id: ticket.id, label: `${ticket.number} · ${ticket.title}` })),
      );
    })();
  }, [auth.status, orderId]);

  const stockItemById = useMemo(() => {
    const map = new Map<string, StockItemView>();
    for (const item of stockItems) map.set(item.id, item);
    return map;
  }, [stockItems]);

  const locationName = useMemo(() => {
    const map = new Map<string, string>();
    for (const location of locations) map.set(location.id, location.name);
    return map;
  }, [locations]);

  const lines = useMemo(
    () => [...(detail?.lines ?? [])].sort((a, b) => a.position - b.position),
    [detail],
  );

  const lineById = useMemo(() => {
    const map = new Map<string, PurchaseOrderLineView>();
    for (const line of lines) map.set(line.id, line);
    return map;
  }, [lines]);

  const canReadFinancials = (auth.me?.permissions ?? []).includes("financial:read");

  function isSerialized(line: PurchaseOrderLineView): boolean {
    const item = line.stock_item_id == null ? undefined : stockItemById.get(line.stock_item_id);
    return item?.kind === "serialized";
  }

  function lineUnit(line: PurchaseOrderLineView): string {
    const item = line.stock_item_id == null ? undefined : stockItemById.get(line.stock_item_id);
    return item?.unit ?? "";
  }

  async function runOrderAction(
    action: "send" | "close" | "cancel",
    reason?: string,
  ): Promise<void> {
    if (detail === null) return;
    const before = detail;
    const label = action === "send" ? "Send order" : action === "close" ? "Close order" : "Cancel order";
    const nextStatus: PurchaseOrderStatus =
      action === "send" ? "ordered" : action === "close" ? "closed" : "cancelled";
    setActionError(null);
    setNotice(null);
    setBusy(action);
    const box: { next: PurchaseOrderDetail | null } = { next: null };
    const result = await runEnvelopeMutation({
      describe: label,
      entityId: orderId,
      buildBody: (base) =>
        action === "cancel"
          ? { reason: reason !== undefined && reason.trim() !== "" ? reason.trim() : null, ...base }
          : { ...base },
      optimistic: () =>
        setDetail((cur) => (cur === null ? cur : { ...cur, order: { ...cur.order, status: nextStatus } })),
      revert: () => setDetail(before),
      request: async (client, body) => {
        const response =
          action === "send"
            ? await client.POST("/api/v1/purchase-orders/{id}/order", {
                params: { path: { id: orderId } },
                body: body as components["schemas"]["CloseOrderRequest"],
              })
            : action === "close"
              ? await client.POST("/api/v1/purchase-orders/{id}/close", {
                  params: { path: { id: orderId } },
                  body: body as components["schemas"]["CloseOrderRequest"],
                })
              : await client.POST("/api/v1/purchase-orders/{id}/cancel", {
                  params: { path: { id: orderId } },
                  body: body as components["schemas"]["CancelOrderRequest"],
                });
        if (response.data !== undefined) box.next = response.data;
        return response;
      },
      versions: (response) => [[response.order.id, response.mutation.version]],
    });
    setBusy(null);
    if (result.ok === false) {
      setActionError(`${label} failed: ${rejectionText(result, "the server refused.")}`);
      return;
    }
    if (box.next !== null) {
      setDetail(box.next);
      setDeeplinks((cur) => cur);
    } else {
      await refresh();
    }
    setNotice(
      action === "send"
        ? "Order sent to the vendor."
        : action === "close"
          ? "Order closed."
          : "Order cancelled.",
    );
  }

  function openReceive(): void {
    if (detail === null) return;
    const qtys: Record<string, string> = {};
    const serials: Record<string, string> = {};
    for (const line of detail.lines) {
      const remaining = line.qty_ordered_milli - line.qty_received_milli;
      qtys[line.id] = remaining > 0 ? formatQtyMilli(remaining, "") : "";
      serials[line.id] = "";
    }
    setReceive({
      locationId: locations[0]?.id ?? "",
      qtys,
      serials,
      carrier: "",
      trackingRef: "",
      note: "",
      allowOver: false,
    });
    setReceiveError(null);
    setReceiveScanNotice(null);
    setReceiveScanCounts({});
    setReceiveAllowVisible(false);
    setReceiveOpen(true);
  }

  async function submitReceive(): Promise<void> {
    if (detail === null) return;
    setReceiveError(null);
    if (receive.locationId === "") {
      setReceiveError("Choose the location the goods landed at.");
      return;
    }
    const receiveLines: ReceiveLineInput[] = [];
    const received: { itemId: string; name: string }[] = [];
    for (const line of detail.lines) {
      const raw = receive.qtys[line.id] ?? "";
      if (raw.trim() === "") continue;
      const qty = parseQtyToMilli(raw);
      if (qty === null) {
        setReceiveError(`Line “${line.description}” needs a quantity of zero or more.`);
        return;
      }
      if (qty === 0) continue;
      const entry: ReceiveLineInput = { line_id: line.id, qty_milli: qty };
      if (isSerialized(line)) {
        const serialNumbers = (receive.serials[line.id] ?? "")
          .split(/[\n,]+/)
          .map((value) => value.trim())
          .filter((value) => value !== "");
        const units = qty / 1000;
        if (qty % 1000 !== 0) {
          setReceiveError(`Line “${line.description}” counts in whole units — round the quantity.`);
          return;
        }
        if (serialNumbers.length === 0) {
          setReceiveError(`Line “${line.description}” is serialized — one serial per unit.`);
          return;
        }
        if (serialNumbers.length !== units) {
          setReceiveError(
            `Line “${line.description}” needs ${units} serial number${units === 1 ? "" : "s"} to match the quantity.`,
          );
          return;
        }
        entry.serial_numbers = serialNumbers;
      }
      receiveLines.push(entry);
      if (line.stock_item_id != null) {
        received.push({ itemId: line.stock_item_id, name: line.description });
      }
    }
    if (receiveLines.length === 0) {
      setReceiveError("Enter a quantity for at least one line.");
      return;
    }
    setReceiveBusy(true);
    // A receipt is a server-computed document with ledger side effects, so
    // the dialog waits for the response instead of patching locally.
    const box: { next: PurchaseOrderDetail | null } = { next: null };
    const result = await runEnvelopeMutation({
      describe: "Receive purchase order",
      buildBody: (base) => ({
        location_id: receive.locationId,
        lines: receiveLines,
        carrier: receive.carrier.trim() === "" ? null : receive.carrier.trim(),
        tracking_ref: receive.trackingRef.trim() === "" ? null : receive.trackingRef.trim(),
        note: receive.note.trim() === "" ? null : receive.note.trim(),
        allow_over_receipt: receive.allowOver,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: async (client, body) => {
        const response = await client.POST("/api/v1/purchase-orders/{id}/receive", {
          params: { path: { id: orderId } },
          body,
        });
        if (response.data !== undefined) box.next = response.data.order;
        return response;
      },
      versions: (response) => [[response.receipt.id, response.mutation.version]],
    });
    setReceiveBusy(false);
    if (result.ok === false) {
      setReceiveError(receiveFailureText(result));
      setReceiveAllowVisible(true);
      return;
    }
    if (box.next !== null) {
      setDetail(box.next);
    } else {
      await refresh();
    }
    setReceiveOpen(false);
    setNotice("Receipt recorded.");
    setReceivedLines(received);
  }

  /**
   * Scan-to-receive: an item or unit stages its open PO line (one unit per
   * scan), a location sets the put-away bin. Nothing commits here — the
   * operator still records the receipt.
   */
  function receiveScan(event: ScanEvent): void {
    const resolution = event.resolution;
    setReceiveScanNotice(null);
    if (resolution.kind === "stock_location") {
      if (resolution.location_id != null) {
        setReceive((cur) => ({ ...cur, locationId: resolution.location_id ?? "" }));
      }
      return;
    }
    const scannedItemId = resolution.stock_item_id ?? resolution.entity_id ?? null;
    const line = detail?.lines.find((candidate) => candidate.stock_item_id === scannedItemId) ?? null;
    if (line === null) {
      setReceiveScanNotice(`“${resolution.input}” is not on this order — nothing to stage.`);
      return;
    }
    if (isSerialized(line)) {
      if (resolution.kind !== "serialized_unit" || resolution.secondary == null || resolution.secondary === "") {
        setReceiveScanNotice(`“${line.description}” is serialized — scan a unit's serial code.`);
        return;
      }
      const serial = resolution.secondary;
      setReceive((cur) => {
        const existing = (cur.serials[line.id] ?? "")
          .split(/[\n,]+/)
          .map((value) => value.trim())
          .filter((value) => value !== "");
        if (existing.includes(serial)) return cur;
        const serials = [...existing, serial];
        return {
          ...cur,
          qtys: { ...cur.qtys, [line.id]: String(serials.length) },
          serials: { ...cur.serials, [line.id]: serials.join("\n") },
        };
      });
    } else {
      setReceive((cur) => {
        const count = (receiveScanCounts[line.id] ?? 0) + 1;
        return { ...cur, qtys: { ...cur.qtys, [line.id]: String(count) } };
      });
      setReceiveScanCounts((cur) => ({ ...cur, [line.id]: (cur[line.id] ?? 0) + 1 }));
    }
  }

  function openLineEdit(line: PurchaseOrderLineView): void {
    setEditingLine(line);
    setLineForm({
      description: line.description,
      qty: formatQtyMilli(line.qty_ordered_milli, ""),
      unitCost: line.unit_cost_minor == null ? "" : (line.unit_cost_minor / 100).toFixed(2),
    });
    setLineError(null);
  }

  async function saveLine(): Promise<void> {
    if (detail === null || editingLine === null) return;
    setLineError(null);
    const description = lineForm.description.trim();
    if (description === "") {
      setLineError("Description is required.");
      return;
    }
    const qty = parseQtyToMilli(lineForm.qty);
    if (qty === null || qty <= 0) {
      setLineError("Quantity must be above zero.");
      return;
    }
    const unitCost = lineForm.unitCost.trim() === "" ? 0 : Math.round(Number(lineForm.unitCost) * 100);
    if (Number.isNaN(unitCost) || unitCost < 0) {
      setLineError("Unit cost must be zero or more.");
      return;
    }
    const box: { next: PurchaseOrderDetail | null } = { next: null };
    setBusy("line-save");
    const result = await runEnvelopeMutation({
      describe: "Update purchase order line",
      entityId: editingLine.id,
      buildBody: (base) => ({
        description,
        qty_ordered_milli: qty,
        unit_cost_minor: unitCost,
        position: editingLine.position,
        stock_item_id: editingLine.stock_item_id ?? null,
        catalog_item_id: editingLine.catalog_item_id ?? null,
        ...base,
      }),
      optimistic: () =>
        setDetail((cur) =>
          cur === null
            ? cur
            : {
                ...cur,
                lines: cur.lines.map((line) =>
                  line.id === editingLine.id
                    ? { ...line, description, qty_ordered_milli: qty, unit_cost_minor: unitCost }
                    : line,
                ),
              },
        ),
      revert: () =>
        setDetail((cur) =>
          cur === null
            ? cur
            : {
                ...cur,
                lines: cur.lines.map((line) => (line.id === editingLine.id ? editingLine : line)),
              },
        ),
      request: async (client, body) => {
        const response = await client.PATCH("/api/v1/purchase-orders/{id}/lines/{line_id}", {
          params: { path: { id: orderId, line_id: editingLine.id } },
          body,
        });
        if (response.data !== undefined) box.next = response.data;
        return response;
      },
      versions: (response) => [[editingLine.id, response.mutation.version]],
    });
    setBusy(null);
    if (result.ok === false) {
      setLineError(rejectionText(result, "Could not save the line."));
      return;
    }
    if (box.next !== null) {
      setDetail(box.next);
    } else {
      await refresh();
    }
    setEditingLine(null);
    setNotice("Line updated.");
  }

  async function deleteLine(line: PurchaseOrderLineView): Promise<void> {
    if (detail === null) return;
    setActionError(null);
    setNotice(null);
    setBusy(`line-delete-${line.id}`);
    const box: { next: PurchaseOrderDetail | null } = { next: null };
    const result = await runEnvelopeMutation({
      describe: "Delete purchase order line",
      entityId: line.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () =>
        setDetail((cur) => (cur === null ? cur : { ...cur, lines: cur.lines.filter((l) => l.id !== line.id) })),
      revert: () => setDetail(detail),
      request: async (client, body) => {
        const response = await client.DELETE("/api/v1/purchase-orders/{id}/lines/{line_id}", {
          params: { path: { id: orderId, line_id: line.id } },
          body,
        });
        if (response.data !== undefined) box.next = response.data;
        return response;
      },
      versions: (response) => [[line.id, response.mutation.version]],
    });
    setBusy(null);
    if (result.ok === false) {
      setActionError(`Could not remove “${line.description}”: ${rejectionText(result, "the server refused.")}`);
      return;
    }
    if (box.next !== null) {
      setDetail(box.next);
    } else {
      await refresh();
    }
    setNotice("Line removed.");
  }

  async function addLink(): Promise<void> {
    setLinkError(null);
    const url = linkForm.url.trim();
    if (url === "") {
      setLinkError("A URL is required.");
      return;
    }
    setLinkBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Add purchase order link",
      buildBody: (base) => ({
        kind: linkForm.kind,
        url,
        label: linkForm.label.trim() === "" ? null : linkForm.label.trim(),
        carrier: linkForm.carrier.trim() === "" ? null : linkForm.carrier.trim(),
        line_id: linkForm.lineId === "" ? null : linkForm.lineId,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/purchase-orders/{id}/links", {
          params: { path: { id: orderId } },
          body,
        }),
      versions: (response) => [[response.link.id, response.mutation.version]],
    });
    setLinkBusy(false);
    if (result.ok === false) {
      setLinkError(rejectionText(result, "Could not add the link. The vendor URL must be https."));
      return;
    }
    setLinkForm(emptyLinkForm());
    setLinkFormOpen(false);
    await refresh();
  }

  async function recordBill(): Promise<void> {
    if (detail === null) return;
    setBillError(null);
    const trimmed = billAmount.trim();
    const amountMinor =
      trimmed === "" ? null : Number.isNaN(Number(trimmed)) ? undefined : Math.round(Number(trimmed) * 100);
    if (amountMinor === undefined || (amountMinor !== null && amountMinor < 0)) {
      setBillError("Enter an amount in major units, or leave it blank.");
      return;
    }
    setBillBusy(true);
    const box: { next: PurchaseOrderDetail | null } = { next: null };
    const result = await runEnvelopeMutation({
      describe: "Record vendor bill",
      entityId: detail.bill?.id,
      buildBody: (base) => ({
        amount_minor: amountMinor,
        bill_number: billNumber.trim() === "" ? null : billNumber.trim(),
        currency: null,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: async (client, body) => {
        const response = await client.PUT("/api/v1/purchase-orders/{id}/bill", {
          params: { path: { id: orderId } },
          body,
        });
        if (response.data !== undefined) box.next = { ...detail, bill: response.data.bill };
        return response;
      },
      versions: (response) => [[response.bill.id, response.mutation.version]],
    });
    setBillBusy(false);
    if (result.ok === false) {
      setBillError(rejectionText(result, "Could not record the bill."));
      return;
    }
    if (box.next !== null) setDetail(box.next);
    setBillFormOpen(false);
    setNotice("Bill recorded.");
  }

  async function postBill(): Promise<void> {
    if (detail === null) return;
    setBillError(null);
    setBillBusy(true);
    const box: { next: PurchaseOrderDetail | null } = { next: null };
    const result = await runEnvelopeMutation({
      describe: "Post vendor bill",
      entityId: detail.bill?.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {},
      revert: () => {},
      request: async (client, body) => {
        const response = await client.POST("/api/v1/purchase-orders/{id}/bill/post", {
          params: { path: { id: orderId } },
          body,
        });
        if (response.data !== undefined) box.next = { ...detail, bill: response.data.bill };
        return response;
      },
      versions: (response) => [[response.bill.id, response.mutation.version]],
    });
    setBillBusy(false);
    if (result.ok === false) {
      setBillError(rejectionText(result, "Could not post the bill."));
      return;
    }
    if (box.next !== null) setDetail(box.next);
    setNotice("Bill posted.");
  }

  if (detail === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;
  if (detail === null) return <Spinner />;

  const order = detail.order;
  const vendorName = deeplinks?.vendor.name ?? null;
  const receipts = detail.receipts;
  const links = deeplinks?.links ?? [];
  const vendorLinks = links.filter((link) => link.subject === "vendor");
  const orderLinks = links.filter((link) => link.subject === "purchase_order");
  const lineLinks = links.filter((link) => link.subject === "purchase_order_line");
  const lineLinkGroups: { key: string; context: string; links: DeeplinkEntry[] }[] = [];
  for (const link of lineLinks) {
    const key = link.line_id ?? `position-${link.line_position ?? 0}`;
    const existing = lineLinkGroups.find((group) => group.key === key);
    const context =
      link.line_description ?? (link.line_position == null ? "Line" : `Line ${link.line_position + 1}`);
    if (existing !== undefined) {
      existing.links.push(link);
    } else {
      lineLinkGroups.push({ key, context, links: [link] });
    }
  }
  const canSend = order.status === "draft";
  const canReceive = order.status === "ordered" || order.status === "partially_received";
  const canClose = order.status === "partially_received" || order.status === "received";
  const canCancel = (order.status === "draft" || order.status === "ordered") && receipts.length === 0;
  const canEditLines = order.status === "draft";
  const showBillCard = canReadFinancials || detail.bill != null || detail.totals != null;
  const bill = detail.bill ?? null;
  const billable = order.status === "partially_received" || order.status === "received" || order.status === "closed";
  const billSummary =
    bill === null
      ? billable
        ? "Record the supplier invoice against this order."
        : "The bill opens once goods are received."
      : bill.posted_at == null
        ? "Recorded, not posted — posting locks the currency snapshot."
        : `Posted ${bill.posted_at.slice(0, 10)}.`;
  const receivedTotalMinor = detail.totals?.received_minor ?? null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <header className="m-ledger-header">
        <div className="m-ledger-title-row">
          <h1 className="m-ledger-title font-mono" data-testid="purchasing-order-number">
            {order.number}
          </h1>
          <OrderStatusTag status={order.status} />
          <div className="m-ledger-header-action">
            {canSend && (
              <Button disabled={busy !== null} onClick={() => void runOrderAction("send")} data-testid="purchasing-send">
                Send order
              </Button>
            )}
            {canReceive && (
              <Button onClick={openReceive} data-testid="purchasing-receive">
                Receive goods
              </Button>
            )}
            {canClose && (
              <Button
                variant="secondary"
                disabled={busy !== null}
                onClick={() => void runOrderAction("close")}
                data-testid="purchasing-close"
              >
                Close
              </Button>
            )}
            {canCancel && (
              <Button
                variant="ghost"
                disabled={busy !== null}
                onClick={() => {
                  setCancelReason("");
                  setCancelOpen(true);
                }}
                data-testid="purchasing-cancel"
              >
                Cancel
              </Button>
            )}
          </div>
        </div>
        <p className="m-ledger-subtitle">
          {vendorName ?? "Vendor"} · {links.length} portal link{links.length === 1 ? "" : "s"} ·{" "}
          <Link href="/purchasing" style={{ color: "var(--m-status-info-fg)" }}>
            Purchase orders
          </Link>
        </p>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {(
            [
              { label: "Expected", value: order.expected_at ?? "—" },
              { label: "Currency", value: order.currency },
              { label: "Vendor reference", value: order.vendor_reference ?? "—" },
              { label: "Ordered", value: order.ordered_at == null ? "—" : order.ordered_at.slice(0, 10) },
            ] as const
          ).map((field) => (
            <div key={field.label} className="grid gap-0.5">
              <span className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                {field.label}
              </span>
              <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
                {field.value}
              </span>
            </div>
          ))}
        </div>
      </header>

      {actionError !== null && (
        <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-action-error">
          {actionError}
        </p>
      )}
      {notice !== null && (
        <p className="text-xs" role="status" style={{ color: "var(--m-status-success-fg)" }} data-testid="purchasing-action-notice">
          {notice}
        </p>
      )}
      {receivedLines.length > 0 && (
        <Card className="m-rise grid gap-2 p-4" data-testid="purchasing-received-dispatch">
          <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            Received into the ledger. Dispatch straight to a ticket without opening it:
          </p>
          <div className="flex flex-wrap gap-1">
            {receivedLines.map((line) => (
              <Button
                key={line.itemId}
                size="sm"
                variant="secondary"
                onClick={() => setDispatch({ open: true, itemId: line.itemId, kind: "consume" })}
                data-testid={`purchasing-dispatch-${line.itemId}`}
              >
                Dispatch {line.name}
              </Button>
            ))}
          </div>
        </Card>
      )}

      <Card className="m-rise grid gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Lines
            </h2>
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              {canEditLines
                ? "Draft order — quantities and costs are editable here."
                : "Sent to the vendor — lines stay read-only."}
            </p>
          </div>
          {detail.totals != null && (
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
              Ordered {money(detail.totals.ordered_minor, order.currency)} · received{" "}
              {money(detail.totals.received_minor, order.currency)}
            </p>
          )}
        </div>
        <div className="m-ledger-card" data-testid="purchasing-lines-card">
          <div className="m-ledger-table-wrap">
          <Table data-testid="purchasing-lines-table">
            <thead>
              <tr>
                <Th>#</Th>
                <Th>Description</Th>
                <Th className="text-right">Ordered</Th>
                <Th className="text-right">Received</Th>
                <Th className="text-right">Unit cost</Th>
                <Th>Receipt</Th>
                {canEditLines && <Th className="text-right">Actions</Th>}
              </tr>
            </thead>
            <tbody>
              {lines.map((line) => (
                <Tr key={line.id} className="row-hover" data-testid={`purchasing-line-row-${line.id}`}>
                  <Td className="font-mono text-xs tabular-nums">{line.position + 1}</Td>
                  <Td>
                    {line.description}
                    {isSerialized(line) && (
                      <span className="ml-1.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
                        serialized
                      </span>
                    )}
                  </Td>
                  <Td className="text-right font-mono text-xs tabular-nums">
                    {formatQtyMilli(line.qty_ordered_milli, lineUnit(line))}
                  </Td>
                  <Td
                    className="text-right font-mono text-xs tabular-nums"
                    data-testid={`purchasing-line-received-${line.id}`}
                  >
                    {formatQtyMilli(line.qty_received_milli, lineUnit(line))}
                  </Td>
                  <Td className="text-right font-mono text-xs tabular-nums">
                    {line.unit_cost_minor == null
                      ? "—"
                      : money(line.unit_cost_minor, order.currency)}
                  </Td>
                  <Td data-testid={`purchasing-line-receipt-${line.id}`}>
                    <ReceiptStateTag state={line.receipt_state} />
                  </Td>
                  {canEditLines && (
                    <Td>
                      <div className="flex justify-end gap-1">
                        {line.unit_cost_minor != null && (
                          <Button size="sm" variant="secondary" onClick={() => openLineEdit(line)} data-testid={`purchasing-line-edit-${line.id}`}>
                            Edit
                          </Button>
                        )}
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={busy === `line-delete-${line.id}`}
                          onClick={() => void deleteLine(line)}
                          data-testid={`purchasing-line-delete-${line.id}`}
                        >
                          Remove
                        </Button>
                      </div>
                    </Td>
                  )}
                </Tr>
              ))}
              {lines.length === 0 && (
                <Tr>
                  <Td colSpan={canEditLines ? 7 : 6}>
                    <EmptyState message="This order has no lines yet." />
                  </Td>
                </Tr>
              )}
            </tbody>
          </Table>
          </div>
          <footer className="m-ledger-footer">
            <span className="m-ledger-shown tabular-nums" data-testid="purchasing-lines-count">
              {lines.length} line{lines.length === 1 ? "" : "s"}
            </span>
            {detail.totals != null ? (
              <span className="m-ledger-hints">
                ordered {money(detail.totals.ordered_minor, order.currency)} · received{" "}
                {money(detail.totals.received_minor, order.currency)}
              </span>
            ) : null}
          </footer>
        </div>
        {canEditLines && lines.some((line) => line.unit_cost_minor == null) && (
          <FieldHint>Line costs are hidden without financial access, so those lines stay read-only.</FieldHint>
        )}
      </Card>

      <Card className="m-rise grid gap-3 p-4">
        <div>
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Receipts
          </h2>
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            Every receipt posts ledger rows at the location the goods landed.
          </p>
        </div>
        {receipts.length === 0 ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }} data-testid="purchasing-receipts-empty">
            Nothing received yet.
          </p>
        ) : (
          <div className="grid gap-3" data-testid="purchasing-receipts">
            {receipts.map((receipt: PurchaseReceiptView) => (
              <div
                key={receipt.id}
                className="grid gap-2 border-t pt-3"
                style={{ borderColor: "var(--m-border)" }}
                data-testid={`purchasing-receipt-${receipt.id}`}
              >
                <div className="flex flex-wrap items-baseline justify-between gap-2">
                  <p className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                    {locationName.get(receipt.location_id) ?? "Unknown location"}
                  </p>
                  <p className="font-mono text-xs" style={{ color: "var(--m-text-muted)" }}>
                    {receipt.received_at.slice(0, 16).replace("T", " ")}
                  </p>
                </div>
                {(receipt.carrier != null || receipt.tracking_ref != null) && (
                  <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                    {[receipt.carrier, receipt.tracking_ref].filter((value) => value != null).join(" · ")}
                  </p>
                )}
                <div className="overflow-x-auto">
                  <Table>
                    <thead>
                      <tr>
                        <Th>Line</Th>
                        <Th className="text-right">Quantity</Th>
                        <Th className="text-right">Ledger</Th>
                      </tr>
                    </thead>
                    <tbody>
                      {receipt.lines.map((receiptLine) => {
                        const poLine = lineById.get(receiptLine.po_line_id);
                        const itemId = poLine?.stock_item_id ?? null;
                        const item = itemId == null ? undefined : stockItemById.get(itemId);
                        return (
                          <Tr key={receiptLine.id}>
                            <Td className="text-xs">{poLine?.description ?? "Removed line"}</Td>
                            <Td className="text-right font-mono text-xs tabular-nums">
                              {formatQtyMilli(receiptLine.qty_milli, poLine === undefined ? "" : lineUnit(poLine))}
                            </Td>
                            <Td className="text-right">
                              {item === undefined ? (
                                <span style={{ color: "var(--m-text-muted)" }}>—</span>
                              ) : (
                                <Link
                                  href={`/stock/${item.id}`}
                                  className="text-xs font-medium"
                                  style={{ color: "var(--m-accent)" }}
                                  data-testid={`purchasing-receipt-ledger-${receiptLine.id}`}
                                >
                                  Ledger
                                </Link>
                              )}
                            </Td>
                          </Tr>
                        );
                      })}
                    </tbody>
                  </Table>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      <Card className="m-rise grid gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Open at the vendor
            </h2>
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Each link opens in a new tab.
            </p>
          </div>
          {links.length > 0 && (
            <Button
              size="sm"
              variant="secondary"
              onClick={() => setLinkFormOpen((v) => !v)}
              data-testid="purchasing-link-form-toggle"
            >
              Add link
            </Button>
          )}
        </div>

        {(links.length === 0 || linkFormOpen) && <FieldHint>Add one link per portal page. URLs must be https.</FieldHint>}

        {links.length > 0 && (
          <div className="grid gap-3" data-testid="purchasing-deeplinks">
            {vendorLinks.length > 0 && (
              <div className="grid gap-1.5">
                <p className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  {vendorName ?? "Vendor"}
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {vendorLinks.map((link) => (
                    <Button key={link.link_id} asChild size="sm" variant="secondary">
                      <a
                        href={link.url}
                        target="_blank"
                        rel="noreferrer"
                        data-testid={`purchasing-deeplink-${link.link_id}`}
                      >
                        {link.label ?? LINK_KIND_LABELS[link.kind]}
                        {link.kind === "tracking" && link.carrier != null ? ` · ${link.carrier}` : ""}
                      </a>
                    </Button>
                  ))}
                </div>
              </div>
            )}
            {orderLinks.length > 0 && (
              <div className="grid gap-1.5">
                <p className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  Order {deeplinks?.number ?? order.number}
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {orderLinks.map((link) => (
                    <Button key={link.link_id} asChild size="sm" variant="secondary">
                      <a
                        href={link.url}
                        target="_blank"
                        rel="noreferrer"
                        data-testid={`purchasing-deeplink-${link.link_id}`}
                      >
                        {link.label ?? LINK_KIND_LABELS[link.kind]}
                        {link.kind === "tracking" && link.carrier != null ? ` · ${link.carrier}` : ""}
                      </a>
                    </Button>
                  ))}
                </div>
              </div>
            )}
            {lineLinkGroups.map((group) => (
              <div key={group.key} className="grid gap-1.5">
                <p className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  {group.context}
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {group.links.map((link) => (
                    <Button key={link.link_id} asChild size="sm" variant="secondary">
                      <a
                        href={link.url}
                        target="_blank"
                        rel="noreferrer"
                        data-testid={`purchasing-deeplink-${link.link_id}`}
                      >
                        {link.label ?? LINK_KIND_LABELS[link.kind]}
                        {link.kind === "tracking" && link.carrier != null ? ` · ${link.carrier}` : ""}
                      </a>
                    </Button>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}

        {(links.length === 0 || linkFormOpen) && (
          <div className="grid gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-link-url">URL</Label>
                <Input
                  id="purchasing-link-url"
                  value={linkForm.url}
                  onChange={(e) => setLinkForm({ ...linkForm, url: e.target.value })}
                  placeholder="https://"
                  data-testid="purchasing-link-url"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-link-label">Label</Label>
                <Input
                  id="purchasing-link-label"
                  value={linkForm.label}
                  onChange={(e) => setLinkForm({ ...linkForm, label: e.target.value })}
                  data-testid="purchasing-link-label"
                />
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-link-kind">Kind</Label>
                <Select
                  value={linkForm.kind}
                  onValueChange={(value) => setLinkForm({ ...linkForm, kind: value as ExternalLinkKind })}
                >
                  <SelectTrigger id="purchasing-link-kind" data-testid="purchasing-link-kind">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {LINK_KINDS.map((kind) => (
                      <SelectItem key={kind} value={kind}>
                        {LINK_KIND_LABELS[kind]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-link-line">Line (optional)</Label>
                <Select
                  value={linkForm.lineId === "" ? "none" : linkForm.lineId}
                  onValueChange={(value) => setLinkForm({ ...linkForm, lineId: value === "none" ? "" : value })}
                >
                  <SelectTrigger id="purchasing-link-line" data-testid="purchasing-link-line">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">Whole order</SelectItem>
                    {lines.map((line) => (
                      <SelectItem key={line.id} value={line.id}>
                        {`Line ${line.position + 1} · ${line.description}`}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-link-carrier">Carrier</Label>
                <Input
                  id="purchasing-link-carrier"
                  value={linkForm.carrier}
                  onChange={(e) => setLinkForm({ ...linkForm, carrier: e.target.value })}
                  data-testid="purchasing-link-carrier"
                />
              </div>
            </div>
            {linkError !== null && (
              <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-link-error">
                {linkError}
              </p>
            )}
            <div className="flex justify-end gap-1">
              {links.length > 0 && (
                <Button variant="secondary" onClick={() => setLinkFormOpen(false)}>
                  Cancel
                </Button>
              )}
              <Button disabled={linkBusy} onClick={() => void addLink()} data-testid="purchasing-link-add">
                Add link
              </Button>
            </div>
          </div>
        )}
      </Card>

      {showBillCard && (
        <Card className="m-rise grid gap-3 p-4" data-testid="purchasing-bill-card">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
                Vendor bill
              </h2>
              <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                {billSummary}
              </p>
            </div>
            <div className="flex flex-wrap gap-1">
              {bill !== null && bill.posted_at == null && !billFormOpen && (
                <Button
                  variant="secondary"
                  disabled={billBusy}
                  onClick={() => {
                    setBillAmount((bill.amount_minor / 100).toFixed(2));
                    setBillNumber(bill.bill_number ?? "");
                    setBillError(null);
                    setBillFormOpen(true);
                  }}
                  data-testid="purchasing-bill-edit"
                >
                  Update amount
                </Button>
              )}
              {bill !== null && bill.posted_at == null && (
                <Button variant="secondary" disabled={billBusy} onClick={() => void postBill()} data-testid="purchasing-bill-post">
                  Post bill
                </Button>
              )}
            </div>
          </div>

          {bill !== null && (
            <div className="grid gap-3 sm:grid-cols-3">
              <div className="grid gap-0.5">
                <span className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  Amount
                </span>
                <span className="font-mono text-[13px] tabular-nums" style={{ color: "var(--m-text)" }} data-testid="purchasing-bill-amount">
                  {bill.amount_minor == null ? "—" : money(bill.amount_minor, bill.currency)}
                </span>
              </div>
              <div className="grid gap-0.5">
                <span className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  Bill number
                </span>
                <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
                  {bill.bill_number ?? "—"}
                </span>
              </div>
              <div className="grid gap-0.5">
                <span className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  Base amount
                </span>
                <span className="font-mono text-[13px] tabular-nums" style={{ color: "var(--m-text)" }}>
                  {bill.posted_at !== null && bill.base_amount_minor == null
                    ? // A posted base-currency bill carries an all-NULL FX
                      // group (ADR 0002): the base fact is the bill itself.
                      `${money(bill.amount_minor ?? 0, bill.currency)} (bill is in the base currency)`
                    : bill.base_amount_minor == null
                      ? "—"
                      : money(bill.base_amount_minor, bill.currency)}
                </span>
              </div>
            </div>
          )}

          {(bill === null || billFormOpen) && billable && (
            <div className="grid gap-2 sm:grid-cols-3 sm:items-end">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-bill-amount-input">Amount</Label>
                <Input
                  id="purchasing-bill-amount-input"
                  inputMode="decimal"
                  value={billAmount}
                  placeholder={receivedTotalMinor == null ? "0.00" : (receivedTotalMinor / 100).toFixed(2)}
                  onChange={(e) => setBillAmount(e.target.value)}
                  data-testid="purchasing-bill-amount-input"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-bill-number">Bill number</Label>
                <Input
                  id="purchasing-bill-number"
                  value={billNumber}
                  onChange={(e) => setBillNumber(e.target.value)}
                  data-testid="purchasing-bill-number"
                />
              </div>
              <div className="flex items-end gap-1">
                {bill !== null && (
                  <Button variant="secondary" onClick={() => setBillFormOpen(false)}>
                    Cancel
                  </Button>
                )}
                <Button disabled={billBusy} onClick={() => void recordBill()} data-testid="purchasing-bill-record">
                  {bill === null ? "Record bill" : "Save bill"}
                </Button>
              </div>
            </div>
          )}

          {bill === null && !billable && (
            <FieldHint>Goods must be received before a bill can be recorded.</FieldHint>
          )}

          {billError !== null && (
            <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-bill-error">
              {billError}
            </p>
          )}
        </Card>
      )}

      <Dialog open={receiveOpen} onOpenChange={setReceiveOpen}>
        <DialogContent title="Receive goods">
          <div className="grid max-h-[80vh] gap-3 overflow-auto p-1">
            <div data-testid="purchasing-receive-scan">
              <ScanBar
                surface="receiving"
                onScan={receiveScan}
                hint="Scan a unit's code to stage its line, or a bin code to set the location."
              />
            </div>
            {receiveScanNotice !== null && (
              <p className="text-xs" role="status" style={{ color: "var(--m-status-warning-fg)" }} data-testid="purchasing-receive-scan-notice">
                {receiveScanNotice}
              </p>
            )}
            <div className="grid gap-1">
              <Label htmlFor="purchasing-receive-location">Location</Label>
              <Select
                value={receive.locationId}
                onValueChange={(value) => setReceive((cur) => ({ ...cur, locationId: value }))}
              >
                <SelectTrigger id="purchasing-receive-location" data-testid="purchasing-receive-location">
                  <SelectValue placeholder="Choose a stock location" />
                </SelectTrigger>
                <SelectContent>
                  {locations.map((location) => (
                    <SelectItem key={location.id} value={location.id}>
                      {location.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHint>Where the goods landed — each line posts a ledger row here.</FieldHint>
            </div>

            {lines.map((line) => (
              <div
                key={line.id}
                className="grid gap-2 border-t pt-3"
                style={{ borderColor: "var(--m-border)" }}
                data-testid={`purchasing-receive-line-${line.id}`}
              >
                <div className="flex flex-wrap items-baseline justify-between gap-2">
                  <p className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                    {line.description}
                  </p>
                  <p className="font-mono text-xs" style={{ color: "var(--m-text-muted)" }}>
                    ordered {formatQtyMilli(line.qty_ordered_milli, lineUnit(line))} · received{" "}
                    {formatQtyMilli(line.qty_received_milli, lineUnit(line))}
                  </p>
                </div>
                <div className="grid gap-1 sm:max-w-50">
                  <Label htmlFor={`purchasing-receive-qty-${line.id}`}>Receive now</Label>
                  <Input
                    id={`purchasing-receive-qty-${line.id}`}
                    inputMode="decimal"
                    value={receive.qtys[line.id] ?? ""}
                    onChange={(e) =>
                      setReceive((cur) => ({ ...cur, qtys: { ...cur.qtys, [line.id]: e.target.value } }))
                    }
                    data-testid={`purchasing-receive-qty-${line.id}`}
                  />
                </div>
                {isSerialized(line) && (
                  <div className="grid gap-1">
                    <Label htmlFor={`purchasing-receive-serials-${line.id}`}>Serial numbers</Label>
                    <textarea
                      id={`purchasing-receive-serials-${line.id}`}
                      className="m-textarea"
                      rows={2}
                      value={receive.serials[line.id] ?? ""}
                      onChange={(e) =>
                        setReceive((cur) => ({ ...cur, serials: { ...cur.serials, [line.id]: e.target.value } }))
                      }
                      placeholder="One serial per unit, one per line"
                      data-testid={`purchasing-receive-serials-${line.id}`}
                    />
                    <FieldHint>One serial per unit — the count must match the received quantity.</FieldHint>
                  </div>
                )}
              </div>
            ))}

            <div className="grid gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-receive-carrier">Carrier</Label>
                <Input
                  id="purchasing-receive-carrier"
                  value={receive.carrier}
                  onChange={(e) => setReceive((cur) => ({ ...cur, carrier: e.target.value }))}
                  data-testid="purchasing-receive-carrier"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-receive-tracking">Tracking ref</Label>
                <Input
                  id="purchasing-receive-tracking"
                  value={receive.trackingRef}
                  onChange={(e) => setReceive((cur) => ({ ...cur, trackingRef: e.target.value }))}
                  data-testid="purchasing-receive-tracking"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-receive-note">Note</Label>
                <Input
                  id="purchasing-receive-note"
                  value={receive.note}
                  onChange={(e) => setReceive((cur) => ({ ...cur, note: e.target.value }))}
                  data-testid="purchasing-receive-note"
                />
              </div>
            </div>

            {receiveError !== null && (
              <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-receive-error">
                {receiveError}
              </p>
            )}
            {receiveAllowVisible && (
              <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                <Checkbox
                  checked={receive.allowOver}
                  onCheckedChange={(value) => setReceive((cur) => ({ ...cur, allowOver: value === true }))}
                  data-testid="purchasing-over-receipt"
                />
                Allow over-receipt — accept more than the ordered quantity and mark the line over
              </label>
            )}

            <DialogActions>
              <Button variant="secondary" onClick={() => setReceiveOpen(false)}>
                Cancel
              </Button>
              <Button disabled={receiveBusy} onClick={() => void submitReceive()} data-testid="purchasing-receive-submit">
                Record receipt
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={editingLine !== null} onOpenChange={(open) => (open ? undefined : setEditingLine(null))}>
        <DialogContent title="Edit line">
          <div className="grid gap-3 p-1">
            <div className="grid gap-1">
              <Label htmlFor="purchasing-line-form-description">Description</Label>
              <Input
                id="purchasing-line-form-description"
                value={lineForm.description}
                onChange={(e) => setLineForm({ ...lineForm, description: e.target.value })}
                data-testid="purchasing-line-form-description"
              />
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-line-form-qty">Quantity</Label>
                <Input
                  id="purchasing-line-form-qty"
                  inputMode="decimal"
                  value={lineForm.qty}
                  onChange={(e) => setLineForm({ ...lineForm, qty: e.target.value })}
                  data-testid="purchasing-line-form-qty"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-line-form-cost">Unit cost</Label>
                <Input
                  id="purchasing-line-form-cost"
                  inputMode="decimal"
                  value={lineForm.unitCost}
                  onChange={(e) => setLineForm({ ...lineForm, unitCost: e.target.value })}
                  data-testid="purchasing-line-form-cost"
                />
              </div>
            </div>
            {lineError !== null && (
              <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-line-form-error">
                {lineError}
              </p>
            )}
            <DialogActions>
              <Button variant="secondary" onClick={() => setEditingLine(null)}>
                Cancel
              </Button>
              <Button disabled={busy === "line-save"} onClick={() => void saveLine()} data-testid="purchasing-line-form-save">
                Save line
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={cancelOpen} onOpenChange={setCancelOpen}>
        <DialogContent title="Cancel purchase order">
          <div className="grid gap-3 p-1">
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Cancelling closes the order for good. Add a reason if the vendor asked for one.
            </p>
            <div className="grid gap-1">
              <Label htmlFor="purchasing-cancel-reason">Reason (optional)</Label>
              <Input
                id="purchasing-cancel-reason"
                value={cancelReason}
                onChange={(e) => setCancelReason(e.target.value)}
                data-testid="purchasing-cancel-reason"
              />
            </div>
            <DialogActions>
              <Button variant="secondary" onClick={() => setCancelOpen(false)}>
                Keep order
              </Button>
              <Button
                variant="destructive"
                disabled={busy !== null}
                onClick={() => {
                  setCancelOpen(false);
                  void runOrderAction("cancel", cancelReason);
                }}
                data-testid="purchasing-cancel-confirm"
              >
                Cancel order
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <MovementDialog
        open={dispatch.open}
        onOpenChange={(open) => setDispatch((cur) => ({ ...cur, open }))}
        items={stockItems}
        locations={locations}
        initialKind={dispatch.kind}
        initialItemId={dispatch.itemId}
        ticketOptions={ticketOptions}
        scanSurface="stock-locations"
        searchableItems
        onRecorded={() => {
          setNotice("Dispatched — the movement is on the chosen ticket's timeline.");
          setDispatch({ open: false });
          void refresh();
        }}
      />
    </div>
  );
}
