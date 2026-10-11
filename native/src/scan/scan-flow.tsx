// Scan resolution card + resolver hook — one implementation for the Scan
// tab, the deep-link receiver, and (lane 2) the camera view.
//
// Purpose: a scan NEVER guesses and NEVER auto-submits. The resolver returns
// the typed meaning; ambiguous scans render a picker; unknown scans echo the
// text back and say so; accepted kinds offer an explicit "Open" action
// routed by the frozen vocabulary (marula://stock-item/<uuid>, …).

import { useCallback, useEffect, useState } from "react";
import { Keyboard, Pressable, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";

import type { ScanResolution } from "@/core/models";
import { kindLabel, resolveScan, scanRoute, scanSummary } from "@/core/scan";
import { useAuth } from "@/auth/auth-context";
import { friendlyMutationError } from "@/core/errors";
import { useConnectivity } from "@/data/connectivity";
import { asyncStore } from "@/data/storage";
import { Button, Card, Pill } from "@/ui/primitives";
import { infoPair, shape, type, useTheme } from "@/ui/tokens";

/** The offline saved-scan shelf — the pack's "Saved for later" case. A scan
 * the resolver can't reach (no signal) is saved on the phone and resolved
 * automatically when the connection returns. One JSON list, namespaced. */
const SAVED_SCANS_KEY = "scan.saved";
const savedStore = asyncStore("scans");

interface SavedScan {
  id: string;
  text: string;
  savedAt: string;
}

async function readSaved(): Promise<SavedScan[]> {
  const raw = await savedStore.get(SAVED_SCANS_KEY);
  if (raw === null) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as SavedScan[]) : [];
  } catch {
    return [];
  }
}

async function writeSaved(rows: SavedScan[]): Promise<void> {
  await savedStore.set(SAVED_SCANS_KEY, JSON.stringify(rows));
}

/** The offline resolver: saves the scan and reports the honest outcome. */
export function useSavedScans() {
  const { online } = useConnectivity();
  const [saved, setSaved] = useState<SavedScan[]>([]);
  const [hydrated, setHydrated] = useState(false);

  useEffect(() => {
    void readSaved().then((rows) => {
      setSaved(rows);
      setHydrated(true);
    });
  }, []);

  const save = useCallback(async (text: string) => {
    const rows = await readSaved();
    // One copy per code: a repeated scan of the same label is one intent.
    const next = rows.some((row) => row.text === text)
      ? rows
      : [
          ...rows,
          {
            id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
            text,
            savedAt: new Date().toISOString(),
          },
        ];
    await writeSaved(next);
    setSaved(next);
  }, []);

  const drop = useCallback(async (id: string) => {
    const rows = (await readSaved()).filter((row) => row.id !== id);
    await writeSaved(rows);
    setSaved(rows);
  }, []);

  const clearResolved = useCallback(async (texts: string[]) => {
    const done = new Set(texts);
    const rows = (await readSaved()).filter((row) => !done.has(row.text));
    await writeSaved(rows);
    setSaved(rows);
  }, []);

  return { saved, hydrated, online, save, drop, clearResolved };
}

export function useScanResolver() {
  const { client } = useAuth();
  const { online } = useConnectivity();
  const saved = useSavedScans();
  const [resolution, setResolution] = useState<ScanResolution | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The code the shelf took while offline — its own register, never an
  // error: the pack's scan-10 is a save, not a failure.
  const [savedCode, setSavedCode] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const resolve = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (trimmed.length === 0) return;
      setSavedCode(null);
      // The offline case that must never lose a scan: save it, say so. The
      // shelf drains itself the moment the connection returns.
      if (!online) {
        await saved.save(trimmed);
        setSavedCode(trimmed);
        setError(null);
        setResolution(null);
        return;
      }
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
    [client, online, saved],
  );

  // Connectivity returned: drain the shelf, newest intent last, and surface
  // the last resolution like a fresh scan.
  const { hydrated, saved: shelf, clearResolved } = saved;
  useEffect(() => {
    if (!online || !hydrated || shelf.length === 0) return;
    let cancelled = false;
    const drain = async () => {
      const rows = [...shelf];
      const resolved: string[] = [];
      let last: ScanResolution | null = null;
      for (const row of rows) {
        if (cancelled) return;
        try {
          last = await resolveScan(client, row.text);
          resolved.push(row.text);
        } catch {
          break;
        }
      }
      if (cancelled) return;
      if (resolved.length > 0) {
        await clearResolved(resolved);
        if (last !== null) {
          setResolution(last);
          setError(null);
        }
      }
    };
    void drain();
    return () => {
      cancelled = true;
    };
  }, [online, hydrated, shelf, clearResolved, client]);

  // One reset, all three registers: a sheet dismissed behind you never
  // leaves the last code's state on screen.
  const clear = useCallback(() => {
    setResolution(null);
    setError(null);
    setSavedCode(null);
  }, []);

  return { resolution, error, savedCode, busy, resolve, clear, saved };
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
