// Tool sheets — the scan pack's check-out / check-in sheets (`scan-08`,
// `scan-09`), shared by the tools, kit, and serial surfaces.
//
// Purpose: one sheet grammar for the two tools transactions:
//  - Check-out: wrench tile, name · TL code · from, a green Available pill,
//    a small DUE BACK label over the three-way segmented control (Today
//    17:00 chosen, Tomorrow, +2 days), facts (Checked out to, For visit),
//    the primary Check out, and the nudge caption.
//  - Check-in (overdue loan): the red-tinted pill with an alert icon — the
//    only red, with words — facts (Out to, Since, Was due), the primary
//    Check in here, and Call + Open loan secondaries.
//
// Non-purpose: no ledger writes here; the sheets call back with the chosen
// due date so the owning surface runs the queued write.

import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { type, useTheme } from "@/ui/tokens";
import {
  FactRow,
  Facts,
  PACK_ICONS,
  PackLabel,
  PackPill,
  PrimaryButton,
  SecondaryButton,
  Segment,
  Sheet,
} from "@/ui/pack";

export type DueBackChoice = "today" | "tomorrow" | "plus2";

const DUE_OPTIONS: { value: DueBackChoice; label: string }[] = [
  { value: "today", label: "Today 17:00" },
  { value: "tomorrow", label: "Tomorrow" },
  { value: "plus2", label: "Fri 2 Oct" },
];

export function dueBackDate(choice: DueBackChoice): string {
  const base = new Date();
  if (choice === "tomorrow") base.setDate(base.getDate() + 1);
  if (choice === "plus2") base.setDate(base.getDate() + 2);
  base.setHours(17, 0, 0, 0);
  return base.toISOString();
}

export function ToolCheckoutSheet({
  visible,
  onClose,
  toolName,
  toolCode,
  from,
  holderName,
  visitLabel,
  onCheckout,
}: {
  visible: boolean;
  onClose: () => void;
  toolName: string;
  toolCode: string;
  from: string;
  holderName: string;
  visitLabel?: string | null;
  onCheckout: (dueChoice: DueBackChoice) => void;
}) {
  const { tokens } = useTheme();
  const [due, setDue] = useState<DueBackChoice>("today");
  return (
    <Sheet
      visible={visible}
      onClose={onClose}
      title={toolName}
      subtitle={toolCode ? `${toolCode} · ${from}` : from}
      testID="tool-checkout-sheet"
    >
      <View style={styles.tileRow}>
        <View style={[styles.tile, { backgroundColor: tokens.surfaceHover }]}>
          <Ionicons name="construct-outline" size={24} color={tokens.text} />
        </View>
        <PackPill pair={tokens.success} label="Available" />
      </View>
      <View>
        <PackLabel style={{ marginBottom: 8 }}>Due back</PackLabel>
        <Segment options={DUE_OPTIONS} value={due} onChange={setDue} testIDPrefix="due-back" />
      </View>
      <Facts>
        <FactRow first label="Checked out to">{holderName}</FactRow>
        {visitLabel != null ? <FactRow label="For visit">{visitLabel}</FactRow> : null}
      </Facts>
      <PrimaryButton label="Check out" onPress={() => onCheckout(due)} testID="tool-checkout-commit" />
      <Text style={[styles.caption, { color: tokens.textSecondary }]}>
        A nudge comes at 16:30 if it is still with you.
      </Text>
    </Sheet>
  );
}

export function ToolCheckinSheet({
  visible,
  onClose,
  toolName,
  toolCode,
  overdueDays,
  holderName,
  sinceLabel,
  wasDueLabel,
  onCheckin,
  onCall,
  onOpenLoan,
}: {
  visible: boolean;
  onClose: () => void;
  toolName: string;
  toolCode: string;
  overdueDays: number | null;
  holderName: string;
  sinceLabel: string;
  wasDueLabel: string;
  onCheckin: () => void;
  onCall?: () => void;
  onOpenLoan?: () => void;
}) {
  const { tokens } = useTheme();
  const overdue = overdueDays !== null && overdueDays > 0;
  return (
    <Sheet
      visible={visible}
      onClose={onClose}
      title={toolName}
      subtitle={toolCode}
      testID="tool-checkin-sheet"
    >
      <View style={styles.tileRow}>
        <View style={[styles.tile, { backgroundColor: tokens.surfaceHover }]}>
          <Ionicons name={PACK_ICONS.tools} size={24} color={tokens.text} />
        </View>
        {overdue ? (
          <View style={[styles.overduePill, { backgroundColor: tokens.danger.background }]}>
            <Ionicons name={PACK_ICONS.alert} size={15} color={tokens.danger.text} />
            <Text style={[styles.overdueText, { color: tokens.danger.text }]}>
              {`Overdue ${overdueDays} day${overdueDays === 1 ? "" : "s"}`}
            </Text>
          </View>
        ) : (
          <PackPill pair={tokens.warning} label="Out on loan" />
        )}
      </View>
      <Text style={[styles.caption, { color: tokens.textSecondary }]}>
        Due soon is blush; overdue is the only red, and it says so in words.
      </Text>
      <Facts>
        <FactRow first label="Out to">{holderName}</FactRow>
        <FactRow label="Since">{sinceLabel}</FactRow>
        <FactRow label="Was due">{wasDueLabel}</FactRow>
      </Facts>
      <PrimaryButton label="Check in here" onPress={onCheckin} testID="tool-checkin-commit" />
      <View style={styles.secondaryRow}>
        {onCall !== undefined ? (
          <SecondaryButton style={{ flex: 1 }} label={`Call ${holderName}`} onPress={onCall} />
        ) : null}
        {onOpenLoan !== undefined ? (
          <SecondaryButton style={{ flex: 1 }} label="Open loan" onPress={onOpenLoan} />
        ) : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  tileRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  tile: {
    width: 48,
    height: 48,
    borderRadius: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  overduePill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    height: 26,
    paddingHorizontal: 10,
    borderRadius: 999,
  },
  overdueText: {
    fontFamily: type.sansSemiBold,
    fontSize: 12,
  },
  caption: {
    fontFamily: type.sans,
    fontSize: 12,
    lineHeight: 17,
  },
  secondaryRow: {
    flexDirection: "row",
    gap: 10,
  },
});
