// Stock lookup — the pack's `14-stock-location` grammar over the catalog.
//
// Purpose: the field answer to "do we have one, and where is it?" without a
// vein of network. Rows are the pack's row cards: item name + SKU line,
// per-location counts, blush LOW, TOOL when flagged, and the total. Search
// is an instant filter; the cache truth is labeled honestly when offline.

import Ionicons from "@expo/vector-icons/Ionicons";
import { router } from "expo-router";
import { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import {
  MODELS,
  fetchOnHand,
  fetchStockItems,
  fetchToolsAvailability,
  type OnHandRow,
  type ToolAvailabilityRow,
} from "@/core/models";
import { useModel } from "@/data/sync-context";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  PACK_ICONS,
  actionReserve,
} from "@/ui/pack";
import { chrome, type, useTheme } from "@/ui/tokens";

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
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Stock</Text>
        <Text style={[styles.subtitle, { color: tokens.textSecondary }]}>
          Availability from the device cache — it updates when you reconnect.
        </Text>
      </View>
      <View testID="stock-lookup-screen" style={[styles.scroll, { paddingBottom: actionReserve() }]}>
        <View style={[styles.search, { backgroundColor: tokens.surface, borderColor: tokens.border }]}>
          <Ionicons name={PACK_ICONS.search} size={18} color={tokens.textMuted} />
          <TextInput
            testID="stock-search"
            value={query}
            onChangeText={setQuery}
            placeholder="Name, SKU, or part number"
            placeholderTextColor={tokens.textMuted}
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="search"
            style={[styles.searchInput, { color: tokens.text }]}
          />
        </View>

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

        <View style={[styles.list, { borderColor: tokens.border }]}>
          {rows.map(({ item, stock, totalMilli, availability }) => (
            <Pressable
              key={item.id}
              testID={`stock-row-${item.id}`}
              onPress={() => router.push(`/stock-item/${item.id}`)}
              accessibilityRole="button"
              style={({ pressed }) => [
                styles.rowCard,
                { borderBottomColor: tokens.border, opacity: pressed ? 0.85 : 1 },
              ]}
            >
              <View style={styles.rowHead}>
                <Text style={[styles.itemName, { color: tokens.text }]} numberOfLines={1}>
                  {item.name}
                </Text>
                <Text style={[styles.total, { color: tokens.text }]}>
                  {formatQty(totalMilli)} {item.unit ?? ""}
                </Text>
                <Ionicons name={PACK_ICONS.chev} size={16} color={tokens.textMuted} />
              </View>
              {item.sku != null || item.part_number != null ? (
                <Text style={[styles.skuLine, { color: tokens.textSecondary }]}>
                  {[item.sku, item.part_number].filter(Boolean).join(" · ")}
                </Text>
              ) : null}
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
                    <View style={styles.locationEnd}>
                      <Text style={[styles.locationQty, { color: tokens.text }]}>
                        {formatQty(row.qty_milli)} {row.unit ?? ""}
                      </Text>
                      {row.low ? (
                        <View style={[styles.lowPill, { backgroundColor: tokens.warning.background }]}>
                          <Text style={[styles.lowText, { color: tokens.warning.text }]}>LOW</Text>
                        </View>
                      ) : null}
                    </View>
                  </View>
                ))
              )}
              {availability.length > 0 && !availabilityDuplicatesStock(stock, availability) ? (
                <View style={styles.toolRow}>
                  <Ionicons name="construct-outline" size={14} color={chrome.goldText} />
                  <Text style={[styles.toolText, { color: tokens.textSecondary }]}>
                    {availability
                      .map(
                        (row) =>
                          `${row.available_units} available${
                            row.checked_out_units > 0 ? ` · ${row.checked_out_units} out` : ""
                          }${row.location_name != null ? ` · ${row.location_name}` : ""}`,
                      )
                      .join("  |  ")}
                  </Text>
                </View>
              ) : null}
              {item.is_tool === true ? (
                <View style={[styles.toolPill, { backgroundColor: tokens.neutral.background }]}>
                  <Text style={[styles.lowText, { color: tokens.text }]}>TOOL</Text>
                </View>
              ) : null}
            </Pressable>
          ))}
        </View>
      </View>
    </View>
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
  screen: {
    flex: 1,
  },
  header: {
    paddingHorizontal: 16,
    paddingTop: 12,
    gap: 2,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 26,
    letterSpacing: -0.4,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
  scroll: {
    flex: 1,
    paddingHorizontal: 16,
    paddingTop: 12,
    gap: 12,
  },
  search: {
    height: 48,
    borderRadius: 12,
    borderWidth: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
  },
  searchInput: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 15,
  },
  list: {
    borderWidth: 1,
    borderRadius: 14,
    overflow: "hidden",
  },
  rowCard: {
    paddingHorizontal: 14,
    paddingVertical: 12,
    gap: 6,
    minHeight: 64,
    borderBottomWidth: 1,
  },
  rowHead: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  itemName: {
    flex: 1,
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  total: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  skuLine: {
    fontFamily: type.sans,
    fontSize: 13,
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
  locationEnd: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  locationQty: {
    fontFamily: type.sansMedium,
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  lowPill: {
    height: 24,
    paddingHorizontal: 10,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  lowText: {
    fontFamily: type.sansSemiBold,
    fontSize: 12,
  },
  toolRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  toolText: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 12,
  },
  toolPill: {
    alignSelf: "flex-start",
    height: 24,
    paddingHorizontal: 10,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  none: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  offlineNote: {
    fontFamily: type.sans,
    fontSize: 12,
  },
});
