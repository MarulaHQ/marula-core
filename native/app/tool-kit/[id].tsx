// Toolbox (kit) detail — the scan-the-box-tag destination.
//
// Purpose: one scan of a toolbox stencil resolves the kit, and this screen
// runs the whole-kit transaction (Wave 32 lane 3: kits are checked out as
// ONE transaction while each unit rides its own ledger leg). Availability is
// per location and honest — a kit that cannot go out says which member is
// missing and how many units are there.

import { router, useLocalSearchParams } from "expo-router";
import { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { REASON_COPY } from "@/core/errors";
import {
  MODELS,
  fetchKitAvailability,
  fetchOnHand,
  fetchStockLocations,
  fetchStockSerials,
  fetchToolCheckouts,
  fetchToolKit,
  type StockLocationRow,
  type ToolCheckoutDetail,
} from "@/core/models";
import { checkinTools, checkoutTools } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { commitConfirmed } from "@/ui/haptics";
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
import { infoPair, shape, type, useTheme } from "@/ui/tokens";

export default function ToolKitScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const { me } = useAuth();
  const { mutate } = useMutation();
  const [locationId, setLocationId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [bannerOk, setBannerOk] = useState(false);

  const kit = useModel(
    id !== undefined ? MODELS.kit(id) : "tools.kit:missing",
    useCallback((client) => fetchToolKit(client, id ?? ""), [id]),
  );
  const locations = useModel(MODELS.stockLocations, fetchStockLocations, {
    maxAgeMs: 5 * 60_000,
  });
  const onHand = useModel(MODELS.onHand, fetchOnHand, { maxAgeMs: 60_000 });
  const serials = useModel(MODELS.stockSerials, fetchStockSerials, { maxAgeMs: 60_000 });
  // Default to a location that actually holds the kit's members, so the
  // screen answers "can I grab this here?" before the tech picks anything.
  const kitMemberIds = useMemo(
    () => new Set((kit.data?.members ?? []).map((member) => member.stock_item_id)),
    [kit.data],
  );
  const preferredLocationId = useMemo(() => {
    const row = (onHand.data ?? []).find(
      (candidate) => kitMemberIds.has(candidate.stock_item_id) && candidate.qty_milli > 0,
    );
    if (row !== undefined) return row.stock_location_id;
    // Serialized members have no on-hand count; their whereabouts live on
    // the serial row.
    const serial = (serials.data ?? []).find(
      (candidate) =>
        candidate.stock_item_id !== null &&
        kitMemberIds.has(candidate.stock_item_id) &&
        candidate.location_id !== null &&
        candidate.status === "in_stock",
    );
    return serial?.location_id ?? null;
  }, [kitMemberIds, onHand.data, serials.data]);
  // The availability query must describe the location the checkout will use,
  // so fall back to the first location until the user picks another.
  const effectiveLocationId =
    locationId ?? preferredLocationId ?? (locations.data ?? [])[0]?.id ?? null;
  const availability = useModel(
    id !== undefined
      ? MODELS.kitAvailability(id, effectiveLocationId ?? undefined)
      : "tools.kit:missing:availability",
    useCallback(
      (client) => fetchKitAvailability(client, id ?? "", effectiveLocationId ?? undefined),
      [id, effectiveLocationId],
    ),
  );
  const checkouts = useModel(MODELS.toolCheckouts, fetchToolCheckouts, { maxAgeMs: 30_000 });

  const openCheckout: ToolCheckoutDetail | undefined = useMemo(
    () =>
      (checkouts.data ?? []).find(
        (row) => row.checkout.kit_id === id && row.checkout.status !== "returned",
      ),
    [checkouts.data, id],
  );
  const mine = me !== null && openCheckout?.checkout.holder_user_id === me.user_id;
  const ready = availability.data?.ready === true;
  const selectedLocation = effectiveLocationId;

  if (id === undefined) return null;
  if (kit.status === "error" && kit.data === undefined) {
    return (
      <Screen testID="tool-kit-screen">
        <EmptyState
          message={
            kit.error?.reason === "not_found"
              ? (REASON_COPY.not_found ?? "That toolbox no longer exists.")
              : (kit.error?.text ?? "Couldn't load this toolbox.")
          }
        />
      </Screen>
    );
  }
  if (kit.status === "loading" && kit.data === undefined) {
    return (
      <Screen testID="tool-kit-screen" scroll={false}>
        <Loading label="Loading toolbox" />
      </Screen>
    );
  }

  const act = async (action: "checkout" | "checkin") => {
    if (me === null || kit.data === undefined) return;
    setBusy(true);
    setBanner(null);
    const result =
      action === "checkout"
        ? await mutate(checkoutTools, {
            holderUserId: me.user_id,
            fromLocationId: selectedLocation ?? openCheckout?.checkout.from_location_id ?? "",
            dueAt: dueTomorrow(),
            items: [],
            kitId: kit.data.kit.id,
            label: `Check out ${kit.data.kit.name}`,
          })
        : await mutate(checkinTools, { checkoutId: openCheckout?.checkout.id ?? "" });
    setBusy(false);
    setBannerOk(result.ok);
    if (result.ok) {
      commitConfirmed();
      setBanner(
        result.queued
          ? "Saved on this device — it syncs when you're back online."
          : action === "checkout"
            ? "Checked out — the kit left as one transaction."
            : "Returned — every unit is back in stock.",
      );
    } else {
      setBanner(result.failure?.text ?? "The change couldn't be saved.");
    }
  };

  return (
    <Screen testID="tool-kit-screen">
      <Pressable onPress={() => router.back()} hitSlop={8} testID="tool-kit-back">
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>{kit.data?.kit.name}</Text>
        <View style={styles.pills}>
          {kit.data?.kit.code != null ? (
            <Pill pair={tokens.neutral} label={String(kit.data.kit.code)} />
          ) : null}
          <Pill pair={infoPair(tokens)} label="TOOLBOX" />
          {ready ? <Pill pair={tokens.success} label="READY" /> : null}
          {openCheckout !== undefined ? (
            <Pill
              pair={mine ? tokens.warning : tokens.neutral}
              label={mine ? "CHECKED OUT TO YOU" : "OUT"}
            />
          ) : null}
        </View>
      </View>

      {banner !== null ? (
        bannerOk ? <SuccessNote text={banner} /> : <ErrorNote text={banner} />
      ) : null}

      <SectionHeader title="Availability" />
      <Card>
        <View style={styles.chipWrap}>
          {(locations.data ?? []).map((location: StockLocationRow) => {
            const selected = location.id === selectedLocation;
            return (
              <Pressable
                key={location.id}
                testID={`kit-location-${location.id}`}
                onPress={() => setLocationId(location.id)}
                accessibilityRole="button"
                accessibilityState={{ selected }}
                style={[
                  styles.locationChip,
                  {
                    backgroundColor: tokens.surfaceHover,
                    borderColor: selected ? tokens.primary : tokens.border,
                  },
                ]}
              >
                <Text
                  style={[
                    styles.locationChipText,
                    { color: selected ? tokens.accentText : tokens.text },
                  ]}
                >
                  {location.name}
                </Text>
              </Pressable>
            );
          })}
        </View>
        {availability.data !== undefined ? (
          <>
            {availability.data.members.map((member) => (
              <View key={member.stock_item_id} style={styles.memberRow}>
                <View style={styles.memberBody}>
                  <Text style={[styles.memberName, { color: tokens.text }]}>
                    {member.item_name}
                  </Text>
                  <Mono muted>
                    {member.available_units} of {member.required_units} available
                  </Mono>
                </View>
                <Pill
                  pair={member.ready ? tokens.success : tokens.warning}
                  label={member.ready ? "READY" : "SHORT"}
                />
              </View>
            ))}
            {availability.data.members.length === 0 ? (
              <Text style={[styles.hint, { color: tokens.textMuted }]}>
                This toolbox has no member tools yet.
              </Text>
            ) : null}
          </>
        ) : (
          <Loading label="Checking availability" />
        )}
      </Card>

      <SectionHeader title="Whole-kit transaction" />
      <Card>
        {openCheckout === undefined ? (
          <>
            <Text style={[styles.hint, { color: tokens.textSecondary }]}>
              Check the kit out as one transaction — every tool rides its own ledger line, and
              the single-holder rule still applies per unit.
            </Text>
            <Button
              label={ready ? "Check out kit to me" : "Check out anyway (short kit)"}
              testID="kit-checkout"
              fullWidth
              loading={busy}
              disabled={me === null || selectedLocation === null}
              onPress={() => void act("checkout")}
            />
            {selectedLocation === null ? (
              <Text style={[styles.hint, { color: tokens.warning.text }]}>
                Pick a location first — the checkout needs a place the kit leaves from.
              </Text>
            ) : null}
          </>
        ) : mine ? (
          <>
            <Text style={[styles.hint, { color: tokens.textSecondary }]}>
              You hold this kit. Returning it closes every outstanding unit in one transaction.
            </Text>
            <Button
              label="Return the kit"
              testID="kit-checkin"
              fullWidth
              loading={busy}
              onPress={() => void act("checkin")}
            />
          </>
        ) : (
          <Text style={[styles.hint, { color: tokens.textSecondary }]}>
            {openCheckout.checkout.holder_user_id !== null
              ? `Someone else holds this kit (due ${new Date(openCheckout.checkout.due_at).toLocaleString()}).`
              : "This kit is out of the crib right now."}
          </Text>
        )}
      </Card>

      <SectionHeader title={`Members${kit.data !== undefined ? ` (${kit.data.members.length})` : ""}`} />
      <Card style={styles.memberList}>
        {(kit.data?.members ?? []).map((member) => (
          <View key={member.id} style={styles.memberRow}>
            <View style={styles.memberBody}>
              <Text style={[styles.memberName, { color: tokens.text }]}>{member.item_name}</Text>
              {member.item_sku != null ? <Mono muted>{member.item_sku}</Mono> : null}
            </View>
            <Mono muted>×{Math.round(member.qty_milli / 1000)}</Mono>
          </View>
        ))}
      </Card>
    </Screen>
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
    letterSpacing: -0.4,
  },
  pills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  chipWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  locationChip: {
    minHeight: 40,
    paddingHorizontal: 12,
    borderRadius: shape.radiusPill,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  locationChipText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  memberRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
    paddingVertical: 4,
  },
  memberBody: {
    flex: 1,
    gap: 2,
  },
  memberName: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  memberList: {
    paddingVertical: 4,
  },
  hint: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
});
