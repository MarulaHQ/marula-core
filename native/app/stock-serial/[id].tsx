// Serialized unit — where-is-it-now, check out, return.
//
// Purpose: the tool checkout/checkin proof surface. The whereabouts read
// model is derived server-side (item, holder, location, due, loan); checkout
// and return are queued writes with idempotency keys, so a queued checkout
// replays exactly once when the link returns.

import { router, useLocalSearchParams } from "expo-router";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { REASON_COPY } from "@/core/errors";
import { MODELS, fetchStockSerials } from "@/core/models";
import { checkinTools, checkoutTools } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
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
import { accentPair, type, useTheme } from "@/ui/tokens";

export default function StockSerialScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const { me } = useAuth();
  const { mutate } = useMutation();
  const [banner, setBanner] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const serials = useModel(MODELS.stockSerials, fetchStockSerials, { maxAgeMs: 30_000 });

  const row = (serials.data ?? []).find((candidate) => candidate.id === id);

  if (id === undefined) return null;
  if (serials.status === "loading" && serials.data === undefined) {
    return (
      <Screen testID="stock-serial-screen" scroll={false}>
        <Loading label="Loading unit" />
      </Screen>
    );
  }
  if (row === undefined) {
    return (
      <Screen testID="stock-serial-screen">
        <EmptyState message={REASON_COPY.not_found ?? "That unit no longer exists."} />
        <Button label="Back" variant="secondary" fullWidth onPress={() => router.back()} />
      </Screen>
    );
  }

  const mine = me !== null && row.holder_user_id === me.user_id;
  const canCheckout = row.status === "in_stock" && row.location_id !== null;
  const canReturn = row.status !== "in_stock" && mine && row.checkout_id !== null;
  const isLoan = row.loan_id !== null;

  const act = async (action: "checkout" | "return") => {
    if (me === null) return;
    setBusy(true);
    setBanner(null);
    const result =
      action === "checkout"
        ? await mutate(checkoutTools, {
            holderUserId: me.user_id,
            fromLocationId: row.location_id ?? "",
            dueAt: dueTomorrow(),
            items: [{ itemId: row.stock_item_id, serialId: row.id }],
          })
        : await mutate(checkinTools, { checkoutId: row.checkout_id ?? "" });
    setBusy(false);
    if (result.ok && result.queued) {
      setBanner("Queued on this device — it syncs when you're back online.");
      return;
    }
    if (!result.ok) {
      setBanner(result.failure?.text ?? "The change couldn't be saved.");
    }
  };

  return (
    <Screen testID="stock-serial-screen">
      <Pressable onPress={() => router.back()} hitSlop={8}>
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>{row.item_name}</Text>
        <Mono muted>{row.serial_number}</Mono>
        <View style={styles.pills}>
          <Pill
            pair={row.status === "in_stock" ? tokens.success : tokens.warning}
            label={String(row.status).replace(/_/g, " ").toUpperCase()}
          />
          {row.is_tool ? <Pill pair={accentPair(tokens)} label="TOOL" /> : null}
          {row.overdue ? <Pill pair={tokens.danger} label="OVERDUE" /> : null}
        </View>
      </View>

      {banner !== null ? <ErrorNote text={banner} /> : null}

      <SectionHeader title="Where it is now" />
      <Card>
        <Detail label="Location" value={row.location_name ?? "—"} />
        <Detail label="Holder" value={mine ? "You" : (row.holder_user_id ?? "—")} />
        <Detail
          label="Due back"
          value={row.due_at != null ? new Date(row.due_at).toLocaleString() : "—"}
        />
        {isLoan ? (
          <Detail label="Loan" value="On a client loan — returns ship with the tools lane." />
        ) : null}
      </Card>

      <SectionHeader title="Actions" />
      <Card>
        {canCheckout ? (
          <Button
            label="Check out to me"
            testID="serial-checkout"
            fullWidth
            loading={busy}
            onPress={() => void act("checkout")}
          />
        ) : null}
        {canReturn ? (
          <Button
            label="Return"
            testID="serial-return"
            fullWidth
            loading={busy}
            onPress={() => void act("return")}
          />
        ) : null}
        {!canCheckout && !canReturn ? (
          <Text style={[styles.hint, { color: tokens.textMuted }]}>
            {row.status === "in_stock"
              ? "This unit needs a home location before it can go out."
              : mine
                ? "This unit is out through a client loan — the loan return flow ships with the tools lane."
                : "This unit is held by someone else."}
          </Text>
        ) : null}
        <Button
          label="Open the parent item"
          variant="secondary"
          fullWidth
          onPress={() => router.push(`/stock-item/${row.stock_item_id}`)}
        />
      </Card>
    </Screen>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  const { tokens } = useTheme();
  return (
    <View style={styles.detailRow}>
      <Text style={[styles.detailLabel, { color: tokens.textMuted }]}>{label.toUpperCase()}</Text>
      <Text style={[styles.detailValue, { color: tokens.text }]} numberOfLines={2}>
        {value}
      </Text>
    </View>
  );
}

function dueTomorrow(): string {
  return new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString();
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
  },
  pills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  detailRow: {
    gap: 2,
    paddingVertical: 4,
  },
  detailLabel: {
    fontFamily: type.sansMedium,
    fontSize: 11,
    letterSpacing: 0.8,
  },
  detailValue: {
    fontFamily: type.sans,
    fontSize: 15,
  },
  hint: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
});
