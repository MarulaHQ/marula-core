// Stock location — the pack's `14-stock-location`: what is in this bin.
//
// Purpose: a bin card, not a cage — name + code in the header, Contents
// grouped, each row 44px+ with item, qty and a blush LOW pill, tapping a
// row into the item. No bottom action bar (the desk moves stock through
// the Move screen).

import { router, useLocalSearchParams } from "expo-router";
import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { REASON_COPY } from "@/core/errors";
import { MODELS, fetchOnHand, fetchStockLocation, type OnHandRow } from "@/core/models";
import { useModel } from "@/data/sync-context";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  PackHeader,
  PackLabel,
  PackPill,
  PACK_ICONS,
  actionReserve,
} from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";

export default function StockLocationScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const location = useModel(
    id !== undefined ? MODELS.location(id) : "location:missing",
    (client) => fetchStockLocation(client, id ?? ""),
  );
  const onHand = useModel(MODELS.onHand, fetchOnHand, { maxAgeMs: 60_000 });
  const rows = useMemo(
    () => (onHand.data ?? []).filter((row: OnHandRow) => row.stock_location_id === id),
    [onHand.data, id],
  );

  if (id === undefined) return null;
  if (location.status === "error" && location.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <EmptyState
          message={
            location.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That location no longer exists.")
              : (location.error?.text ?? "Couldn't load this location.")
          }
        />
      </View>
    );
  }
  if (location.status === "loading" && location.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <Loading label="Loading location" />
      </View>
    );
  }

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.header}>
        <PackHeader
          onBack={() => {
            if (router.canGoBack()) router.back();
            else router.replace("/(tabs)/board");
          }}
          backTestID="stock-location-back"
          title={location.data?.name ?? "Location"}
          detail={location.data?.code != null ? String(location.data.code) : undefined}
        />
      </View>
      <View testID="stock-location-screen" style={[styles.scroll, { paddingBottom: actionReserve() }]}>
        {location.error !== undefined && location.data !== undefined ? (
          <ErrorNote text={location.error.text} onRetry={location.refresh} />
        ) : null}
        <PackLabel style={{ marginBottom: 8 }}>
          {`Contents${rows.length > 0 ? ` (${rows.length})` : ""}`}
        </PackLabel>
        {rows.length === 0 ? <EmptyState message="This location is empty." /> : null}
        <View style={[styles.list, { borderColor: tokens.border }]}>
          {rows.map((row: OnHandRow) => (
            <Pressable
              key={`${row.stock_item_id}-${row.stock_location_id}`}
              testID={`location-row-${row.stock_item_id}`}
              onPress={() => router.push(`/stock-item/${row.stock_item_id}`)}
              accessibilityRole="button"
              style={[styles.row, { borderBottomColor: tokens.border }]}
            >
              <View
                style={[styles.rowDisc, { backgroundColor: tokens.surfaceHover }]}
              >
                <Ionicons name={PACK_ICONS.box} size={18} color={tokens.textSecondary} />
              </View>
              <View style={styles.rowBody}>
                <Text style={[styles.rowTitle, { color: tokens.text }]} numberOfLines={1}>
                  {row.item_name}
                </Text>
                <Text style={[styles.rowQty, { color: tokens.textSecondary }]}>
                  {(row.qty_milli / 1000).toLocaleString()} {row.unit ?? ""}
                </Text>
              </View>
              {row.low ? <PackPill pair={tokens.warning} label="LOW" /> : null}
              <Ionicons name={PACK_ICONS.chev} size={16} color={tokens.textMuted} />
            </Pressable>
          ))}
        </View>
        {rows.length === 0 ? null : (
          <Text style={[styles.note, { color: tokens.textMuted }]}>
            Tap an item for its history and a one-tap use.
          </Text>
        )}
      </View>
    </View>
  );
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
    gap: 14,
  },
  list: {
    borderWidth: 1,
    borderRadius: 14,
    overflow: "hidden",
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 56,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  rowDisc: {
    width: 40,
    height: 40,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  rowBody: {
    flex: 1,
    minWidth: 0,
    gap: 1,
  },
  rowTitle: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  rowQty: {
    fontFamily: type.sans,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
  note: {
    fontFamily: type.sans,
    fontSize: 12,
    textAlign: "center",
  },
});
