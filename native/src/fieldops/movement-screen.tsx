// Move stock — the pack's `17-move` on the Wave 33 field flow.
//
// Purpose: one screen runs the three field movements the tech actually
// performs, all on the same public endpoints the web dialog uses:
//   Use       → POST /stock/consume   (anchored to the active ticket)
//   Transfer  → POST /stock/transfer  (van ↔ shelf)
//   Receive   → POST /stock/receive   (confirm a shipment marker on site)
//
// The grammar is Wave 32 lane 6's, never relaxed: scan or search → stage →
// pick context (ticket, locations) → the human commits. A scan fills; it
// never records. Every commit rides the offline queue with an idempotency
// key, so a movement made in a dead zone replays exactly once.

import { router, useLocalSearchParams } from "expo-router";
import { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import {
  MODELS,
  fetchActiveTimer,
  fetchOnHand,
  fetchStockItems,
  fetchStockLocations,
  fetchTicket,
  type OnHandRow,
  type ScanResolution,
  type StockItemView,
  type StockLocationRow,
} from "@/core/models";
import { consumeStock, receiveStock, transferStock } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { ScanStation } from "@/scan/scan-station";
import { commitConfirmed } from "@/ui/haptics";
import {
  Button,
  Card,
  ErrorNote,
  Field,
  Mono,
  Pill,
  Screen,
  SectionHeader,
  SuccessNote,
} from "@/ui/primitives";
import { PackHeader, PackLabel } from "@/ui/pack";
import { infoPair, shape, type, useTheme } from "@/ui/tokens";

type MoveMode = "consume" | "transfer" | "receive";

const MODES: { mode: MoveMode; label: string; hint: string }[] = [
  { mode: "consume", label: "Use", hint: "Parts leaving stock onto the job." },
  { mode: "transfer", label: "Transfer", hint: "Van ↔ shelf — stock that stays stock." },
  { mode: "receive", label: "Receive", hint: "Confirm goods arriving on site." },
];

const SURFACE_FOR: Record<MoveMode, string> = {
  consume: "ticket-consume",
  transfer: "stock-move",
  receive: "stock-move",
};

export function MovementScreen() {
  const { tokens } = useTheme();
  const params = useLocalSearchParams<{
    mode?: string;
    item?: string;
    from?: string;
    to?: string;
    ticket?: string;
  }>();
  const { mutate } = useMutation();

  const initialMode: MoveMode =
    params.mode === "transfer" || params.mode === "receive" ? params.mode : "consume";
  const [mode, setMode] = useState<MoveMode>(initialMode);
  const [item, setItem] = useState<ScanResolution | null>(null);
  const [fromId, setFromId] = useState<string | null>(params.from ?? null);
  const [toId, setToId] = useState<string | null>(params.to ?? null);
  const [qty, setQty] = useState("1");
  const [note, setNote] = useState("");
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [bannerOk, setBannerOk] = useState(false);

  const locations = useModel(MODELS.stockLocations, fetchStockLocations, {
    maxAgeMs: 5 * 60_000,
  });
  const onHand = useModel(MODELS.onHand, fetchOnHand, { maxAgeMs: 60_000 });
  const items = useModel(MODELS.stockItems, fetchStockItems, { maxAgeMs: 5 * 60_000 });
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const ticketParam = params.ticket ?? timer.data?.ticket_id ?? null;
  const ticket = useModel(
    ticketParam !== null ? MODELS.ticket(ticketParam) : "ticket:missing",
    useCallback(
      (client) => fetchTicket(client, ticketParam ?? ""),
      [ticketParam],
    ),
  );

  // A `?item=<id>` hand-off (e.g. a shipment marker's Confirm receipt) stages
  // the cached item without a scan; the operator still scans units for
  // serialized stock.
  const preselected = useMemo(() => {
    if (params.item === undefined) return null;
    const cached = (items.data ?? []).find((candidate: StockItemView) => candidate.id === params.item);
    if (cached !== undefined) return resolutionForItem(cached);
    const row = (onHand.data ?? []).find(
      (candidate: OnHandRow) => candidate.stock_item_id === params.item,
    );
    if (row !== undefined) {
      return {
        input: row.item_name,
        kind: "stock_item",
        entity_id: row.stock_item_id,
        stock_item_id: row.stock_item_id,
        serial_id: null,
        location_id: null,
        device_asset_id: null,
        display_name: row.item_name,
        secondary: null,
        deep_link: null,
        is_tool: false,
        item_kind: "consumable",
        serial_status: null,
        candidates: [],
      } as ScanResolution;
    }
    return null;
  }, [items.data, onHand.data, params.item]);
  const staged = item ?? preselected;

  const onHandForItem = useMemo(
    () =>
      (onHand.data ?? []).filter(
        (row: OnHandRow) => staged !== null && row.stock_item_id === staged.stock_item_id,
      ),
    [onHand.data, staged],
  );

  const serialized = staged?.item_kind === "serialized";
  const quantity = Number.parseFloat(qty.replace(",", "."));
  const qtyMilli = Number.isFinite(quantity) && quantity > 0 ? Math.round(quantity * 1000) : 0;

  const acceptScan = useCallback(
    (resolution: ScanResolution) => {
      const scannedLocation = resolution.location_id;
      if (resolution.kind === "stock_location" && scannedLocation != null) {
        if (mode === "transfer" && fromId === null) setFromId(scannedLocation);
        else setToId(scannedLocation);
        return;
      }
      setItem(resolution);
      if (resolution.serial_id !== null && resolution.stock_item_id !== null) {
        const row = (onHand.data ?? []).find(
          (candidate) => candidate.stock_item_id === resolution.stock_item_id,
        );
        if (row !== undefined && mode === "consume") setFromId(row.stock_location_id);
      }
    },
    [fromId, mode, onHand.data],
  );

  const locationName = useCallback(
    (id: string | null): string => {
      if (id === null) return "—";
      const row = (locations.data ?? []).find((candidate: StockLocationRow) => candidate.id === id);
      return row?.name ?? id.slice(0, 8);
    },
    [locations.data],
  );

  const canCommit =
    staged !== null &&
    !busy &&
    (mode === "consume"
      ? fromId !== null && (serialized ? staged.serial_id !== null : qtyMilli > 0)
      : mode === "transfer"
        ? fromId !== null &&
          toId !== null &&
          fromId !== toId &&
          (serialized ? staged.serial_id !== null : qtyMilli > 0)
        : toId !== null &&
          (serialized ? staged.serial_id !== null : qtyMilli > 0));

  const commit = useCallback(async () => {
    if (staged === null) return;
    setBusy(true);
    setBanner(null);
    const itemId = staged.stock_item_id ?? "";
    const qtyValue = serialized ? 1000 : qtyMilli;
    const trimmedNote = note.trim();
    const result =
      mode === "consume"
        ? await mutate(consumeStock, {
            itemId,
            itemName: staged.display_name,
            locationId: fromId ?? "",
            qtyMilli: qtyValue,
            ...(staged.serial_id !== null ? { serialId: staged.serial_id } : {}),
            ...(staged.secondary !== null ? { serialNumber: staged.secondary } : {}),
            ...(ticketParam !== null ? { ticketId: ticketParam } : {}),
            ...(trimmedNote !== "" ? { note: trimmedNote } : {}),
          })
        : mode === "transfer"
          ? await mutate(transferStock, {
              itemId,
              itemName: staged.display_name,
              fromLocationId: fromId ?? "",
              fromLocationName: locationName(fromId),
              toLocationId: toId ?? "",
              toLocationName: locationName(toId),
              qtyMilli: qtyValue,
              ...(staged.serial_id !== null ? { serialId: staged.serial_id } : {}),
              ...(staged.secondary !== null ? { serialNumber: staged.secondary } : {}),
              ...(trimmedNote !== "" ? { note: trimmedNote } : {}),
            })
          : await mutate(receiveStock, {
              itemId,
              itemName: staged.display_name,
              locationId: toId ?? "",
              locationName: locationName(toId),
              qtyMilli: qtyValue,
              ...(serialized && staged.secondary != null
                ? { serialNumbers: [staged.secondary] }
                : {}),
              ...(ticketParam !== null ? { ticketId: ticketParam } : {}),
              ...(trimmedNote !== "" ? { note: trimmedNote } : {}),
            });
    setBusy(false);
    setBannerOk(result.ok);
    if (result.ok) {
      commitConfirmed();
      setBanner(
        result.queued
          ? "Saved on this device — the ledger updates when it syncs."
          : mode === "consume"
            ? "Consumed — the movement is on the ledger."
            : mode === "transfer"
              ? "Transferred — both locations updated."
              : "Received — the stock is on the van.",
      );
      setItem(null);
      setNote("");
      setQty("1");
    } else {
      setBanner(result.failure?.text ?? "The movement couldn't be saved.");
    }
  }, [
    fromId,
    locationName,
    mode,
    mutate,
    note,
    qtyMilli,
    serialized,
    staged,
    ticketParam,
    toId,
  ]);

  const searchMatches = useMemo(() => {
    const term = search.trim().toLowerCase();
    if (term.length < 2) return [];
    return (items.data ?? [])
      .filter(
        (candidate: StockItemView) =>
          candidate.name.toLowerCase().includes(term) ||
          (candidate.sku ?? "").toLowerCase().includes(term) ||
          (candidate.part_number ?? "").toLowerCase().includes(term),
      )
      .slice(0, 8);
  }, [items.data, search]);

  return (
    <Screen testID="move-screen">
      <PackHeader
        onBack={() => router.back()}
        backTestID="move-back"
        title="Move stock"
        detail="Scan or search, stage, then confirm."
      />

      <View style={styles.modeRow}>
        {MODES.map((option) => {
          const selected = option.mode === mode;
          return (
            <Pressable
              key={option.mode}
              testID={`move-mode-${option.mode}`}
              onPress={() => setMode(option.mode)}
              accessibilityRole="button"
              accessibilityState={{ selected }}
              style={[
                styles.modeChip,
                {
                  backgroundColor: selected ? tokens.primary : tokens.surface,
                  borderColor: selected ? tokens.primary : tokens.border,
                },
              ]}
            >
              <Text
                style={[
                  styles.modeChipText,
                  { color: selected ? tokens.primaryText : tokens.text },
                ]}
              >
                {option.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      <Text style={[styles.modeHint, { color: tokens.textMuted }]}>
        {MODES.find((option) => option.mode === mode)?.hint ?? ""}
      </Text>

      <ScanStation
        surface={SURFACE_FOR[mode]}
        testIDPrefix="move-scan"
        onAccept={acceptScan}
        staged={staged}
        onClear={() => setItem(null)}
        hint="Scan the item, its unit serial, or a bin code."
      />

      <Card>
        <PackLabel style={{ marginBottom: 6 }}>Or search the catalog</PackLabel>
        <Field
          testID="move-search"
          value={search}
          onChangeText={setSearch}
          placeholder="Name, SKU, or part number"
          autoCapitalize="none"
          autoCorrect={false}
        />
        {searchMatches.map((candidate: StockItemView) => (
          <Pressable
            key={candidate.id}
            testID={`move-result-${candidate.id}`}
            onPress={() => {
              setItem(resolutionForItem(candidate));
              setSearch("");
            }}
            style={styles.row}
          >
            <View style={styles.rowBody}>
              <Text style={[styles.rowTitle, { color: tokens.text }]}>{candidate.name}</Text>
              {candidate.sku != null ? <Mono muted>{candidate.sku}</Mono> : null}
            </View>
          </Pressable>
        ))}
      </Card>

      {ticketParam !== null && (mode === "consume" || mode === "receive") ? (
        <Card>
          <View style={styles.anchorRow}>
            <Pill pair={infoPair(tokens)} label={mode === "receive" ? "ARRIVING FOR" : "CHARGED TO"} />
            <Text style={[styles.anchorText, { color: tokens.text }]} numberOfLines={1}>
              {ticket.data !== undefined
                ? `${ticket.data.number} · ${ticket.data.title}`
                : "The active ticket"}
            </Text>
          </View>
          {timer.data?.ticket_id === ticketParam ? (
            <Text style={[styles.anchorHint, { color: tokens.textMuted }]}>
              The running timer anchors this movement to the same job.
            </Text>
          ) : null}
        </Card>
      ) : null}

      <Card>
        {mode !== "receive" ? (
          <>
            <SectionHeader title={mode === "consume" ? "Use from" : "From"} />
            <LocationChips
              locations={locations.data ?? []}
              selectedId={fromId}
              onSelect={setFromId}
              testIDPrefix="move-from"
              availability={onHandForItem}
            />
          </>
        ) : null}
        {mode !== "consume" ? (
          <>
            <SectionHeader title={mode === "transfer" ? "To" : "Receive into"} />
            <LocationChips
              locations={locations.data ?? []}
              selectedId={toId}
              onSelect={setToId}
              testIDPrefix="move-to"
              excludeId={mode === "transfer" ? fromId : null}
            />
          </>
        ) : null}
        {serialized ? (
          <Text style={[styles.serialNote, { color: tokens.textSecondary }]}>
            Serialized stock moves one unit at a time — the scanned serial is the quantity.
          </Text>
        ) : (
          <Field
            label="Quantity"
            testID="move-qty"
            value={qty}
            onChangeText={setQty}
            keyboardType="decimal-pad"
          />
        )}
        <Field
          label="Note"
          testID="move-note"
          value={note}
          onChangeText={setNote}
          placeholder="Optional — what this movement was for"
        />
        <Button
          label={
            mode === "consume"
              ? "Use from stock"
              : mode === "transfer"
                ? "Transfer stock"
                : "Confirm receipt"
          }
          testID="move-commit"
          fullWidth
          loading={busy}
          disabled={!canCommit}
          onPress={() => {
            void commit();
          }}
        />
      </Card>

      {banner !== null ? (
        bannerOk ? <SuccessNote text={banner} /> : <ErrorNote text={banner} />
      ) : null}
    </Screen>
  );
}

function LocationChips({
  locations,
  selectedId,
  onSelect,
  testIDPrefix,
  excludeId = null,
  availability,
}: {
  locations: StockLocationRow[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  testIDPrefix: string;
  excludeId?: string | null;
  availability?: OnHandRow[];
}) {
  const { tokens } = useTheme();
  return (
    <View style={styles.chipWrap}>
      {locations.map((location) => {
        const selected = location.id === selectedId;
        const excluded = excludeId !== null && location.id === excludeId;
        const stock = (availability ?? []).find(
          (row) => row.stock_location_id === location.id,
        );
        return (
          <Pressable
            key={location.id}
            testID={`${testIDPrefix}-${location.id}`}
            disabled={excluded}
            onPress={() => onSelect(location.id)}
            accessibilityRole="button"
            accessibilityState={{ selected, disabled: excluded }}
            style={[
              styles.locationChip,
              {
                backgroundColor: tokens.surfaceHover,
                borderColor: selected ? tokens.primary : tokens.border,
                opacity: excluded ? 0.4 : 1,
              },
            ]}
          >
            <Text
              style={[
                styles.locationChipText,
                { color: tokens.text },
              ]}
            >
              {location.name}
              {stock !== undefined ? ` · ${formatQty(stock.qty_milli)}` : ""}
            </Text>
          </Pressable>
        );
      })}
      {locations.length === 0 ? (
        <Text style={[styles.serialNote, { color: tokens.textMuted }]}>
          No locations are visible on this device yet.
        </Text>
      ) : null}
    </View>
  );
}

export function resolutionForItem(item: StockItemView): ScanResolution {
  return {
    input: item.sku ?? item.name,
    kind: "stock_item",
    entity_id: item.id,
    stock_item_id: item.id,
    serial_id: null,
    location_id: null,
    device_asset_id: null,
    display_name: item.name,
    secondary: item.sku ?? item.part_number ?? null,
    deep_link: null,
    is_tool: item.is_tool === true,
    item_kind: item.kind ?? null,
    serial_status: null,
    candidates: [],
  } as ScanResolution;
}

function formatQty(qtyMilli: number): string {
  return (qtyMilli / 1000).toLocaleString(undefined, { maximumFractionDigits: 3 });
}

const styles = StyleSheet.create({
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  header: {
    paddingTop: 4,
    gap: 2,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 26,
    letterSpacing: -0.4,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  modeRow: {
    flexDirection: "row",
    gap: 8,
  },
  modeChip: {
    flex: 1,
    minHeight: shape.controlHeight,
    borderRadius: shape.radiusControl,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  modeChipText: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  modeHint: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  anchorRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  anchorText: {
    flex: 1,
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  anchorHint: {
    fontFamily: type.sans,
    fontSize: 12,
  },
  chipWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  locationChip: {
    minHeight: 40,
    paddingHorizontal: 12,
    borderRadius: shape.radiusPill,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  locationChipText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  serialNote: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
  },
  rowBody: {
    flex: 1,
    gap: 2,
  },
  rowTitle: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
});
