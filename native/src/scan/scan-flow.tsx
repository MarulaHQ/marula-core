// Scan resolution card + resolver hook — one implementation for the Scan
// tab, the deep-link receiver, and (lane 2) the camera view.
//
// Purpose: a scan NEVER guesses and NEVER auto-submits. The resolver returns
// the typed meaning; ambiguous scans render a picker; unknown scans echo the
// text back and say so; accepted kinds offer an explicit "Open" action
// routed by the frozen vocabulary (meridian://stock-item/<uuid>, …).

import { useCallback, useState } from "react";
import { Keyboard, Pressable, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";

import type { ScanResolution } from "@/core/models";
import { kindLabel, resolveScan, scanRoute, scanSummary } from "@/core/scan";
import { useAuth } from "@/auth/auth-context";
import { friendlyMutationError } from "@/core/errors";
import { Button, Card, Pill } from "@/ui/primitives";
import { infoPair, shape, type, useTheme } from "@/ui/tokens";

export function useScanResolver() {
  const { client } = useAuth();
  const [resolution, setResolution] = useState<ScanResolution | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const resolve = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (trimmed.length === 0) return;
      setBusy(true);
      setError(null);
      try {
        const result = await resolveScan(client, trimmed);
        // The typing is done once the resolver answers; leaving the keyboard
        // up hides the resolution card and its actions.
        Keyboard.dismiss();
        setResolution(result);
      } catch (cause) {
        setError(friendlyMutationError(cause).text);
        setResolution(null);
      } finally {
        setBusy(false);
      }
    },
    [client],
  );

  return { resolution, error, busy, resolve, clear: () => setResolution(null) };
}

export function ScanResolutionCard({
  resolution,
  onClear,
}: {
  resolution: ScanResolution;
  onClear?: () => void;
}) {
  const { tokens } = useTheme();
  const candidates = resolution.candidates ?? [];
  const route = scanRoute(resolution);

  return (
    <Card testID="scan-resolution">
      <View style={styles.header}>
        <Pill
          pair={resolution.kind === "unknown" ? tokens.warning : infoPair(tokens)}
          label={kindLabel(resolution.kind).toUpperCase()}
        />
        {resolution.is_tool ? <Pill pair={tokens.neutral} label="TOOL" /> : null}
        {resolution.item_kind === "serialized" ? (
          <Pill pair={tokens.neutral} label="SERIALIZED" />
        ) : null}
        {resolution.serial_status !== null && resolution.serial_status !== undefined ? (
          <Pill
            pair={resolution.serial_status === "in_stock" ? tokens.success : tokens.warning}
            label={String(resolution.serial_status).replace(/_/g, " ").toUpperCase()}
          />
        ) : null}
      </View>
      <Text style={[styles.title, { color: tokens.text }]}>
        {resolution.display_name !== "" ? resolution.display_name : "No match"}
      </Text>
      <Text style={[styles.summary, { color: tokens.textMuted }]}>{scanSummary(resolution)}</Text>

      {resolution.kind === "unknown" && candidates.length === 0 ? (
        <Text style={[styles.summary, { color: tokens.warning.text }]}>
          Nothing matches “{resolution.input}”. Check the label, or enter the code by hand.
        </Text>
      ) : null}

      {candidates.length > 0 ? (
        <View style={styles.candidates}>
          <Text style={[styles.summary, { color: tokens.textSecondary }]}>
            {candidates.length} records match — pick the exact one (a scan never guesses).
          </Text>
          {candidates.map((candidate) => (
            <Pressable
              key={candidate.entity_id}
              onPress={() => {
                router.push(candidateRoute(resolution, candidate.entity_id));
              }}
              accessibilityRole="button"
              style={[styles.candidate, { borderColor: tokens.border }]}
            >
              <Text style={[styles.candidateText, { color: tokens.text }]}>
                {candidate.display_name}
              </Text>
            </Pressable>
          ))}
        </View>
      ) : null}

      {route !== null && candidates.length === 0 ? (
        <Button
          label="Open"
          testID="scan-open"
          fullWidth
          onPress={() => router.push(route as never)}
        />
      ) : null}
      {onClear !== undefined ? (
        <Button label="Clear" variant="secondary" fullWidth onPress={onClear} />
      ) : null}
    </Card>
  );
}

/** Candidate rows resolve to the same route shape as the winning kind. */
function candidateRoute(resolution: ScanResolution, id: string): never {
  const prefix =
    resolution.kind === "stock_item"
      ? "stock-item"
      : resolution.kind === "serialized_unit"
        ? "stock-serial"
        : resolution.kind === "stock_location"
          ? "stock-location"
          : "asset";
  return `/${prefix}/${id}` as never;
}

const styles = StyleSheet.create({
  header: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
  },
  summary: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  candidates: {
    gap: 8,
    paddingTop: 4,
  },
  candidate: {
    borderWidth: 1,
    borderRadius: shape.radiusControl,
    padding: 12,
    minHeight: 44,
    justifyContent: "center",
  },
  candidateText: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
});
