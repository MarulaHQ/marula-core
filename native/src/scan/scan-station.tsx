// Scan station — the one scan input every field surface mounts.
//
// Purpose: camera (on a real device) or typed/pasted injector (everywhere,
// including the simulator and Maestro), resolved through the shared typed
// resolver, routed by the surface's accepted kinds, and either staged for
// the human's confirm or rejected with a sentence. A scan NEVER commits and
// NEVER guesses between candidates.
//
// The station is the mobile grammar of Wave 32 lane 6: scan or search →
// stage → pick context → human commits.

import { useCallback, useState, type ReactNode } from "react";
import { Keyboard, Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { friendlyMutationError } from "@/core/errors";
import type { ScanResolution } from "@/core/models";
import {
  accepts,
  kindLabel,
  pickCandidate,
  rejectionCopy,
  resolveScan,
  scanSummary,
} from "@/core/scan";
import { CameraScanner } from "@/scan/camera-scanner";
import { scanRejected, scanStaged } from "@/ui/haptics";
import { Button, Card, ErrorNote, Field, Mono, Pill } from "@/ui/primitives";
import { infoPair, type, useTheme } from "@/ui/tokens";

export interface ScanStationProps {
  /** Surface key in SURFACE_ACCEPTS — decides which kinds are usable here. */
  surface: string;
  onAccept: (resolution: ScanResolution) => void;
  staged?: ScanResolution | null;
  onClear?: () => void;
  /** What this surface is for, in the operator's words. */
  hint?: string;
  /** Optional secondary action rendered under the input (e.g. "Search"). */
  extra?: ReactNode;
  testIDPrefix?: string;
}

export function ScanStation({
  surface,
  onAccept,
  staged = null,
  onClear,
  hint,
  extra,
  testIDPrefix = "scan-station",
}: ScanStationProps) {
  const { tokens } = useTheme();
  const { client } = useAuth();
  const [text, setText] = useState("");
  const [cameraOpen, setCameraOpen] = useState(false);
  const [candidates, setCandidates] = useState<ScanResolution | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const finish = useCallback(
    (resolution: ScanResolution) => {
      // The keyboard has done its job once a code resolves; leaving it up
      // hides the context fields and the confirm button under it.
      Keyboard.dismiss();
      setCandidates(null);
      setError(null);
      setText("");
      onAccept(resolution);
      scanStaged();
    },
    [onAccept],
  );

  const resolve = useCallback(
    async (raw: string) => {
      const trimmed = raw.trim();
      if (trimmed.length === 0) return;
      setBusy(true);
      setError(null);
      setCandidates(null);
      // Scan-gun behaviour: the field empties the moment a code is
      // submitted, so the next scan lands in an empty field. A transport
      // failure restores the text so nothing the operator typed is lost.
      setText("");
      try {
        const result = await resolveScan(client, trimmed);
        if ((result.candidates ?? []).length > 0) {
          setCandidates(result);
          return;
        }
        if (!accepts(surface, result)) {
          Keyboard.dismiss();
          setError(rejectionCopy(surface, result));
          scanRejected();
          return;
        }
        finish(result);
      } catch (cause) {
        setText(trimmed);
        setError(friendlyMutationError(cause).text);
      } finally {
        setBusy(false);
      }
    },
    [client, finish, surface],
  );

  const pick = useCallback(
    (entityId: string) => {
      if (candidates === null) return;
      const picked = pickCandidate(candidates, entityId);
      if (!accepts(surface, picked)) {
        Keyboard.dismiss();
        setCandidates(null);
        setError(rejectionCopy(surface, picked));
        scanRejected();
        return;
      }
      finish(picked);
    },
    [candidates, finish, surface],
  );

  return (
    <Card testID={`${testIDPrefix}-card`}>
      {staged !== null ? (
        <View
          testID={`${testIDPrefix}-staged`}
          style={[styles.staged, { borderColor: tokens.primary, backgroundColor: tokens.surfaceHover }]}
        >
          <View style={styles.stagedBody}>
            <View style={styles.stagedHeader}>
              <Pill pair={infoPair(tokens)} label={kindLabel(staged.kind).toUpperCase()} />
              {staged.is_tool ? <Pill pair={tokens.neutral} label="TOOL" /> : null}
            </View>
            <Text style={[styles.stagedTitle, { color: tokens.text }]} numberOfLines={2}>
              {staged.display_name}
            </Text>
            <Mono muted>{scanSummary(staged)}</Mono>
          </View>
          {onClear !== undefined ? (
            <Button
              label="Remove"
              variant="ghost"
              testID={`${testIDPrefix}-clear`}
              onPress={onClear}
            />
          ) : null}
        </View>
      ) : null}
      <>
        {hint !== undefined ? (
          <Text style={[styles.hint, { color: tokens.textMuted }]}>{hint}</Text>
        ) : null}
        <View style={styles.scanRow}>
          <Button
            label="Scan"
            testID={`${testIDPrefix}-camera`}
            onPress={() => setCameraOpen(true)}
          />
          <View style={styles.inputWrap}>
            <Field
              testID={`${testIDPrefix}-input`}
              value={text}
              onChangeText={setText}
              placeholder="Or type a code"
              autoCapitalize="none"
              autoCorrect={false}
              returnKeyType="go"
              onSubmitEditing={() => {
                void resolve(text);
              }}
            />
          </View>
        </View>
        <Button
          label={busy ? "Resolving…" : "Resolve code"}
          variant="secondary"
          testID={`${testIDPrefix}-resolve`}
          fullWidth
          loading={busy}
          disabled={text.trim().length === 0 || busy}
          onPress={() => {
            void resolve(text);
          }}
        />
        {candidates !== null ? (
          <View style={styles.candidates} testID={`${testIDPrefix}-candidates`}>
            <Text style={[styles.hint, { color: tokens.textSecondary }]}>
              {(candidates.candidates ?? []).length} records match — pick the exact
              one (a scan never guesses).
            </Text>
            {(candidates.candidates ?? []).map((candidate, index) => (
              <Pressable
                key={candidate.entity_id}
                testID={`${testIDPrefix}-candidate-${index}`}
                onPress={() => pick(candidate.entity_id)}
                accessibilityRole="button"
                style={[styles.candidate, { borderColor: tokens.border }]}
              >
                <Text style={[styles.candidateTitle, { color: tokens.text }]}>
                  {candidate.display_name}
                </Text>
                {candidate.secondary != null && candidate.secondary !== "" ? (
                  <Mono muted>{candidate.secondary}</Mono>
                ) : null}
              </Pressable>
            ))}
          </View>
        ) : null}
        {error !== null ? <ErrorNote text={error} /> : null}
        {extra}
      </>
      <CameraScanner
        visible={cameraOpen}
        onClose={() => setCameraOpen(false)}
        onScan={(scanned) => {
          setCameraOpen(false);
          void resolve(scanned);
        }}
      />
    </Card>
  );
}

const styles = StyleSheet.create({
  hint: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
  scanRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  inputWrap: {
    flex: 1,
  },
  staged: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: 1,
    borderRadius: 12,
    padding: 12,
  },
  stagedBody: {
    flex: 1,
    gap: 4,
  },
  stagedHeader: {
    flexDirection: "row",
    gap: 6,
  },
  stagedTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 16,
  },
  candidates: {
    gap: 8,
  },
  candidate: {
    borderWidth: 1,
    borderRadius: 12,
    padding: 12,
    minHeight: 48,
    justifyContent: "center",
    gap: 2,
  },
  candidateTitle: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
});
