// Tools register — the pack's `16-tools` on the checkout read model.
//
// Purpose: the shop's loan list, drawn as rows: holder + loan name, the due
// line, an OUT pill (or red OVERDUE — a genuinely late loan is the one red
// here), and yours carrying Return all. The scan station rides under the
// header (scan a tool to return, a stencil for the kit). Check-out opens
// the due-back sheet from the kit/serial screens; the list is the ledger.

import { router } from "expo-router";
import { useCallback, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

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
import { commitConfirmed } from "@/ui/haptics";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  PackHeader,
  PackLabel,
  PACK_ICONS,
  PackPill,
  PinnedBar,
  PrimaryButton,
} from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";

export default function ToolsScreen() {
  const { tokens } = useTheme();
  const { me } = useAuth();
  const { mutate } = useMutation();
  const [banner, setBanner] = useState<string | null>(null);
  const [bannerOk, setBannerOk] = useState(false);
  const [, setBusyId] = useState<string | null>(null);
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
              ? `${resolution.display_name} — return saved on this device.`
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
                ? `${resolution.display_name} — checkout saved on this device.`
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
      setBanner(`${resolution.display_name} is not out on a checkout held by you.`);
    },
    [checkouts.data, me, mutate, serials.data],
  );

  const rows = (checkouts.data ?? []).filter((row) => row.checkout.status !== "returned");

  return (
    <RequireAuth>
      <View style={[styles.screen, { backgroundColor: tokens.background }]}>
        <View style={styles.header}>
          <PackHeader
            onBack={() => {
              if (router.canGoBack()) router.back();
              else router.replace("/(tabs)/board");
            }}
            backTestID="tools-back"
            title="Tools"
            detail={
              rows.length === 0
                ? "Nothing is checked out."
                : rows.length === 1
                  ? "1 open checkout"
                  : `${rows.length} open checkouts`
            }
          />
        </View>
        <View testID="tools-screen" style={styles.scroll}>
          <ScanStation
            surface="field-return"
            testIDPrefix="tools-scan"
            onAccept={(resolution) => {
              void onScan(resolution);
            }}
            hint="Scan a tool to return it, or a toolbox stencil to open the kit."
          />
          {scanBusy ? <Loading label="Updating the checkout" /> : null}
          {banner !== null ? (
            bannerOk ? <Text style={[styles.banner, { color: tokens.success.text }]}>{banner}</Text> : <ErrorNote text={banner} />
          ) : null}
          {checkouts.status === "error" && checkouts.data === undefined ? (
            <ErrorNote text={checkouts.error?.text ?? "Couldn't load checkouts."} onRetry={checkouts.refresh} />
          ) : null}
          {checkouts.status === "loading" && checkouts.data === undefined ? (
            <Loading label="Loading checkouts" />
          ) : null}
          {checkouts.data !== undefined && rows.length === 0 ? (
            <EmptyState message="No tools are out right now." />
          ) : null}
          {rows.length > 0 ? <PackLabel style={{ marginTop: 8 }}>Out now</PackLabel> : null}
          <View style={[styles.list, { borderColor: tokens.border }]}>
            {rows.map((row: ToolCheckoutDetail) => {
              const mine = me !== null && row.checkout.holder_user_id === me.user_id;
              const due = new Date(row.checkout.due_at);
              return (
                <View
                  key={row.checkout.id}
                  testID={`checkout-${row.checkout.id}`}
                  style={[styles.row, { borderBottomColor: tokens.border }]}
                >
                  <View style={styles.rowBody}>
                    <Text style={[styles.rowTitle, { color: tokens.text }]} numberOfLines={1}>
                      {row.lines.length === 1
                        ? serialLabel(row.lines[0]?.serial_id ?? null, row.lines[0]?.stock_item_id ?? "")
                        : `${row.lines.length} tools`}
                    </Text>
                    <Text style={[styles.rowDue, { color: tokens.textSecondary }]}>
                      Due{" "}
                      {due.toLocaleString(undefined, {
                        month: "short",
                        day: "numeric",
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                      {mine ? " · yours" : ""}
                    </Text>
                    {row.lines.length > 1
                      ? row.lines.slice(0, 3).map((line) => (
                          <Text key={line.id} style={[styles.line, { color: tokens.textSecondary }]} numberOfLines={1}>
                            {serialLabel(line.serial_id, line.stock_item_id)}
                          </Text>
                        ))
                      : null}
                  </View>
                  {row.checkout.overdue ? (
                    <PackPill pair={tokens.danger} label="OVERDUE" />
                  ) : (
                    <PackPill pair={tokens.neutral} label="OUT" />
                  )}
                </View>
              );
            })}
          </View>
        </View>
        {rows.some((row: ToolCheckoutDetail) => row.checkout.holder_user_id === me?.user_id) ? (
          <PinnedBar>
            <PrimaryButton
              label="Return all of mine"
              icon={PACK_ICONS.box}
              style={{ flex: 1 }}
              testID="tools-return-mine"
              onPress={() => {
                const mineRows = rows.filter(
                  (row: ToolCheckoutDetail) => row.checkout.holder_user_id === me?.user_id,
                );
                if (mineRows.length === 0) return;
                setBusyId(mineRows[0]?.checkout.id ?? null);
                setBanner(null);
                void mutate(checkinTools, { checkoutId: mineRows[0]?.checkout.id ?? "" })
                  .then((result) => {
                    if (result.ok && result.queued) {
                      setBannerOk(true);
                      setBanner("Return saved on this device — it syncs on reconnect.");
                    } else if (!result.ok) {
                      setBannerOk(false);
                      setBanner(result.failure?.text ?? "The return couldn't be saved.");
                    }
                  })
                  .finally(() => setBusyId(null));
              }}
            />
          </PinnedBar>
        ) : null}
      </View>
    </RequireAuth>
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
  banner: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  list: {
    borderWidth: 1,
    borderRadius: 14,
    overflow: "hidden",
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    minHeight: 64,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderBottomWidth: 1,
  },
  rowBody: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  rowTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  rowDue: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  line: {
    fontFamily: type.sans,
    fontSize: 12,
  },
});
