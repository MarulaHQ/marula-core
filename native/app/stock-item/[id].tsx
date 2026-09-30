// Stock item detail — the pack's `15-stock-item` on live data.
//
// Purpose: drawn as the found-sheet's full page: name + SKU, the facts list
// (On hand, Unit, SKU, Part no.), locations with counts and LOW as blush
// (never red), the movement history as time entries (What · qty · when ·
// note), and the bottom action bar with the pack's two verbs: Take/Use and
// Move.
//
// Purpose is pack shape; mechanics stay the foundation's: consume is the
// queued write with an idempotency key, Take/Use pre-fills it, Move jumps
// to the movement screen staged with this item.

import { router, useLocalSearchParams } from "expo-router";
import { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { REASON_COPY } from "@/core/errors";
import {
  MODELS,
  fetchOnHand,
  fetchStockItem,
  fetchStockMovements,
  type OnHandRow,
} from "@/core/models";
import { consumeStock } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  PackHeader,
  PackLabel,
  PackPill,
  PACK_ICONS,
  PinnedBar,
  PrimaryButton,
  SecondaryButton,
  actionReserve,
} from "@/ui/pack";
import { infoPair, type, useTheme } from "@/ui/tokens";

export default function StockItemScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const [banner, setBanner] = useState<string | null>(null);
  const [locationId, setLocationId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const item = useModel(
    id !== undefined ? MODELS.stockItem(id) : "stock.item:missing",
    useCallback(
      (client) => fetchStockItem(client, id ?? ""),
      [id],
    ),
  );
  const onHand = useModel(MODELS.onHand, fetchOnHand, { maxAgeMs: 60_000 });
  const movements = useModel(
    id !== undefined ? MODELS.stockMovements(id) : "stock.item:missing:movements",
    useCallback((client) => fetchStockMovements(client, id ?? ""), [id]),
  );

  const rows = useMemo(
    () => (onHand.data ?? []).filter((row: OnHandRow) => row.stock_item_id === id),
    [onHand.data, id],
  );
  const selectedLocation = locationId ?? rows[0]?.stock_location_id ?? null;
  const selectedRow = rows.find((row) => row.stock_location_id === selectedLocation);
  const unit = item.data?.unit ?? "";

  if (id === undefined) return null;
  if (item.status === "error" && item.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <EmptyState
          message={
            item.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That item no longer exists.")
              : (item.error?.text ?? "Couldn't load this item.")
          }
        />
      </View>
    );
  }
  if (item.status === "loading" && item.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <Loading label="Loading item" />
      </View>
    );
  }

  const takeOne = () => {
    if (selectedLocation === null) return;
    setBusy(true);
    setBanner(null);
    void mutate(consumeStock, {
      itemId: id,
      ...(item.data !== undefined ? { itemName: item.data.name } : {}),
      locationId: selectedLocation,
      qtyMilli: 1000,
    })
      .then((result) => {
        if (result.ok && result.queued) {
          setBanner("Saved on this device — the ledger updates when it syncs.");
        } else if (result.ok) {
          setBanner("Used — the movement is on the ledger.");
        } else {
          setBanner(result.failure?.text ?? "Couldn't record the use.");
        }
      })
      .finally(() => setBusy(false));
  };

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.header}>
        <PackHeader
          onBack={() => {
            if (router.canGoBack()) router.back();
            else router.replace("/(tabs)/board");
          }}
          backTestID="stock-item-back"
          title={item.data?.name ?? "Item"}
          detail={unit !== "" ? `Unit: ${unit}` : undefined}
        />
      </View>

      <View style={[styles.scroll, { paddingBottom: actionReserve() }]} testID="stock-item-screen">
        <View style={styles.pills}>
          {item.data?.sku != null ? <PackPill pair={tokens.neutral} label={String(item.data.sku)} /> : null}
          {item.data?.part_number != null ? (
            <PackPill pair={tokens.neutral} label={String(item.data.part_number)} />
          ) : null}
          {item.data?.is_tool === true ? (
            <PackPill pair={infoPair(tokens)} label="TOOL" />
          ) : null}
        </View>

        {banner !== null ? <ErrorNote text={banner} /> : null}

        <View>
          <PackLabel style={{ marginBottom: 8 }}>On hand</PackLabel>
          {rows.length === 0 ? (
            <EmptyState message="No stock recorded at any location." />
          ) : (
            <View style={styles.rowsCard}>
              {rows.map((row: OnHandRow) => {
                const selected = row.stock_location_id === selectedLocation;
                return (
                  <Pressable
                    key={row.stock_location_id}
                    testID={`onhand-${row.stock_location_id}`}
                    onPress={() => setLocationId(row.stock_location_id)}
                    accessibilityRole="radio"
                    accessibilityState={{ selected }}
                    style={[styles.locationRow, { borderBottomColor: tokens.border }]}
                  >
                    <Ionicons name={PACK_ICONS.location} size={18} color={tokens.textSecondary} />
                    <Text style={[styles.locationName, { color: tokens.text }]}>
                      {row.location_name}
                    </Text>
                    <View style={styles.locationEnd}>
                      <Text style={[styles.qty, { color: tokens.text }]}>
                        {formatQty(row.qty_milli)} {unit}
                      </Text>
                      {row.low ? <PackPill pair={tokens.warning} label="LOW" /> : null}
                    </View>
                  </Pressable>
                );
              })}
            </View>
          )}
        </View>

        <View>
          <PackLabel style={{ marginBottom: 8 }}>Movement history</PackLabel>
          {movements.status === "error" && movements.data === undefined ? (
            <ErrorNote text={movements.error?.text ?? "Couldn't load the history."} onRetry={movements.refresh} />
          ) : null}
          {movements.data !== undefined && movements.data.length === 0 ? (
            <EmptyState message="No movements recorded yet." />
          ) : null}
          <View style={styles.rowsCard}>
            {(movements.data ?? []).slice(0, 12).map((movement, index) => (
              <View
                key={movement.id}
                style={[styles.movementRow, index === 0 ? null : { borderTopColor: tokens.border, borderTopWidth: 1 }]}
              >
                <View style={styles.movementBody}>
                  <Text style={[styles.movementKind, { color: tokens.text }]}>
                    {movementKindLabel(movement.kind)} · {formatQty(movement.qty_milli)} {unit}
                  </Text>
                  <Text style={[styles.movementWhen, { color: tokens.textSecondary }]}>
                    {new Date(movement.created_at).toLocaleString()}
                    {movement.note != null && movement.note !== "" ? ` · ${movement.note}` : ""}
                  </Text>
                </View>
              </View>
            ))}
          </View>
        </View>
      </View>

      <PinnedBar>
        <SecondaryButton
          label={selectedRow !== undefined && selectedRow.low ? "Use 1 (low stock)" : "Use 1"}
          icon={PACK_ICONS.box}
          style={{ flex: 1 }}
          disabled={selectedLocation === null || busy}
          onPress={takeOne}
        />
        <PrimaryButton
          label="Move stock"
          icon={PACK_ICONS.kanban}
          style={{ flex: 1.2 }}
          onPress={() => router.push(`/move?item=${id}` as never)}
        />
      </PinnedBar>
    </View>
  );
}

function movementKindLabel(kind: string): string {
  switch (kind.toLowerCase()) {
    case "consume":
      return "Used";
    case "receive":
      return "Received";
    case "transfer":
      return "Moved";
    default:
      return kind;
  }
}

function formatQty(qtyMilli: number | null | undefined): string {
  if (qtyMilli == null) return "—";
  return (qtyMilli / 1000).toLocaleString(undefined, {
    maximumFractionDigits: 3,
  });
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
  },
  header: {
    paddingHorizontal: 12,
    paddingTop: 8,
  },
  scroll: {
    flex: 1,
    paddingHorizontal: 16,
    paddingTop: 12,
    gap: 18,
  },
  pills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  rowsCard: {
    borderWidth: 1,
    borderRadius: 14,
    borderColor: "transparent",
    overflow: "hidden",
  },
  locationRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 44,
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  locationName: {
    flex: 1,
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  locationEnd: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  qty: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  movementRow: {
    paddingHorizontal: 14,
    paddingVertical: 10,
    minHeight: 44,
    justifyContent: "center",
  },
  movementBody: {
    gap: 2,
  },
  movementKind: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  movementWhen: {
    fontFamily: type.sans,
    fontSize: 12,
  },
});
