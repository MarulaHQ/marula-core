// Stock location — what is in this bin right now.

import { router, useLocalSearchParams } from "expo-router";
import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { REASON_COPY } from "@/core/errors";
import { MODELS, fetchOnHand, fetchStockLocation, type OnHandRow } from "@/core/models";
import { useModel } from "@/data/sync-context";
import {
  Button,
  Card,
  EmptyState,
  ErrorNote,
  Loading,
  Mono,
  Pill,
  Screen,
  SectionHeader,
} from "@/ui/primitives";
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
      <Screen testID="stock-location-screen">
        <EmptyState
          message={
            location.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That location no longer exists.")
              : (location.error?.text ?? "Couldn't load this location.")
          }
        />
      </Screen>
    );
  }
  if (location.status === "loading" && location.data === undefined) {
    return (
      <Screen testID="stock-location-screen" scroll={false}>
        <Loading label="Loading location" />
      </Screen>
    );
  }

  return (
    <Screen testID="stock-location-screen">
      <Pressable onPress={() => router.back()} hitSlop={8}>
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>{location.data?.name}</Text>
        {location.data?.code != null ? <Mono muted>{String(location.data.code)}</Mono> : null}
      </View>
      {location.error !== undefined && location.data !== undefined ? (
        <ErrorNote text={location.error.text} onRetry={location.refresh} />
      ) : null}
      <SectionHeader title={`Contents${rows.length > 0 ? ` (${rows.length})` : ""}`} />
      {rows.length === 0 ? <EmptyState message="This location is empty." /> : null}
      <Card style={rows.length > 0 ? styles.listCard : undefined}>
        {rows.map((row: OnHandRow) => (
          <Pressable
            key={`${row.stock_item_id}-${row.stock_location_id}`}
            onPress={() => router.push(`/stock-item/${row.stock_item_id}`)}
            style={styles.row}
          >
            <View style={styles.rowBody}>
              <Text style={[styles.rowTitle, { color: tokens.text }]}>{row.item_name}</Text>
              <Mono muted>{(row.qty_milli / 1000).toLocaleString()} {row.unit ?? ""}</Mono>
            </View>
            {row.low ? <Pill pair={tokens.warning} label="LOW" /> : null}
          </Pressable>
        ))}
      </Card>
      <Button label="Back" variant="ghost" fullWidth onPress={() => router.back()} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  header: {
    paddingTop: 4,
    gap: 4,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 24,
  },
  listCard: {
    padding: 0,
    paddingHorizontal: 16,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 48,
    paddingVertical: 6,
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
