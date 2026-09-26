// Today's pack — the phone mirror of the web day pack (Wave 32 lane 8).
//
// Purpose: the trip-preparation surface. The phone lists what to grab for
// today's stops, where to grab it, and how healthy the stock is, and a tick
// writes the same plan note the desk writes. Ticks never touch the movement
// ledger; grabbing is the existing checkout transaction one tap away.
//
// Non-purpose: no own read model, no local list. It renders
// GET /api/v1/day-pack and posts ticks, like every other surface.

import Ionicons from "@expo/vector-icons/Ionicons";
import { router } from "expo-router";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import {
  MODELS,
  fetchDayPack,
  type DayPackKit,
  type DayPackLine,
  type DayPackTrip,
} from "@/core/models";
import { tickDayPack, untickDayPack } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { tick as tickHaptic } from "@/ui/haptics";
import {
  Button,
  Card,
  Loading,
  Mono,
  Pill,
  SectionHeader,
} from "@/ui/primitives";
import { type, useTheme, type StatusPair } from "@/ui/tokens";

export function DayPackCard() {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const pack = useModel(MODELS.dayPack(), fetchDayPack, { maxAgeMs: 60_000 });
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const data = pack.data;
  const trips: DayPackTrip[] = data?.trips ?? [];
  const collected = data?.counts.collected ?? 0;
  const totalLines = data?.counts.lines ?? 0;
  const unavailable = data?.counts.unavailable ?? 0;

  const toggle = async (lineKey: string, ticked: boolean, ticketId?: string | null) => {
    setBusyKey(lineKey);
    tickHaptic();
    const args = {
      lineKey,
      ...(ticketId !== undefined && ticketId !== null ? { ticketId } : {}),
    };
    await (ticked ? mutate(untickDayPack, args) : mutate(tickDayPack, args));
    setBusyKey(null);
  };

  return (
    <>
      <SectionHeader title="Today's pack" />
      <Card testID="day-pack">
        <View style={styles.headerRow}>
          <View style={styles.headerBody}>
            <Text style={[styles.title, { color: tokens.text }]}>What to grab</Text>
            {pack.offline && data !== undefined ? (
              <Text style={[styles.hint, { color: tokens.textMuted }]}>
                Showing the last sync from this device.
              </Text>
            ) : (
              <Text style={[styles.hint, { color: tokens.textMuted }]}>
                A tick writes a plan note — it never moves stock.
              </Text>
            )}
          </View>
          {data !== undefined ? (
            <View style={styles.pills}>
              <Pill
                pair={collected >= totalLines && totalLines > 0 ? tokens.success : tokens.neutral}
                label={`${collected} OF ${totalLines}`}
              />
              {unavailable > 0 ? (
                <Pill pair={tokens.warning} label={`${unavailable} SHORT`} />
              ) : null}
            </View>
          ) : null}
        </View>

        {pack.status === "loading" && data === undefined ? (
          <Loading label="Loading today's pack" />
        ) : null}

        {data !== undefined && trips.length === 0 ? (
          <Text style={[styles.hint, { color: tokens.textMuted }]}>
            No stops or picks for today. The desk&apos;s list shows up here as soon
            as it exists.
          </Text>
        ) : null}

        {trips.map((trip) => (
          <View key={`trip-${trip.index}`} style={styles.trip}>
            <Text style={[styles.tripTitle, { color: tokens.textSecondary }]}>
              {tripLabel(trip)}
            </Text>
            {trip.lines.map((line) => (
              <LineRow
                key={line.line_key}
                line={line}
                busy={busyKey === line.line_key}
                onToggle={() =>
                  void toggle(line.line_key, line.tick !== null && line.tick !== undefined, line.ticket_id)
                }
              />
            ))}
            {trip.kits.map((kit) => (
              <KitRow
                key={kit.line_key}
                kit={kit}
                busy={busyKey === kit.line_key}
                onToggle={() =>
                  void toggle(kit.line_key, kit.tick !== null && kit.tick !== undefined, kit.ticket_id)
                }
                onGrab={() => router.push(`/tool-kit/${kit.kit_id}` as never)}
              />
            ))}
            {(trip.coverage_warnings ?? []).map((warning) => (
              <Text key={warning} style={[styles.hint, { color: tokens.warning.text }]}>
                {warning}
              </Text>
            ))}
          </View>
        ))}
      </Card>
    </>
  );
}

function tripLabel(trip: DayPackTrip): string {
  const stops = trip.stops.length;
  const stopWord = stops === 1 ? "stop" : "stops";
  if (trip.opens_at_office) return `Trip ${trip.index + 1} · ${stops} ${stopWord} · re-collect at the office`;
  const first = trip.stops[0];
  if (first !== undefined) {
    return `Trip ${trip.index + 1} · ${stops} ${stopWord} · leaves for ${first.site_label ?? first.title}`;
  }
  return `Trip ${trip.index + 1}`;
}

function availabilityPair(
  tokens: ReturnType<typeof useTheme>["tokens"],
  status: string,
): StatusPair {
  if (status === "available") return tokens.success;
  if (status === "partial") return tokens.warning;
  if (status === "unavailable") return tokens.danger;
  return tokens.neutral;
}

function availabilityLabel(status: string): string {
  if (status === "available") return "READY";
  if (status === "partial") return "PARTIAL";
  if (status === "unavailable") return "NONE";
  return "NOT SET";
}

function formatQty(qtyMilli: number): string {
  return (qtyMilli / 1000).toLocaleString(undefined, { maximumFractionDigits: 3 });
}

function LineRow({
  line,
  busy,
  onToggle,
}: {
  line: DayPackLine;
  busy: boolean;
  onToggle: () => void;
}) {
  const { tokens } = useTheme();
  const ticked = line.tick !== null && line.tick !== undefined;
  const source = line.source;
  const where =
    source != null
      ? source.is_tech_vehicle
        ? "already aboard"
        : `at ${source.name}`
      : null;
  return (
    <Pressable
      testID={`pack-line-${line.line_key}`}
      onPress={onToggle}
      disabled={busy}
      accessibilityRole="checkbox"
      accessibilityState={{ checked: ticked }}
      style={[styles.line, { opacity: busy ? 0.6 : 1 }]}
    >
      <View
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
        style={[
          styles.checkbox,
          {
            borderColor: ticked ? tokens.primary : tokens.border,
            backgroundColor: ticked ? tokens.primary : "transparent",
          },
        ]}
      >
        {ticked ? <Ionicons name="checkmark" size={16} color={tokens.primaryText} /> : null}
      </View>
      <View style={styles.lineBody}>
        <View style={styles.lineHeader}>
          <Text
            style={[
              styles.lineName,
              { color: tokens.text, textDecorationLine: ticked ? "line-through" : "none" },
            ]}
            numberOfLines={2}
          >
            {line.item_name}
          </Text>
          <Pill
            pair={availabilityPair(tokens, line.availability.status)}
            label={availabilityLabel(line.availability.status)}
          />
        </View>
        <Mono muted>
          {formatQty(line.qty_milli)} {line.unit}
          {line.provenance === "pick" ? " · pin" : ""}
          {where !== null ? ` · ${where}` : ""}
          {line.availability.holders !== undefined && line.availability.holders.length > 0
            ? ` · out with ${line.availability.holders[0]?.display_name ?? "someone"}`
            : ""}
        </Mono>
      </View>
    </Pressable>
  );
}

function KitRow({
  kit,
  busy,
  onToggle,
  onGrab,
}: {
  kit: DayPackKit;
  busy: boolean;
  onToggle: () => void;
  onGrab: () => void;
}) {
  const { tokens } = useTheme();
  const ticked = kit.tick !== null && kit.tick !== undefined;
  return (
    <View testID={`pack-kit-${kit.kit_id}`} style={styles.line}>
      <Pressable
        onPress={onToggle}
        disabled={busy}
        accessibilityRole="checkbox"
        accessibilityState={{ checked: ticked }}
        hitSlop={8}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
        style={[
          styles.checkbox,
          {
            borderColor: ticked ? tokens.primary : tokens.border,
            backgroundColor: ticked ? tokens.primary : "transparent",
          },
        ]}
      >
        {ticked ? <Ionicons name="checkmark" size={16} color={tokens.primaryText} /> : null}
      </Pressable>
      <View style={styles.lineBody}>
        <View style={styles.lineHeader}>
          <Text
            style={[
              styles.lineName,
              { color: tokens.text, textDecorationLine: ticked ? "line-through" : "none" },
            ]}
            numberOfLines={2}
          >
            {kit.name}
          </Text>
          <Pill
            pair={kit.ready ? tokens.success : tokens.warning}
            label={kit.ready ? "KIT READY" : `${kit.available_units} OF ${kit.required_units}`}
          />
        </View>
        <Mono muted>{toolboxLabel(kit)}</Mono>
        <Button
          label={ticked ? "Open the kit" : "Grab the kit"}
          variant="secondary"
          testID={`pack-kit-grab-${kit.kit_id}`}
          onPress={onGrab}
        />
      </View>
    </View>
  );
}

function toolboxLabel(kit: DayPackKit): string {
  const code = kit.code != null && kit.code !== "" ? ` · ${kit.code}` : "";
  const missing =
    kit.missing !== undefined && kit.missing.length > 0 ? ` · missing ${kit.missing.join(", ")}` : "";
  return `Toolbox · ${kit.member_count} tools${code}${missing}`;
}

const styles = StyleSheet.create({
  headerRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
  },
  headerBody: {
    flex: 1,
    gap: 2,
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
  },
  hint: {
    fontFamily: type.sans,
    fontSize: 12,
    lineHeight: 17,
  },
  pills: {
    flexDirection: "row",
    gap: 6,
  },
  trip: {
    gap: 8,
    marginTop: 4,
  },
  tripTitle: {
    fontFamily: type.mono,
    fontSize: 11,
    letterSpacing: 0.4,
    textTransform: "uppercase",
  },
  line: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
    minHeight: 44,
    paddingVertical: 4,
  },
  checkbox: {
    width: 24,
    height: 24,
    borderRadius: 6,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 2,
  },
  lineBody: {
    flex: 1,
    gap: 3,
  },
  lineHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  lineName: {
    flex: 1,
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  kitHint: {
    fontFamily: type.mono,
    fontSize: 11,
  },
});
