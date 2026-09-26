// Stock lookup — availability over the offline cache.
//
// Purpose: the field answer to "do we have one, and where is it?" without a
// venue of network. On-hand rows, the catalog, and tool availability are all
// cached read models, so this screen renders the last synced truth, marks
// itself offline honestly, and reconciles when the link returns.

import Ionicons from "@expo/vector-icons/Ionicons";
import { router } from "expo-router";
import { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import {
  MODELS,
  fetchOnHand,
  fetchStockItems,
  fetchToolsAvailability,
  type OnHandRow,
  type ToolAvailabilityRow,
} from "@/core/models";
import { useModel } from "@/data/sync-context";
import {
  Card,
  EmptyState,
  ErrorNote,
  Field,
  Loading,
  Mono,
  Pill,
  Screen,
} from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export function StockLookupScreen() {
  const { tokens } = useTheme();
  const [query, setQuery] = useState("");

  const items = useModel(MODELS.stockItems, fetchStockItems, { maxAgeMs: 5 * 60_000 });
  const onHand = useModel(MODELS.onHand, fetchOnHand, { maxAgeMs: 60_000 });
  const tools = useModel(MODELS.toolsAvailability(), fetchToolsAvailability, {
    maxAgeMs: 60_000,
  });

  const offline = items.offline || onHand.offline;

  const rows = useMemo(() => {
    const term = query.trim().toLowerCase();
    const list = items.data ?? [];
    const filtered =
      term.length === 0
        ? list
        : list.filter(
            (item) =>
              item.name.toLowerCase().includes(term) ||
              (item.sku ?? "").toLowerCase().includes(term) ||
              (item.part_number ?? "").toLowerCase().includes(term),
          );
    return filtered
      .map((item) => {
        const stock = (onHand.data ?? []).filter(
          (row: OnHandRow) => row.stock_item_id === item.id,
        );
        const totalMilli = stock.reduce((sum, row) => sum + row.qty_milli, 0);
        const availability = (tools.data ?? []).filter(
          (row: ToolAvailabilityRow) => row.stock_item_id === item.id,
        );
        return { item, stock, totalMilli, availability };
      })
      .sort((a, b) => a.item.name.localeCompare(b.item.name));
  }, [items.data, onHand.data, query, tools.data]);

  return (
    <Screen testID="stock-lookup-screen">
      <Pressable onPress={() => router.back()} hitSlop={8} testID="stock-back">
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Stock</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
          Availability from this device’s cache — it updates when you reconnect.
        </Text>
      </View>

      <Field
        testID="stock-search"
        value={query}
        onChangeText={setQuery}
        placeholder="Name, SKU, or part number"
        autoCapitalize="none"
        autoCorrect={false}
        returnKeyType="search"
      />

      {items.status === "error" && items.data === undefined ? (
        <ErrorNote text={items.error?.text ?? "Couldn't load the catalog."} onRetry={items.refresh} />
      ) : null}
      {items.status === "loading" && items.data === undefined ? (
        <Loading label="Loading stock" />
      ) : null}
      {items.data !== undefined && rows.length === 0 ? (
        <EmptyState message="No items match — check the spelling or scan the label." />
      ) : null}
      {offline && items.data !== undefined ? (
        <Text style={[styles.offlineNote, { color: tokens.textMuted }]}>
          Showing the last sync from this device.
        </Text>
      ) : null}

      {rows.map(({ item, stock, totalMilli, availability }) => (
        <Card
          key={item.id}
          testID={`stock-row-${item.id}`}
          onPress={() => router.push(`/stock-item/${item.id}`)}
        >
          <View style={styles.cardHeader}>
            <Text style={[styles.itemName, { color: tokens.text }]} numberOfLines={1}>
              {item.name}
            </Text>
            {item.is_tool === true ? <Pill pair={tokens.neutral} label="TOOL" /> : null}
            {stock.some((row) => row.low) ? <Pill pair={tokens.warning} label="LOW" /> : null}
          </View>
          {item.sku != null || item.part_number != null ? (
            <Mono muted>{[item.sku, item.part_number].filter(Boolean).join(" · ")}</Mono>
          ) : null}
          <View style={styles.locations}>
            {stock.length === 0 ? (
              <Text style={[styles.none, { color: tokens.textMuted }]}>
                None on hand at any location.
              </Text>
            ) : (
              stock.map((row: OnHandRow) => (
                <View key={row.stock_location_id} style={styles.locationRow}>
                  <Text style={[styles.locationName, { color: tokens.textSecondary }]} numberOfLines={1}>
                    {row.location_name}
                  </Text>
                  <Mono>
                    {formatQty(row.qty_milli)} {row.unit ?? ""}
                  </Mono>
                </View>
              ))
            )}
          </View>
          {availability.length > 0 && !availabilityDuplicatesStock(stock, availability) ? (
            <View style={styles.toolRow}>
              <Ionicons name="construct-outline" size={14} color={tokens.textMuted} />
              <Mono muted>
                {availability
                  .map(
                    (row) =>
                      `${row.available_units} available${
                        row.checked_out_units > 0 ? ` · ${row.checked_out_units} out` : ""
                      }${row.location_name != null ? ` · ${row.location_name}` : ""}`,
                  )
                  .join("  |  ")}
              </Mono>
            </View>
          ) : null}
          <Text style={[styles.total, { color: tokens.text }]}>
            {formatQty(totalMilli)} {item.unit ?? ""} on hand
          </Text>
        </Card>
      ))}
    </Screen>
  );
}

/** One location row plus one availability row with the same count say the
 * same thing twice; the location row wins (vision review finding 6 sibling). */
function availabilityDuplicatesStock(stock: OnHandRow[], availability: ToolAvailabilityRow[]): boolean {
  if (stock.length !== 1 || availability.length !== 1) return false;
  const row = stock[0];
  const tool = availability[0];
  if (row === undefined || tool === undefined) return false;
  if (tool.location_name != null && tool.location_name !== row.location_name) return false;
  return Math.round(row.qty_milli / 1000) === tool.available_units;
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
    fontSize: 28,
    letterSpacing: -0.5,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
  cardHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexWrap: "wrap",
  },
  itemName: {
    flex: 1,
    fontFamily: type.sansSemiBold,
    fontSize: 16,
  },
  locations: {
    gap: 4,
  },
  locationRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    minHeight: 28,
  },
  locationName: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 14,
  },
  none: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  toolRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  total: {
    fontFamily: type.sansMedium,
    fontSize: 14,
  },
  offlineNote: {
    fontFamily: type.sans,
    fontSize: 12,
  },
});
