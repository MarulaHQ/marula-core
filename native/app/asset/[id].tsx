// Device asset — the pack's `19-asset` on the tag deep link.
//
// Purpose: a record, not a form: icon tile, identity pills (Healthy from
// status, WARRANTY +warranty where the org records it), the facts list
// (Ownership, Scope, Purchased, Observed fields), and the honest note that
// lifecycle legs live on the desk today. Move + Add to visit arrive with
// the field-asset lane; the action bar keeps the pack's two verbs ready
// without pretending writes exist.

import Ionicons from "@expo/vector-icons/Ionicons";
import { router, useLocalSearchParams } from "expo-router";
import { StyleSheet, Text, View } from "react-native";

import { REASON_COPY } from "@/core/errors";
import { MODELS, fetchAsset } from "@/core/models";
import { useModel } from "@/data/sync-context";
import { EmptyState, Loading } from "@/ui/primitives";
import {
  FactRow,
  Facts,
  PACK_ICONS,
  PackHeader,
  PackLabel,
  PackPill,
  actionReserve,
} from "@/ui/pack";
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
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <EmptyState
          message={
            asset.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That asset no longer exists.")
              : (asset.error?.text ?? "Couldn't load this asset.")
          }
        />
      </View>
    );
  }
  if (asset.status === "loading" && asset.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <Loading label="Loading asset" />
      </View>
    );
  }

  const healthy = String(asset.data?.status ?? "").toLowerCase() === "active";

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.header}>
        <PackHeader
          onBack={() => {
            if (router.canGoBack()) router.back();
            else router.replace("/(tabs)/board");
          }}
          backTestID="asset-back"
          title={asset.data?.name ?? "Asset"}
          detail={asset.data?.tag != null ? String(asset.data.tag) : undefined}
        />
      </View>
      <View testID="asset-screen" style={[styles.scroll, { paddingBottom: actionReserve() }]}>
        <View style={styles.identity}>
          <View style={[styles.iconTile, { backgroundColor: tokens.surfaceHover }]}>
            <Ionicons name={PACK_ICONS.asset} size={28} color={tokens.text} />
          </View>
          <View style={styles.pills}>
            {healthy ? <PackPill pair={tokens.success} label="HEALTHY" /> : (
              <PackPill pair={tokens.neutral} label={String(asset.data?.status ?? "—").toUpperCase()} />
            )}
            {asset.data?.kind != null ? (
              <PackPill pair={infoPair(tokens)} label={String(asset.data.kind).toUpperCase()} />
            ) : null}
          </View>
        </View>

        <View>
          <PackLabel style={{ marginBottom: 8 }}>Details</PackLabel>
          <Facts>
            <FactRow first label="Ownership">
              {String(asset.data?.ownership ?? "—")}
            </FactRow>
            <FactRow label="Scope">{String(asset.data?.scope ?? "—")}</FactRow>
            <FactRow label="Purchased">
              {asset.data?.purchase_date != null ? String(asset.data.purchase_date) : "—"}
            </FactRow>
            <FactRow label="Observed fields">
              {asset.data?.observed_fields != null
                ? `${Object.keys(asset.data.observed_fields as Record<string, unknown>).length} recorded`
                : "—"}
            </FactRow>
          </Facts>
        </View>

        <Text style={[styles.note, { color: tokens.textSecondary }]}>
          Asset lifecycle actions (label reprint, warranty, vault secrets) live on the web admin
          surface today; the mobile asset leg follows the tools lane.
        </Text>
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
    gap: 16,
  },
  identity: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
  },
  iconTile: {
    width: 64,
    height: 64,
    borderRadius: 16,
    alignItems: "center",
    justifyContent: "center",
  },
  pills: {
    flex: 1,
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  note: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 20,
  },
});
