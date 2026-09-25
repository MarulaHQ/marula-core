// Stock item detail — on-hand by location, ledger, and the consume action.
//
// Purpose: the foundation's consumption surface, entirely server-backed.
// On-hand and the movement ledger are read models (live for own writes;
// cross-client stock events are not WS-routed today — recorded contract
// gap); Consume is a queued write with an idempotency key, so an offline
// consumption replays exactly once and reconciles from the ledger.

import { router, useLocalSearchParams } from "expo-router";
import { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

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
import {
  Button,
  Card,
  EmptyState,
  ErrorNote,
  Field,
  Loading,
  Mono,
  Pill,
  Screen,
  SectionHeader,
} from "@/ui/primitives";
import { accentPair, type, useTheme } from "@/ui/tokens";

export default function StockItemScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const [banner, setBanner] = useState<string | null>(null);
  const [locationId, setLocationId] = useState<string | null>(null);
  const [qty, setQty] = useState("1");
  const [note, setNote] = useState("");
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

  if (id === undefined) return null;
  if (item.status === "error" && item.data === undefined) {
    return (
      <Screen testID="stock-item-screen">
        <EmptyState
          message={
            item.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That item no longer exists.")
              : (item.error?.text ?? "Couldn't load this item.")
          }
        />
      </Screen>
    );
  }
  if (item.status === "loading" && item.data === undefined) {
    return (
      <Screen testID="stock-item-screen" scroll={false}>
        <Loading label="Loading item" />
      </Screen>
    );
  }

  const quantity = Number.parseFloat(qty.replace(",", "."));
  const qtyMilli = Number.isFinite(quantity) ? Math.round(quantity * 1000) : Number.NaN;

  return (
    <Screen testID="stock-item-screen">
      <Pressable onPress={() => router.back()} hitSlop={8}>
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>{item.data?.name}</Text>
        <View style={styles.pills}>
          {item.data?.sku != null ? <Pill pair={tokens.neutral} label={String(item.data.sku)} /> : null}
          {item.data?.part_number != null ? (
            <Pill pair={tokens.neutral} label={String(item.data.part_number)} />
          ) : null}
          {item.data?.is_tool === true ? <Pill pair={accentPair(tokens)} label="TOOL" /> : null}
        </View>
      </View>

      {banner !== null ? <ErrorNote text={banner} /> : null}

      <SectionHeader title="On hand" />
      {rows.length === 0 ? (
        <EmptyState message="No stock recorded at any location." />
      ) : (
        <Card>
          {rows.map((row: OnHandRow) => {
            const selected = row.stock_location_id === selectedLocation;
            return (
              <Pressable
                key={row.stock_location_id}
                testID={`onhand-${row.stock_location_id}`}
                onPress={() => setLocationId(row.stock_location_id)}
                style={styles.locationRow}
              >
                <View style={styles.locationBody}>
                  <Text style={[styles.locationName, { color: tokens.text }]}>
                    {row.location_name}
                  </Text>
                  <Mono muted>{formatQty(row.qty_milli)} {item.data?.unit ?? ""}</Mono>
                </View>
                {row.low ? <Pill pair={tokens.warning} label="LOW" /> : null}
                {selected ? <Pill pair={accentPair(tokens)} label="SELECTED" /> : null}
              </Pressable>
            );
          })}
        </Card>
      )}

      <SectionHeader title="Consume from stock" />
      <Card>
        <Field
          label="Quantity"
          testID="consume-qty"
          value={qty}
          onChangeText={setQty}
          keyboardType="decimal-pad"
          hint={selectedRow !== undefined ? `Available: ${formatQty(selectedRow.qty_milli)}` : undefined}
        />
        <Field
          label="Note"
          testID="consume-note"
          value={note}
          onChangeText={setNote}
          placeholder="Charged to the job"
          hint="Consumption records against the chosen location; attach a ticket from the job screen."
        />
        <Button
          label="Consume"
          testID="consume-submit"
          fullWidth
          loading={busy}
          disabled={selectedLocation === null || !Number.isFinite(qtyMilli) || qtyMilli <= 0}
          onPress={() => {
            if (selectedLocation === null || !Number.isFinite(qtyMilli)) return;
            setBusy(true);
            setBanner(null);
            void mutate(consumeStock, {
              itemId: id,
              ...(item.data !== undefined ? { itemName: item.data.name } : {}),
              locationId: selectedLocation,
              qtyMilli,
              ...(note.trim() !== "" ? { note: note.trim() } : {}),
            })
              .then((result) => {
                if (result.ok && result.queued) {
                  setBanner("Queued on this device — the ledger updates when it syncs.");
                } else if (!result.ok) {
                  setBanner(result.failure?.text ?? "Couldn't record the consumption.");
                }
              })
              .finally(() => setBusy(false));
          }}
        />
      </Card>

      <SectionHeader title="Ledger" />
      {movements.status === "error" && movements.data === undefined ? (
        <ErrorNote text={movements.error?.text ?? "Couldn't load the ledger."} onRetry={movements.refresh} />
      ) : null}
      {movements.data !== undefined && movements.data.length === 0 ? (
        <EmptyState message="No movements recorded yet." />
      ) : null}
      {(movements.data ?? []).slice(0, 12).map((movement) => (
        <Card key={movement.id} style={styles.movement}>
          <View style={styles.movementHeader}>
            <Text style={[styles.movementKind, { color: tokens.text }]}>
              {movement.kind.toUpperCase()}
            </Text>
            <Mono muted>{new Date(movement.created_at).toLocaleString()}</Mono>
          </View>
          <Mono muted>
            {formatQty(movement.qty_milli)}
            {movement.note != null && movement.note !== "" ? ` · ${movement.note}` : ""}
          </Mono>
        </Card>
      ))}
    </Screen>
  );
}

function formatQty(qtyMilli: number | null | undefined): string {
  if (qtyMilli == null) return "—";
  return (qtyMilli / 1000).toLocaleString(undefined, {
    maximumFractionDigits: 3,
  });
}

const styles = StyleSheet.create({
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  header: {
    paddingTop: 4,
    gap: 6,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 24,
    letterSpacing: -0.4,
  },
  pills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  locationRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    minHeight: 44,
    paddingVertical: 6,
  },
  locationBody: {
    flex: 1,
    gap: 2,
  },
  locationName: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  movement: {
    gap: 4,
  },
  movementHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  movementKind: {
    fontFamily: type.sansSemiBold,
    fontSize: 13,
    letterSpacing: 0.6,
  },
});
