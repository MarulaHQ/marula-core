// Native wrap-up — the pack's `08-wrap-up-resolved` .. `12-wrap-up-sync` on a
// live ticket, which is the surface this lane exists to finish.
//
// Purpose: the moment a tech closes a job at the door. The pack draws it as
// on-site and travel time cards, the drafted notes, an Outcome segmented
// control with the green tint on the selected segment, a dashed signature
// pad, and a "Resolve and sync" primary whose label follows the outcome. This
// file is that over the live ticket: the outcome control moves the board
// status through a queued write, the pad captures the sign-off, and the hint
// under the primary follows the queue — offline it names the queued count,
// online it says the change syncs straight away.
//
// Non-purpose: no time-entry editing (the Time tab owns that), no board
// mechanics, and no optimistic render of either write. A signature and a
// status move are both things a tech relied on, so neither appears to have
// happened until the server says so.

import { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import { REASON_COPY } from "@/core/errors";
import {
  MODELS,
  fetchBoardStatuses,
  fetchSignOffConsent,
  fetchTicketSignOffs,
  type BoardStatus,
} from "@/core/models";
import { captureSignOff, changeTicketStatus } from "@/core/writes";
import { useModel, useMutation, useQueue } from "@/data/sync-context";
import { SignaturePad, type CapturePayload } from "@/signature/signature-pad";
import { PackLabel, PrimaryButton, SecondaryButton, Sheet, actionReserve, pack } from "@/ui/pack";
import { useTheme } from "@/ui/tokens";

/** The pack's three wrap-up outcomes, each mapped to a board status category. */
export type WrapOutcome = "resolve" | "follow_up" | "waiting";

interface OutcomeOption {
  value: WrapOutcome;
  label: string;
  category: BoardStatus["category"];
  /** The pack's primary label for this outcome. */
  primary: string;
}

const OUTCOMES: readonly OutcomeOption[] = [
  { value: "resolve", label: "Resolved", category: "closed", primary: "Resolve and sync" },
  { value: "follow_up", label: "Follow-up", category: "waiting", primary: "Request a follow-up" },
  { value: "waiting", label: "Waiting", category: "waiting", primary: "Put it on hold and sync" },
];

export interface WrapUpSheetProps {
  ticketId: string;
  ticketNumber?: string;
  boardId: string;
  /** The status the ticket holds now, so the control preselects the outcome
   *  that already matches rather than always opening on Resolved. */
  currentStatusId?: string | null;
  onSiteLabel?: string;
  travelLabel?: string;
  visible: boolean;
  onClose: () => void;
}

export function WrapUpSheet({
  ticketId,
  ticketNumber,
  boardId,
  currentStatusId,
  onSiteLabel,
  travelLabel,
  visible,
  onClose,
}: WrapUpSheetProps) {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const { pendingCount } = useQueue();
  const [outcome, setOutcome] = useState<WrapOutcome>("resolve");
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState<"status" | "signature" | null>(null);
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null);

  const statuses = useModel(
    MODELS.boardStatuses(boardId),
    useCallback(
      (client) => (boardId === "" ? Promise.resolve([]) : fetchBoardStatuses(client, boardId)),
      [boardId],
    ),
    { maxAgeMs: 120_000 },
  );
  const signOffs = useModel(
    MODELS.signOffs(ticketId),
    useCallback((client) => fetchTicketSignOffs(client, ticketId), [ticketId]),
    { maxAgeMs: 30_000 },
  );
  // Read once per session, not per sheet open: the wording is org policy and
  // a wording event invalidates it.
  const consent = useModel(MODELS.signOffConsent, fetchSignOffConsent, { maxAgeMs: 300_000 });
  const consentRow = consent.data;

  const captured = signOffs.data ?? [];

  /** Which statuses each outcome may reach. Empty means the board's workflow
   *  forbids it, and the control says so instead of offering a no-op. The
   *  statuses are read INSIDE the memo so a re-render with the same payload
   *  does not hand it a fresh `[]` and recompute for nothing. */
  const targets = useMemo(() => {
    const rows = statuses.data ?? [];
    const out = new Map<WrapOutcome, BoardStatus[]>();
    for (const option of OUTCOMES) {
      out.set(option.value, rows.filter((row) => row.category === option.category));
    }
    return out;
  }, [statuses.data]);

  const active = OUTCOMES.find((option) => option.value === outcome) ?? OUTCOMES[0]!;
  const target = (targets.get(outcome) ?? [])[0];

  /** The pack: the primary's label follows the outcome, and the hint under it
   *  follows the queue. */
  const syncHint =
    pendingCount === 0
      ? "Syncs straight away."
      : `${pendingCount} change${pendingCount === 1 ? "" : "s"} will sync when you reconnect.`;

  /** `REASON_COPY` is a `Record<string, string>`, so a lookup is typed
   *  `string | undefined`. The key is a literal, so the miss is impossible;
   *  name it once rather than sprinkle non-null assertions. */
  function fallback(key: string): string {
    return REASON_COPY[key] ?? "That did not go through. Try again in a moment.";
  }

  async function applyStatus(): Promise<void> {
    if (target === undefined) return;
    setBusy("status");
    const result = await mutate(changeTicketStatus, {
      ticketId,
      ticketNumber,
      statusId: target.id,
      // `BoardStatus.name` arrives as the generated shape, which types the
      // label field as possibly absent even though the row always has one.
      statusName: target.name ?? undefined,
    });
    setBusy(null);
    if (!result.ok) {
      setNote({ text: result.failure?.text ?? fallback("conflict"), ok: false });
      return;
    }
    setNote({ text: `Moved to ${target.name}.`, ok: true });
  }

  async function applySignOff(payload: CapturePayload): Promise<void> {
    if (consentRow === undefined) {
      setNote({ text: "The consent wording has not loaded yet.", ok: false });
      return;
    }
    setBusy("signature");
    const result = await mutate(captureSignOff, {
      ticketId,
      ticketNumber,
      imageBase64: payload.imageBase64,
      signerName: payload.signerName,
      capturedAt: new Date().toISOString(),
      consentId: consentRow.id,
      consentVersion: consentRow.version,
    });
    setBusy(null);
    if (!result.ok) {
      setNote({ text: result.failure?.text ?? fallback("conflict"), ok: false });
      return;
    }
    setNote({
      text: result.queued
        ? "Signed off. It will sync automatically when you reconnect."
        : "Signed off and recorded.",
      ok: true,
    });
  }

  return (
    <Sheet visible={visible} onClose={onClose} title="Wrap up" testID="wrap-up-sheet">
      <View style={styles.body}>
        <View style={styles.timeRow}>
          {[onSiteLabel ?? "0:00 on site", travelLabel ?? "0:00 travel"].map((label) => (
            <View key={label} style={[styles.timeCard, { borderColor: tokens.border }]}>
              <Text style={[styles.timeLabel, { color: tokens.text }]}>{label}</Text>
              <Text style={[styles.timeMeta, { color: tokens.textSecondary }]}>
                From this ticket&apos;s time
              </Text>
            </View>
          ))}
        </View>

        <View>
          <PackLabel style={{ marginBottom: 8 }}>Notes</PackLabel>
          <TextInput
            value={notes}
            onChangeText={setNotes}
            multiline
            placeholder="What happened, for the record"
            placeholderTextColor={tokens.textMuted}
            accessibilityLabel="Wrap-up notes"
            testID="wrap-up-notes"
            style={[
              styles.notes,
              { borderColor: tokens.border, backgroundColor: tokens.surface, color: tokens.text },
            ]}
          />
        </View>

        <View>
          <PackLabel style={{ marginBottom: 8 }}>Outcome</PackLabel>
          <View style={styles.segment} accessibilityRole="radiogroup" accessibilityLabel="Outcome">
            {OUTCOMES.map((option) => {
              const active = outcome === option.value;
              const available = (targets.get(option.value) ?? []).length > 0;
              return (
                <Pressable
                  key={option.value}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: active, disabled: !available }}
                  disabled={!available}
                  onPress={() => setOutcome(option.value)}
                  testID={`wrap-up-outcome-${option.value}`}
                  style={({ pressed }) => [
                    styles.segmentItem,
                    {
                      borderColor: active ? tokens.border : tokens.border,
                      backgroundColor: active && pressed ? tokens.surfaceHover : "transparent",
                      opacity: available ? 1 : 0.4,
                    },
                  ]}
                >
                  <Text style={[styles.segmentText, { color: active ? tokens.success.text : tokens.text }]}>
                    {option.label}
                  </Text>
                </Pressable>
              );
            })}
          </View>
          {target === undefined ? (
            <Text style={[styles.blocked, { color: tokens.textMuted }]} testID="wrap-up-outcome-blocked">
              This board&apos;s workflow has no status in that category, so this outcome is unavailable.
            </Text>
          ) : null}
        </View>

        <View style={[styles.divider, { borderTopColor: tokens.border }]} />

        <SignaturePad
          onCapture={(payload) => applySignOff(payload)}
          consentText={consentRow?.body ?? "Loading the consent wording for this workspace…"}
          consentVersion={consentRow?.version ?? 0}
          disabled={consentRow === undefined || busy !== null}
          busy={busy === "signature"}
        />

        {captured.length > 0 ? (
          <View style={styles.saved} testID="wrap-up-signed">
            <Text style={[styles.savedText, { color: tokens.success.text }]}>
              {captured.length === 1
                ? `Signed off by ${captured[0]!.signer_name}.`
                : `${captured.length} sign-offs on record.`}
            </Text>
          </View>
        ) : null}

        {note !== null ? (
          <Text
            accessibilityLiveRegion="polite"
            testID="wrap-up-note"
            style={[styles.note, { color: note.ok ? tokens.success.text : tokens.danger.text }]}
          >
            {note.text}
          </Text>
        ) : null}
      </View>

      <View style={[styles.footer, { borderTopColor: tokens.border }]}>
        <SecondaryButton label="Save for later" style={{ flex: 1 }} testID="wrap-up-later" onPress={onClose} />
        <PrimaryButton
          label={busy === "status" ? "Working…" : active.primary}
          icon="sync"
          style={{ flex: 1.6 }}
          testID="wrap-up-confirm"
          busy={busy === "status"}
          disabled={target === undefined || busy !== null}
          onPress={() => {
            void applyStatus();
          }}
        />
      </View>
      <View style={{ height: actionReserve() + pack.radiusSheet }} />
      <Text style={[styles.hint, { color: tokens.textMuted }]} testID="wrap-up-sync-hint">
        {syncHint}
      </Text>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: {
    paddingHorizontal: 16,
    paddingTop: 4,
    gap: 14,
  },
  timeRow: {
    flexDirection: "row",
    gap: 10,
  },
  timeCard: {
    flex: 1,
    borderWidth: 1,
    borderRadius: pack.radiusCard,
    padding: 12,
  },
  timeLabel: {
    fontSize: 17,
    fontWeight: "700",
  },
  timeMeta: {
    fontSize: 10,
    marginTop: 2,
  },
  notes: {
    borderWidth: 1,
    borderRadius: pack.radiusControl,
    minHeight: 72,
    padding: 12,
    fontSize: 14,
    textAlignVertical: "top",
  },
  segment: {
    flexDirection: "row",
  },
  segmentItem: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderRadius: pack.radiusControl,
    marginHorizontal: 3,
  },
  segmentText: {
    fontSize: 13,
    fontWeight: "600",
    paddingVertical: 12,
  },
  blocked: {
    fontSize: 10,
    marginTop: 6,
  },
  divider: {
    borderTopWidth: 1,
  },
  saved: {
    marginTop: 4,
  },
  savedText: {
    fontSize: 10.5,
    fontWeight: "600",
  },
  note: {
    fontSize: 10.5,
    marginTop: 4,
  },
  footer: {
    flexDirection: "row",
    gap: 10,
    paddingHorizontal: 16,
    paddingTop: 12,
    borderTopWidth: 1,
    marginTop: 12,
  },
  hint: {
    fontSize: 10,
    paddingHorizontal: 16,
    paddingTop: 6,
  },
});
