// Device asset — read view for the printed asset-tag deep link.

import { router, useLocalSearchParams } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { REASON_COPY } from "@/core/errors";
import { MODELS, fetchAsset } from "@/core/models";
import { useModel } from "@/data/sync-context";
import {
  Button,
  Card,
  EmptyState,
  Loading,
  Mono,
  Pill,
  Screen,
  SectionHeader,
} from "@/ui/primitives";
import { infoPair, type, useTheme } from "@/ui/tokens";

export default function AssetScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const asset = useModel(
    id !== undefined ? MODELS.asset(id) : "asset:missing",
    (client) => fetchAsset(client, id ?? ""),
  );

  if (id === undefined) return null;
  if (asset.status === "error" && asset.data === undefined) {
    return (
      <Screen testID="asset-screen">
        <EmptyState
          message={
            asset.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That asset no longer exists.")
              : (asset.error?.text ?? "Couldn't load this asset.")
          }
        />
      </Screen>
    );
  }
  if (asset.status === "loading" && asset.data === undefined) {
    return (
      <Screen testID="asset-screen" scroll={false}>
        <Loading label="Loading asset" />
      </Screen>
    );
  }

  return (
    <Screen testID="asset-screen">
      <Pressable onPress={() => router.back()} hitSlop={8}>
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>{asset.data?.name}</Text>
        {asset.data?.tag != null ? <Mono muted>{String(asset.data.tag)}</Mono> : null}
        <View style={styles.pills}>
          {asset.data?.status != null ? (
            <Pill pair={tokens.neutral} label={String(asset.data.status).toUpperCase()} />
          ) : null}
          {asset.data?.kind != null ? (
            <Pill pair={infoPair(tokens)} label={String(asset.data.kind).toUpperCase()} />
          ) : null}
        </View>
      </View>
      <SectionHeader title="Details" />
      <Card>
        <Detail label="Ownership" value={String(asset.data?.ownership ?? "—")} />
        <Detail label="Scope" value={String(asset.data?.scope ?? "—")} />
        <Detail
          label="Purchased"
          value={asset.data?.purchase_date != null ? String(asset.data.purchase_date) : "—"}
        />
        <Detail
          label="Observed fields"
          value={
            asset.data?.observed_fields != null
              ? `${Object.keys(asset.data.observed_fields).length} recorded`
              : "—"
          }
        />
      </Card>
      <Card>
        <Text style={[styles.hint, { color: tokens.textMuted }]}>
          Asset lifecycle actions (label reprint, warranty, vault secrets) live on the web admin
          surface today; the mobile asset leg follows the tools lane.
        </Text>
      </Card>
      <Button label="Back" variant="ghost" fullWidth onPress={() => router.back()} />
    </Screen>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  const { tokens } = useTheme();
  return (
    <View style={styles.detailRow}>
      <Text style={[styles.detailLabel, { color: tokens.textMuted }]}>{label.toUpperCase()}</Text>
      <Text style={[styles.detailValue, { color: tokens.text }]}>{value}</Text>
    </View>
  );
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
