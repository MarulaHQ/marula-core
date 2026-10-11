// Scan tab — the pack's camera screen with its sheets.
//
// Purpose: drawn as `12-scan-camera` and the scan pack's result sheets. The
// lens view is dark IN BOTH THEMES (camInk), with a hairline reticle, a
// context chip (the ticket a scan is anchored to), the torch disc, the
// keyboard disc (fallback = type a code), Recent chips, and a bottom "Type
// a code" strip. A resolved code becomes a bottom sheet, never a page:
// found (name + facts + Choose an action), ambiguous (radio pick), unknown
// (what happened + Type a code), and the camera-off screen (honestly why +
// big type-a-code field).
//
// Non-purpose: no side effects. A scan opens or stages; it never moves
// stock — the destinations own their forms.

import Ionicons from "@expo/vector-icons/Ionicons";
import * as Device from "expo-device";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useMemo, useState } from "react";
import { Image, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useModel } from "@/data/sync-context";
import { useConnectivity } from "@/data/connectivity";
import { MODELS, fetchActiveTimer } from "@/core/models";
import { CameraScanner } from "@/scan/camera-scanner";
import { useScanResolver } from "@/scan/scan-flow";
import type { ScanResolution } from "@/core/models";
import {
  ChoiceCard,
  FactRow,
  Facts,
  PACK_ICONS,
  PackPill,
  PrimaryButton,
  SecondaryButton,
  Sheet,
} from "@/ui/pack";
import { chrome, type, useTheme } from "@/ui/tokens";

// The canonical marula fruit (cut from the shipped app icon) — the pack's
// "not recognized" sheet draws the mark; we use the real one, not a redraw.
const FRUIT = require("../../assets/marula-fruit.png") as number;

/** Quick payloads — one per frozen input class (simulator + Maestro). */
const SAMPLES: { label: string; value: string }[] = [
  { label: "Deep link", value: "marula://stock-item/00000000-0000-0000-0000-000000000000" },
  { label: "SKU", value: "RSV-SRV-004" },
  { label: "Serial", value: "SN-10004" },
  { label: "Location", value: "BIN-A1" },
];

export default function ScanScreen() {
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const { text: injectedText } = useLocalSearchParams<{ text?: string }>();
  const [typed, setTyped] = useState("");
  const [cameraOpen, setCameraOpen] = useState(false);
  // The pack's torch disc is a real control: gold when on, and it actually
  // drives the lens (the mockup's "torch when on" is gold — not colour
  // alone, the disc's icon stays).
  const [torch, setTorch] = useState(false);
  const { resolution, error, busy, resolve, clear, saved } = useScanResolver();
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const { online } = useConnectivity();

  const contextLabel = timer.data !== null && timer.data !== undefined ? "Timing a ticket" : null;

  useEffect(() => {
    if (injectedText !== undefined && injectedText !== "") {
      void resolve(injectedText);
    }
  }, [injectedText, resolve]);

  const samples = useMemo(() => SAMPLES, []);
  const dark = chrome.camInk;

  return (
    <View
      testID="scan-screen"
      style={[styles.screen, { backgroundColor: dark, paddingTop: insets.top + 12 }]}
    >
      <View style={styles.topRow}>
        <Pressable
          testID="scan-close"
          accessibilityRole="button"
          accessibilityLabel="Close the scanner"
          onPress={() => router.replace("/(tabs)/board" as never)}
          style={styles.disc}
        >
          <Ionicons name={PACK_ICONS.close} size={20} color={chrome.cream} />
        </Pressable>
        <Text style={[styles.title, { color: chrome.cream }]}>Scan</Text>
        <View style={[styles.ctxChip, { backgroundColor: chrome.navDark }]}>
          <Text style={[styles.ctxText, { color: chrome.cream }]} numberOfLines={1}>
            {contextLabel ?? "Scan any label"}
          </Text>
        </View>
        <Pressable
          testID="scan-torch"
          accessibilityRole="button"
          accessibilityLabel={torch ? "Torch on" : "Torch"}
          accessibilityState={{ checked: torch }}
          onPress={() => setTorch((on) => !on)}
          style={[styles.disc, torch ? { backgroundColor: chrome.gold } : null]}
        >
          <Ionicons name={PACK_ICONS.bolt} size={20} color={torch ? chrome.ink : chrome.cream} />
        </Pressable>
        <Pressable
          testID="scan-keyboard"
          accessibilityRole="button"
          accessibilityLabel="Type a code"
          onPress={() => setCameraOpen(false)}
          style={styles.disc}
        >
          <Ionicons name={PACK_ICONS.keyboard} size={20} color={chrome.cream} />
        </Pressable>
      </View>

      <View style={styles.lens}>
        {!Device.isDevice ? (
          <View style={[styles.reticle, { borderColor: chrome.cream }]}>
            <Corner style={styles.tl} />
            <Corner style={styles.tr} />
            <Corner style={styles.bl} />
            <Corner style={styles.br} />
            <Text style={[styles.lensNote, { color: chrome.cream }]}>
              Point at the label — the scan fills the form, you confirm.
            </Text>
          </View>
        ) : null}
      </View>

      <View style={styles.simulatorPanel}>
        {!online ? (
          <View style={[styles.offlineBanner, { backgroundColor: chrome.goldTintLight }]}>
            <Text style={[styles.offlineText, { color: chrome.goldText }]}>
              {"You're offline. Scans are saved on this phone and resolve by "}
              {"themselves when the connection returns."}
            </Text>
          </View>
        ) : null}
        {saved.saved.length > 0 ? (
          <Pressable
            testID="scan-saved"
            accessibilityRole="button"
            accessibilityLabel={`Saved for later, ${saved.saved.length} waiting`}
            onPress={() => {
              const first = saved.saved[0];
              if (first !== undefined) void resolve(first.text);
            }}
            style={[styles.savedRow, { borderColor: chrome.camLine }]}
          >
            <Ionicons name={PACK_ICONS.clock} size={16} color={chrome.cream} />
            <Text style={[styles.savedText, { color: chrome.cream }]}>
              Saved for later · {saved.saved.length} waiting
            </Text>
          </Pressable>
        ) : null}
        <Text style={[styles.panelLabel, { color: chrome.cream }]}>{typedLabel(busy, error)}</Text>        <View style={styles.simRow}>
          {samples.map((sample) => (
            <Pressable
              key={sample.label}
              testID={`scan-sample-${sample.label.toLowerCase().replace(" ", "-")}`}
              onPress={() => {
                setTyped(sample.value);
                void resolve(sample.value);
              }}
              style={[styles.sampleChip, { borderColor: chrome.navDark }]}
            >
              <Text style={[styles.sampleText, { color: chrome.cream }]}>{sample.label}</Text>
            </Pressable>
          ))}
        </View>
        <View style={[styles.typeField, { backgroundColor: chrome.navDark }]}>
          <TextInput
            testID="scan-input"
            value={typed}
            onChangeText={setTyped}
            placeholder="Type or paste a code — SKU, serial, location"
            placeholderTextColor={chrome.cream}
            autoCapitalize="none"
            autoCorrect={false}
            style={[styles.typeInput, { color: chrome.cream }]}
            onSubmitEditing={() => {
              void resolve(typed);
            }}
            returnKeyType="go"
          />
          <Pressable
            testID="scan-resolve"
            accessibilityRole="button"
            accessibilityLabel="Resolve"
            onPress={() => {
              void resolve(typed);
            }}
            style={styles.typeGo}
          >
            <Ionicons name={PACK_ICONS.search} size={22} color={chrome.gold} />
          </Pressable>
        </View>
        <Pressable
          testID="scan-camera"
          accessibilityRole="button"
          accessibilityLabel="Open the camera"
          onPress={() => setCameraOpen(true)}
          style={styles.typeButton}
        >
          <Ionicons name={PACK_ICONS.keyboard} size={18} color={chrome.cream} />
          <Text style={[styles.typeButtonText, { color: chrome.cream }]}>Type a code</Text>
        </Pressable>
      </View>

      <CameraScanner
        visible={cameraOpen}
        onClose={() => setCameraOpen(false)}
        torch={torch}
        onScan={(scanned) => {
          setCameraOpen(false);
          setTyped(scanned);
          void resolve(scanned);
        }}
      />

      <ResultSheets
        resolution={resolution}
        error={error}
        onClear={clear}
        onRetry={() => {
          void resolve(typed);
        }}
      />
    </View>
  );
}

/** The panel's label names what it is: busy, a rejection, or the quick
 * payloads (the chips are fixed sample codes, never a fake "Recent" list). */
function typedLabel(busy: boolean, error: string | null): string {
  if (busy) return "Resolving…";
  if (error !== null) return error;
  return "Try a code";
}

/** The pack's result sheets, one per resolution fate. */
function ResultSheets({
  resolution,
  error,
  onClear,
  onRetry,
}: {
  resolution: ScanResolution | null;
  error: string | null;
  onClear: () => void;
  onRetry: () => void;
}) {
  const { tokens } = useTheme();
  const router = useRouter();
  if (error !== null) {
    return (
      <Sheet visible onClose={onRetry} title="The scan didn't resolve" testID="scan-error-sheet">
        <Text style={[styles.sheetBody, { color: tokens.textSecondary }]}>{error}</Text>
        <PrimaryButton label="Try again" onPress={onRetry} />
        <SecondaryButton label="Close" onPress={onClear} />
      </Sheet>
    );
  }
  if (resolution === null) return null;
  const candidates = resolution.candidates ?? [];
  const unknown = resolution.kind === "unknown" && candidates.length === 0;
  if (unknown) {
    // The pack's dead end that isn't: neutral, the code echoed, and the
    // three ways out in order of effort. "Add as a new asset" has no phone
    // write path yet (recorded gap 2026-10-05) — it names the honest route
    // on the desk instead of pretending to act.
    return (
      <Sheet visible onClose={onClear} title="Not recognized" testID="scan-unknown-sheet">
        <Image
          source={FRUIT}
          style={styles.unknownMark}
          resizeMode="contain"
          accessibilityIgnoresInvertColors
        />
        <PackPill pair={tokens.warning} label="NO MATCH" />
        <Text style={[styles.sheetBody, { color: tokens.textSecondary }]}>
          Nothing matches “{resolution.input}”. We looked for a stock item, a serial, a
          location and an asset tag.
        </Text>
        <PrimaryButton label="Search by name" icon={PACK_ICONS.search} onPress={onClear} />
        <SecondaryButton
          label="Add as a new asset"
          onPress={() => {
            onClear();
            router.push(`/(tabs)/board` as never);
          }}
        />
        <SecondaryButton label="Scan again" icon={PACK_ICONS.scan} onPress={onClear} />
      </Sheet>
    );
  }
  if (candidates.length > 1) {
    return (
      <AmbiguousSheet
        resolution={resolution}
        candidates={candidates}
        onClear={onClear}
      />
    );
  }
  return (
    <Sheet
      visible
      onClose={onClear}
      title={resolution.display_name !== "" ? resolution.display_name : "Found"}
      subtitle={scanKinds(resolution.kind)}
      testID="scan-found-sheet"
    >
      <Facts>
        <FactRow first label="Code">
          {resolution.input}
        </FactRow>
        <FactRow label="Kind">{scanKinds(resolution.kind)}</FactRow>
        {resolution.serial_status !== null && resolution.serial_status !== undefined ? (
          <FactRow label="Status">
            {String(resolution.serial_status).replace(/_/g, " ")}
          </FactRow>
        ) : null}
      </Facts>
      <PrimaryButton
        label="Open"
        onPress={() => {
          const route = singleRoute(resolution);
          onClear();
          if (route !== null) router.push(route as never);
        }}
      />
      <SecondaryButton label="Scan something else" onPress={onClear} />
    </Sheet>
  );
}

/** The ambiguity picker: the best match (the resolver's first row) is
 * preselected and the primary names the choice, so it cannot be tapped
 * blind — the pack's green-chosen card, reversible until Continue. */
function AmbiguousSheet({
  resolution,
  candidates,
  onClear,
}: {
  resolution: ScanResolution;
  candidates: { entity_id: string; display_name: string; secondary?: string | null }[];
  onClear: () => void;
}) {
  const router = useRouter();
  const [choice, setChoice] = useState(candidates[0]?.entity_id ?? "");
  const chosen =
    candidates.find((candidate) => candidate.entity_id === choice) ?? candidates[0];
  return (
    <Sheet
      visible
      onClose={onClear}
      title="Pick the exact one"
      subtitle={`${candidates.length} records match this code — a scan never guesses.${chosen !== undefined && resolution.display_name === "" ? ` ${chosen.display_name} is the likeliest match.` : ""}`}
      testID="scan-pick-sheet"
    >
      {candidates.map((candidate) => (
        <ChoiceCard
          key={candidate.entity_id}
          title={candidate.display_name}
          detail={candidate.secondary ?? undefined}
          chosen={candidate.entity_id === chosen?.entity_id}
          onPress={() => setChoice(candidate.entity_id)}
          testID={`scan-candidate-${candidate.entity_id}`}
        />
      ))}
      {chosen !== undefined ? (
        <PrimaryButton
          label={`Continue with ${chosen.display_name}`}
          testID="scan-pick-continue"
          onPress={() => {
            onClear();
            router.push(candidateRoute(resolution, chosen.entity_id) as never);
          }}
        />
      ) : null}
      <SecondaryButton label="Search by name" onPress={onClear} />
    </Sheet>
  );
}

function scanKinds(kind: string): string {  switch (kind) {
    case "stock_item":
      return "Stock item";
    case "serialized_unit":
      return "Serialized unit";
    case "stock_location":
      return "Location";
    case "device_asset":
      return "Device asset";
    default:
      return "Result";
  }
}

function singleRoute(resolution: { kind: string; entity_id?: string | null }): string | null {
  if (resolution.entity_id === null || resolution.entity_id === undefined) return null;
  return candidateRoute(resolution, resolution.entity_id);
}

function candidateRoute(resolution: { kind: string }, id: string): string {
  const prefix =
    resolution.kind === "stock_item"
      ? "stock-item"
      : resolution.kind === "serialized_unit"
        ? "stock-serial"
        : resolution.kind === "stock_location"
          ? "stock-location"
          : "asset";
  return `/${prefix}/${id}`;
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
  },
  topRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 16,
  },
  ctxChip: {
    flex: 1,
    minHeight: 36,
    borderRadius: 999,
    paddingHorizontal: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  ctxText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  disc: {
    width: 44,
    height: 44,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: chrome.navDark,
  },
  lens: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  reticle: {
    width: 236,
    height: 236,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "flex-end",
    paddingBottom: 14,
  },
  tl: { top: -2, left: -2, borderTopWidth: 3, borderLeftWidth: 3, borderTopLeftRadius: 18 },
  tr: { top: -2, right: -2, borderTopWidth: 3, borderRightWidth: 3, borderTopRightRadius: 18 },
  bl: { bottom: -2, left: -2, borderBottomWidth: 3, borderLeftWidth: 3, borderBottomLeftRadius: 18 },
  br: { bottom: -2, right: -2, borderBottomWidth: 3, borderRightWidth: 3, borderBottomRightRadius: 18 },
  lensNote: {
    fontFamily: type.sans,
    fontSize: 13,
    textAlign: "center",
    paddingHorizontal: 12,
  },
  simulatorPanel: {
    paddingHorizontal: 16,
    paddingBottom: 24,
    gap: 12,
  },
  offlineBanner: {
    borderRadius: 14,
    paddingVertical: 10,
    paddingHorizontal: 14,
  },
  offlineText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
    lineHeight: 18,
  },
  savedRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
    borderRadius: 12,
    borderWidth: 1,
    paddingHorizontal: 12,
  },
  savedText: {
    fontFamily: type.sansMedium,
    fontSize: 14,
  },
  panelLabel: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  simRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  sampleChip: {
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
  typeField: {
    height: 52,
    borderRadius: 14,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 14,
  },
  typeInput: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 15,
  },
  typeGo: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
  },
  typeButton: {
    height: 52,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: chrome.camLine,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  typeButtonText: {
    fontFamily: type.sansSemiBold,
    fontSize: 16,
  },
  sheetBody: {
    fontFamily: type.sans,
    fontSize: 14,
    lineHeight: 21,
  },
  unknownMark: {
    width: 56,
    height: 68,
    alignSelf: "center",
  },
});

function Corner({ style }: { style: object }) {
  return <View pointerEvents="none" style={[cornerBase, style, { borderColor: chrome.gold }]} />;
}

const cornerBase: object = {
  position: "absolute" as const,
  width: 28,
  height: 28,
};
