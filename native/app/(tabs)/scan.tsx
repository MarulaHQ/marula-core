// Scan tab — the deterministic injector (DX standard) and the resolver.
//
// Purpose: the dev build ships a scan injector so scan behaviour is testable
// in the simulator (which has no camera) and in Maestro: paste any raw
// scanned string, resolve it through POST /api/v1/scan/resolve, and route
// by the typed kind. The camera button is an honest seam — lane 2 adds
// expo-camera and the TestFlight checklist covers the real lens.
//
// Non-purpose: no local parsing or id guessing (the resolver owns meaning).

import Ionicons from "@expo/vector-icons/Ionicons";
import { useLocalSearchParams } from "expo-router";
import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Button, Card, ErrorNote, Field, Screen, SectionHeader } from "@/ui/primitives";
import { ScanResolutionCard, useScanResolver } from "@/scan/scan-flow";
import { type, useTheme } from "@/ui/tokens";

/** Quick payloads for the injector — one per frozen input class. */
const SAMPLES: { label: string; value: string }[] = [
  { label: "Deep link", value: "meridian://stock-item/00000000-0000-0000-0000-000000000000" },
  { label: "SKU", value: "RSV-SRV-004" },
  { label: "Serial", value: "SN-10004" },
  { label: "Location", value: "BIN-A1" },
];

export default function ScanScreen() {
  const { tokens } = useTheme();
  const { text: injectedText } = useLocalSearchParams<{ text?: string }>();
  const [text, setText] = useState("");
  const { resolution, error, busy, resolve, clear } = useScanResolver();

  // Deep-link handoff: meridian://scan-result?text=… resolves immediately.
  // The injector field stays untouched so a paste in progress is never lost.
  useEffect(() => {
    if (injectedText !== undefined && injectedText !== "") {
      void resolve(injectedText);
    }
  }, [injectedText, resolve]);

  return (
    <Screen testID="scan-screen">
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Scan</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
          Resolve a label, SKU, serial, location code, or asset tag.
        </Text>
      </View>

      <Card>
        <Field
          label="Paste a scan"
          testID="scan-input"
          value={text}
          onChangeText={setText}
          placeholder="meridian://stock-item/… or RSV-1234"
          autoCapitalize="none"
          autoCorrect={false}
          onSubmitEditing={() => {
            void resolve(text);
          }}
          returnKeyType="go"
        />
        <Button
          label="Resolve"
          testID="scan-resolve"
          fullWidth
          loading={busy}
          disabled={text.trim().length === 0 || busy}
          onPress={() => {
            void resolve(text);
          }}
        />
        <SectionHeader title="Samples" />
        <View style={styles.samples}>
          {SAMPLES.map((sample) => (
            <Pressable
              key={sample.label}
              testID={`scan-sample-${sample.label.toLowerCase().replace(" ", "-")}`}
              onPress={() => {
                setText(sample.value);
                void resolve(sample.value);
              }}
              style={[styles.sample, { borderColor: tokens.border }]}
            >
              <Text style={[styles.sampleText, { color: tokens.textSecondary }]}>
                {sample.label}
              </Text>
            </Pressable>
          ))}
        </View>
      </Card>

      {error !== null ? (
        <ErrorNote
          text={error}
          onRetry={() => {
            void resolve(text);
          }}
        />
      ) : null}
      {resolution !== null ? (
        <ScanResolutionCard resolution={resolution} onClear={clear} />
      ) : null}

      <SectionHeader title="Camera" />
      <Card>
        <View style={styles.cameraRow}>
          <Ionicons name="camera-outline" size={22} color={tokens.textMuted} />
          <Text style={[styles.cameraText, { color: tokens.textMuted }]}>
            Camera scanning isn’t available in this build yet. Type or paste a code above —
            every scan path works the same once the camera lands.
          </Text>
        </View>
      </Card>
    </Screen>
  );
}

const styles = StyleSheet.create({
  header: {
    paddingTop: 8,
    gap: 2,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 28,
    letterSpacing: -0.5,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  samples: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  sample: {
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 12,
    minHeight: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  sampleText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  cameraRow: {
    flexDirection: "row",
    gap: 10,
    alignItems: "flex-start",
  },
  cameraText: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
});
