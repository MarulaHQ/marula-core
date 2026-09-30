// Serialized unit — the pack's `13-stock-serial` on the server read model.
//
// Purpose: where-is-it-now as a facts list (Location, Holder, Due back),
// status pills (in stock green / out blush, TOOL, red only for OVERDUE),
// and the single honest verb pinned at the bottom: Check out to me, or
// Return when you hold it. The loan case never pretends a return button —
// it says the loan flow owns the return.

import { router, useLocalSearchParams } from "expo-router";
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { REASON_COPY } from "@/core/errors";
import { MODELS, fetchStockSerials } from "@/core/models";
import { checkinTools, checkoutTools } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { commitConfirmed } from "@/ui/haptics";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  FactRow,
  Facts,
  PackHeader,
  PackLabel,
  PACK_ICONS,
  PackPill,
  PinnedBar,
  PrimaryButton,
} from "@/ui/pack";
import { infoPair, type, useTheme } from "@/ui/tokens";

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
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <Loading label="Loading unit" />
      </View>
    );
  }
  if (row === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <EmptyState message={REASON_COPY.not_found ?? "That unit no longer exists."} />
      </View>
    );
  }

  const mine = me !== null && row.holder_user_id === me.user_id;
  const canCheckout = row.status === "in_stock" && row.location_id !== null;
  const canReturn = row.status !== "in_stock" && mine && row.checkout_id !== null;
  const isLoan = row.loan_id !== null;
  const statusPair = row.status === "in_stock" ? tokens.success : tokens.warning;

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
    if (result.ok) {
      commitConfirmed();
      setBanner(
        result.queued
          ? "Saved on this device — it syncs when you're back online."
          : action === "checkout"
            ? "Checked out — it's on you until tomorrow."
            : "Returned — the unit is back in stock.",
      );
    } else {
      setBanner(result.failure?.text ?? "The change couldn't be saved.");
    }
  };

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.header}>
        <PackHeader
          onBack={() => {
            if (router.canGoBack()) router.back();
            else router.replace("/(tabs)/board");
          }}
          backTestID="stock-serial-back"
          title={row.item_name}
          detail={row.serial_number}
        />
      </View>
      <View testID="stock-serial-screen" style={styles.scroll}>
        <View style={styles.pills}>
          <PackPill pair={statusPair} label={statusLabel(row.status)} />
          {row.is_tool ? <PackPill pair={infoPair(tokens)} label="TOOL" /> : null}
          {row.overdue ? <PackPill pair={tokens.danger} label="OVERDUE" /> : null}
        </View>

        {banner !== null ? <ErrorNote text={banner} /> : null}

        <View>
          <PackLabel style={{ marginBottom: 8 }}>Where it is now</PackLabel>
          <Facts>
            <FactRow first label="Location">
              {row.location_name ?? "—"}
            </FactRow>
            <FactRow label="Holder">{mine ? "You" : (row.holder_user_id ?? "—")}</FactRow>
            <FactRow label="Due back">
              {row.due_at != null ? new Date(row.due_at).toLocaleString() : "—"}
            </FactRow>
          </Facts>
          {isLoan ? (
            <Text style={[styles.hint, { color: tokens.textSecondary }]}>
              On a client loan — the return ships with the loan record, not the tool ledger.
            </Text>
          ) : null}
        </View>

        {!canCheckout && !canReturn && !isLoan ? (
          <Text style={[styles.hint, { color: tokens.textSecondary }]}>
            {row.status === "in_stock"
              ? "This unit needs a home location before it can go out."
              : "This unit is held by someone else."}
          </Text>
        ) : null}
      </View>

      {canCheckout ? (
        <PinnedBar>
          <PrimaryButton
            label="Check out to me"
            testID="serial-checkout"
            style={{ flex: 1 }}
            busy={busy}
            onPress={() => {
              void act("checkout");
            }}
          />
        </PinnedBar>
      ) : canReturn ? (
        <PinnedBar>
          <PrimaryButton
            label="Return"
            testID="serial-return"
            icon={PACK_ICONS.box}
            style={{ flex: 1 }}
            busy={busy}
            onPress={() => {
              void act("return");
            }}
          />
        </PinnedBar>
      ) : (
        <PinnedBar>
          <PrimaryButton
            label="Open the parent item"
            style={{ flex: 1 }}
            onPress={() => router.push(`/stock-item/${row.stock_item_id}` as never)}
          />
        </PinnedBar>
      )}
    </View>
  );
}

function statusLabel(status: string): string {
  switch (status) {
    case "in_stock":
      return "In stock";
    case "checked_out":
      return "Checked out";
    case "out":
      return "Out";
    case "loaned":
      return "Loaned";
    default:
      return status.replace(/_/g, " ");
  }
}

function dueTomorrow(): string {
  return new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString();
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
    gap: 16,
  },
  pills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  hint: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 20,
  },
});
