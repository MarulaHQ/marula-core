// Tools register — outstanding checkouts with return, and scan-to-return.
//
// Purpose: the shop's checkout list (holder, due, overdue) with the return
// action for the caller's own tools, plus the field grammar on top: scan a
// tool's serial to return that unit (or check it out), scan a toolbox
// stencil to open the whole-kit transaction. Checkin rides the offline
// queue.

import { router } from "expo-router";
import { useCallback, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import {
  MODELS,
  fetchStockSerials,
  fetchToolCheckouts,
  type ScanResolution,
  type ToolCheckoutDetail,
} from "@/core/models";
import { checkinTools, checkoutTools } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { ScanStation } from "@/scan/scan-station";
import { RequireAuth } from "@/ui/require-auth";
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
  SuccessNote,
} from "@/ui/primitives";
import { accentPair, type, useTheme } from "@/ui/tokens";
import { commitConfirmed } from "@/ui/haptics";

export default function ToolsScreen() {
  const { tokens } = useTheme();
  const { me } = useAuth();
  const { mutate } = useMutation();
  const [banner, setBanner] = useState<string | null>(null);
  const [bannerOk, setBannerOk] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [scanBusy, setScanBusy] = useState(false);
  const checkouts = useModel(MODELS.toolCheckouts, fetchToolCheckouts, { maxAgeMs: 30_000 });
  const serials = useModel(MODELS.stockSerials, fetchStockSerials, { maxAgeMs: 60_000 });

  const serialLabel = (serialId: string | null, itemId: string): string => {
    const serial = (serials.data ?? []).find((candidate) => candidate.id === serialId);
    if (serial !== undefined) {
      return `${serial.item_name} · ${serial.serial_number}`;
    }
    return serialId !== null ? serialId.slice(0, 8) : itemId.slice(0, 8);
  };

  // Scan-to-return: one unit (per-line checkin) or a whole kit (route to the
  // kit screen, which owns the location + availability context).
  const onScan = useCallback(
    async (resolution: ScanResolution) => {
      if (resolution.kind === "tool_kit" && resolution.entity_id !== null) {
        router.push(`/tool-kit/${resolution.entity_id}` as never);
        return;
      }
      if (resolution.kind !== "serialized_unit" || resolution.serial_id === null) return;
      setBanner(null);
      setScanBusy(true);
      const open = (checkouts.data ?? []).find(
        (row: ToolCheckoutDetail) =>
          row.checkout.status !== "returned" &&
          row.lines.some((line) => line.serial_id === resolution.serial_id),
      );
      if (open !== undefined) {
        const line = open.lines.find((candidate) => candidate.serial_id === resolution.serial_id);
        if (open.checkout.holder_user_id !== me?.user_id) {
          setBannerOk(false);
          setBanner(
            `${resolution.display_name} is checked out to someone else — due ${new Date(
              open.checkout.due_at,
            ).toLocaleString()}.`,
          );
          setScanBusy(false);
          return;
        }
        const result = await mutate(checkinTools, {
          checkoutId: open.checkout.id,
          ...(line !== undefined ? { lines: [{ lineId: line.id, condition: "good" }] } : {}),
        });
        setScanBusy(false);
        setBannerOk(result.ok);
        if (result.ok) {
          commitConfirmed();
          setBanner(
            result.queued
              ? `${resolution.display_name} — return queued on this device.`
              : `${resolution.display_name} is back in stock.`,
          );
        } else {
          setBanner(result.failure?.text ?? "The return couldn't be saved.");
        }
        return;
      }
      // Not out: offer a checkout for this unit if it is in stock.
      if (resolution.serial_status === "in_stock" && me !== null) {
        const serial = (serials.data ?? []).find((candidate) => candidate.id === resolution.serial_id);
        if (serial?.location_id != null && serial.stock_item_id !== null) {
          const result = await mutate(checkoutTools, {
            holderUserId: me.user_id,
            fromLocationId: serial.location_id,
            dueAt: new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString(),
            items: [{ itemId: serial.stock_item_id, serialId: resolution.serial_id }],
          });
          setScanBusy(false);
          setBannerOk(result.ok);
          if (result.ok) {
            commitConfirmed();
            setBanner(
              result.queued
                ? `${resolution.display_name} — checkout queued on this device.`
                : `${resolution.display_name} is checked out to you.`,
            );
          } else {
            setBanner(result.failure?.text ?? "The checkout couldn't be saved.");
          }
          return;
        }
      }
      setScanBusy(false);
      setBannerOk(false);
      setBanner(
        `${resolution.display_name} is not out on a checkout held by you.`,
      );
    },
    [checkouts.data, me, mutate, serials.data],
  );

  const rows = (checkouts.data ?? []).filter((row) => row.checkout.status !== "returned");

  return (
    <RequireAuth>
      <Screen testID="tools-screen">
        <Pressable
          onPress={() => router.back()}
          hitSlop={8}
          testID="tools-back"
        >
          <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
        </Pressable>
        <View style={styles.header}>
          <Text style={[styles.title, { color: tokens.text }]}>Tools</Text>
          <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
            {rows.length === 0 ? "Nothing is checked out." : `${rows.length} open checkouts`}
          </Text>
        </View>
        {banner !== null ? (
          bannerOk ? <SuccessNote text={banner} /> : <ErrorNote text={banner} />
        ) : null}
        <ScanStation
          surface="field-return"
          testIDPrefix="tools-scan"
          onAccept={(resolution) => {
            void onScan(resolution);
          }}
          hint="Scan a tool to return it, or a toolbox stencil to open the kit."
        />
        {scanBusy ? <Loading label="Updating the checkout" /> : null}
        {checkouts.status === "error" && checkouts.data === undefined ? (
          <ErrorNote text={checkouts.error?.text ?? "Couldn't load checkouts."} onRetry={checkouts.refresh} />
        ) : null}
        {checkouts.status === "loading" && checkouts.data === undefined ? (
          <Loading label="Loading checkouts" />
        ) : null}
        {checkouts.data !== undefined && rows.length === 0 ? (
          <EmptyState message="No tools are out right now." />
        ) : null}
        <SectionHeader title="" />
        {rows.map((row: ToolCheckoutDetail) => {
          const mine = me !== null && row.checkout.holder_user_id === me.user_id;
          const due = new Date(row.checkout.due_at);
          return (
            <Card key={row.checkout.id} testID={`checkout-${row.checkout.id}`} style={styles.card}>
              <View style={styles.cardHeader}>
                <Text style={[styles.cardTitle, { color: tokens.text }]}>
                  {row.lines.length === 1
                    ? serialLabel(
                        row.lines[0]?.serial_id ?? null,
                        row.lines[0]?.stock_item_id ?? "",
                      )
                    : `${row.lines.length} tools`}
                </Text>
                {row.checkout.overdue ? <Pill pair={tokens.danger} label="OVERDUE" /> : null}
                {mine ? <Pill pair={accentPair(tokens)} label="YOURS" /> : null}
              </View>
              <Mono muted>Due {due.toLocaleString()}</Mono>
              {row.lines.slice(0, 4).map((line) => (
                <Text key={line.id} style={[styles.line, { color: tokens.textSecondary }]}>
                  {serialLabel(line.serial_id, line.stock_item_id)}
                </Text>
              ))}
              {mine ? (
                <Button
                  label="Return all"
                  testID={`checkin-${row.checkout.id}`}
                  variant="secondary"
                  fullWidth
                  loading={busyId === row.checkout.id}
                  onPress={() => {
                    setBusyId(row.checkout.id);
                    setBanner(null);
                    void mutate(checkinTools, { checkoutId: row.checkout.id })
                      .then((result) => {
                        if (result.ok && result.queued) {
                          setBanner("Return queued on this device — it syncs on reconnect.");
                        } else if (!result.ok) {
                          setBanner(result.failure?.text ?? "The return couldn't be saved.");
                        }
                      })
                      .finally(() => setBusyId(null));
                  }}
                />
              ) : null}
            </Card>
          );
        })}
        <Button label="Back" variant="ghost" fullWidth onPress={() => router.back()} />
      </Screen>
    </RequireAuth>
  );
}

const styles = StyleSheet.create({
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  header: {
    paddingTop: 16,
    gap: 2,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 26,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  card: {
    gap: 6,
  },
  cardHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexWrap: "wrap",
  },
  cardTitle: {
    flex: 1,
    fontFamily: type.sansSemiBold,
    fontSize: 16,
  },
  line: {
    fontFamily: type.sans,
    fontSize: 14,
  },
});
