// Marula mobile pack kit — the shared vocabulary the surface pack draws.
//
// Purpose: one implementation of the pieces every mobile screen in the pack
// repeats: status/priority pills, the facts list, the segmented control, the
// radio choice card, bottom sheets with a grab handle, the small-caps label,
// and the severity rules (red means breach, never a form error). Geometry
// comes from the pack's own CSS (source/mobile-core.html,
// source/mobile-scan.html): primary 52px, secondary 44px, text never below
// 12px. Colours come from the landed token file (native/src/ui/tokens.ts),
// never from the pack's raw hexes — the two were verified to match.
//
// Non-purpose: no screens. Screens compose these; nothing here routes.

import React, { useState, type ReactNode } from "react";
import {
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  type StyleProp,
  type TextStyle,
  type ViewStyle,
} from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { chrome, shape, type, useTheme, type StatusPair, type ThemeTokens } from "./tokens";
import { bottomReserve, pack } from "./pack-rules";

type ScrollTextStart = TextStyle;

/** Scrim over a sheet: the pack's rgba(20,19,25,.55) on light, black on dark. */
function scrimFor(mode: "light" | "dark"): string {
  return mode === "dark" ? "rgba(0,0,0,0.6)" : "rgba(20,19,25,0.55)";
}

export { pack, bottomReserve, roundUpTo, shortClock, clockText } from "./pack-rules";

// ---------------------------------------------------------------------------
// Severity rules (pack contract: red means breach; blush is attention)
// ---------------------------------------------------------------------------

export type SlaVerdict = "breach" | "due";

export function slaPair(tokens: ThemeTokens, verdict: SlaVerdict): StatusPair {
  return verdict === "breach" ? tokens.danger : tokens.warning;
}

export function priorityPair(tokens: ThemeTokens, priority: string): StatusPair {
  const p = priority.toLowerCase();
  if (p === "p1" || p === "1" || p === "urgent") return tokens.danger;
  if (p === "p2" || p === "2" || p === "high") return tokens.warning;
  return tokens.neutral;
}

function statusPairFor(tokens: ThemeTokens, category: string): StatusPair {
  switch (category.toLowerCase()) {
    case "resolved":
    case "closed":
      return tokens.success;
    case "in_progress":
      return tokens.warning;
    case "waiting":
    case "waiting_on_client":
      return tokens.neutral;
    default:
      return tokens.neutral;
  }
}
export { statusPairFor };

export function statusLabel(category: string): string {
  const key = category.toLowerCase();
  const known: Record<string, string> = {
    open: "Open",
    new: "New",
    in_progress: "In progress",
    waiting: "Waiting",
    waiting_on_client: "Waiting on client",
    waiting_on_us: "Waiting on us",
    scheduled: "Scheduled",
    on_hold: "On hold",
    resolved: "Resolved",
    closed: "Closed",
  };
  const label = known[key];
  if (label !== undefined) return label;
  const words = category.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

// ---------------------------------------------------------------------------
// Pill
// ---------------------------------------------------------------------------

export function PackPill({
  pair,
  label,
  icon,
  dot,
}: {
  pair: StatusPair;
  label: string;
  icon?: keyof typeof Ionicons.glyphMap;
  dot?: boolean;
}) {
  return (
    <View style={[styles.pill, { backgroundColor: pair.background }]}>
      {icon !== undefined ? <Ionicons name={icon} size={13} color={pair.text} /> : null}
      {dot === true ? (
        <View style={[styles.pillDot, { backgroundColor: pair.text }]} />
      ) : null}
      <Text style={[styles.pillText, { color: pair.text }]}>{label}</Text>
    </View>
  );
}

/** The `.pri` priority chip: P1 red, P2 amber, P3 muted, P4 outline. */
export function PriorityChip({ label }: { label: string }) {
  const { tokens } = useTheme();
  const p = label.toLowerCase();
  if (p === "p1") {
    return (
      <View style={[styles.pri, { backgroundColor: tokens.danger.text }]}>
        <Text style={[styles.priText, { color: tokens.danger.background }]}>{label.toUpperCase()}</Text>
      </View>
    );
  }
  if (p === "p2") {
    return (
      <View style={[styles.pri, { backgroundColor: tokens.warning.text }]}>
        <Text style={[styles.priText, { color: chrome.ink }]}>{label.toUpperCase()}</Text>
      </View>
    );
  }
  if (p === "p4") {
    return (
      <View style={[styles.pri, { backgroundColor: "transparent", borderWidth: 1, borderColor: tokens.border }]}>
        <Text style={[styles.priText, { color: tokens.textSecondary }]}>{label.toUpperCase()}</Text>
      </View>
    );
  }
  return (
    <View style={[styles.pri, { backgroundColor: tokens.neutral.background }]}>
      <Text style={[styles.priText, { color: tokens.text }]}>{label.toUpperCase()}</Text>
    </View>
  );
}

/** The board row's status pill by category, matching the pack locker. */
export function StatusBar({ category }: { category: string | null | undefined }) {
  const { tokens } = useTheme();
  if (category === null || category === undefined) return null;
  return (
    <PackPill pair={statusPairFor(tokens, category)} label={statusLabel(category)} />
  );
}

// ---------------------------------------------------------------------------
// Greeting / headings
// ---------------------------------------------------------------------------

export function PackHeading({ title, detail }: { title: string; detail?: string }) {
  const { tokens } = useTheme();
  return (
    <View style={styles.heading}>
      <Text style={[styles.h1, { color: tokens.text }]}>{title}</Text>
      {detail !== undefined ? (
        <Text style={[styles.h1Detail, { color: tokens.textSecondary }]}>{detail}</Text>
      ) : null}
    </View>
  );
}

/** Small-caps section label (`.lab`). */
export function PackLabel({ children, style }: { children: ReactNode; style?: StyleProp<TextStyleObject> }) {
  const { tokens } = useTheme();
  return <Text style={[styles.lab, { color: tokens.textSecondary }, style]}>{children}</Text>;
}

type TextStyleObject = ScrollTextStart;

// ---------------------------------------------------------------------------
// Buttons: `.mbtn` (52) and `.sbtn` (44)
// ---------------------------------------------------------------------------

export function PrimaryButton({
  label,
  onPress,
  disabled = false,
  busy = false,
  icon,
  style,
  testID,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  busy?: boolean;
  icon?: keyof typeof Ionicons.glyphMap;
  style?: StyleProp<ViewStyle>;
  testID?: string;
}) {
  const { tokens } = useTheme();
  const blocked = disabled || busy;
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      disabled={blocked}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: blocked, busy }}
      style={({ pressed }) => [
        styles.primaryButton,
        { backgroundColor: tokens.primary, opacity: blocked ? 0.4 : pressed ? 0.9 : 1 },
        style,
      ]}
    >
      {icon !== undefined ? <Ionicons name={icon} size={18} color={tokens.primaryText} /> : null}
      <Text style={[styles.primaryButtonText, { color: tokens.primaryText }]}>
        {busy ? "Working…" : label}
      </Text>
    </Pressable>
  );
}

export function SecondaryButton({
  label,
  onPress,
  disabled = false,
  icon,
  style,
  testID,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  icon?: keyof typeof Ionicons.glyphMap;
  style?: StyleProp<ViewStyle>;
  testID?: string;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      style={({ pressed }) => [
        styles.secondaryButton,
        { borderColor: tokens.border, opacity: disabled ? 0.4 : pressed ? 0.85 : 1 },
        style,
      ]}
    >
      {icon !== undefined ? <Ionicons name={icon} size={17} color={tokens.text} /> : null}
      <Text style={[styles.secondaryButtonText, { color: tokens.text }]} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

export function LinkButton({
  label,
  onPress,
  icon,
  testID,
}: {
  label: string;
  onPress: () => void;
  icon?: keyof typeof Ionicons.glyphMap;
  testID?: string;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      hitSlop={6}
      style={styles.linkButton}
    >
      {icon !== undefined ? <Ionicons name={icon} size={15} color={tokens.text} /> : null}
      <Text style={[styles.linkButtonText, { color: tokens.text }]}>{label}</Text>
    </Pressable>
  );
}

// ---------------------------------------------------------------------------
// Facts list (`.facts`): label left, value right, hairline rows
// ---------------------------------------------------------------------------

export function FactRow({
  label,
  children,
  trailing,
  first = false,
  minHeight = pack.factsRowHeight,
}: {
  label?: string;
  children?: ReactNode;
  trailing?: ReactNode;
  first?: boolean;
  minHeight?: number;
}) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.factRow, !first ? { borderTopColor: tokens.border, borderTopWidth: 1 } : null, { minHeight }]}>
      {label !== undefined ? (
        <Text style={[styles.factLabel, { color: tokens.textSecondary }]}>{label}</Text>
      ) : null}
      {children !== undefined ? (
        typeof children === "string" ? (
          <Text style={[styles.factValue, { color: tokens.text }]}>{children}</Text>
        ) : (
          children
        )
      ) : null}
      {trailing}
    </View>
  );
}

export function Facts({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.facts, { borderColor: tokens.border, borderRadius: pack.radiusSheet - 10 }, style]}>
      {children}
    </View>
  );
}

/** A value cell inside a FactRow that stacks a bold line over a small line. */
export function FactValue({ title, detail }: { title: string; detail?: string }) {
  const { tokens } = useTheme();
  return (
    <View style={styles.factStack}>
      <Text style={[styles.factStackTitle, { color: tokens.text }]}>{title}</Text>
      {detail !== undefined ? (
        <Text style={[styles.factStackDetail, { color: tokens.textSecondary }]}>{detail}</Text>
      ) : null}
    </View>
  );
}

// ---------------------------------------------------------------------------
// Segmented control (`.mseg`): radiogroup look, chosen lands in green
// ---------------------------------------------------------------------------

export function Segment<T extends string | number>({
  options,
  value,
  onChange,
  size = "mix",
  testIDPrefix,
}: {
  options: { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  /** `mix` = 44px (pack's pickers), `small` = 36px (the More appearance pair). */
  size?: "mix" | "small";
  testIDPrefix?: string;
}) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.segment, { backgroundColor: tokens.surfaceHover }]}>
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <Pressable
            key={String(option.value)}
            testID={testIDPrefix !== undefined ? `${testIDPrefix}-${option.value}` : undefined}
            onPress={() => onChange(option.value)}
            accessibilityRole="radio"
            accessibilityState={{ selected }}
            style={({ pressed }) => [
              styles.segmentOption,
              size === "small" ? styles.segmentOptionSmall : styles.segmentOptionMix,
              {
                backgroundColor: selected ? tokens.success.background : "transparent",
                opacity: pressed && !selected ? 0.8 : 1,
              },
            ]}
          >
            <Text
              style={[
                styles.segmentText,
                size === "small" ? styles.segmentTextSmall : null,
                { color: selected ? tokens.success.text : tokens.textSecondary },
              ]}
              numberOfLines={1}
            >
              {option.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

// ---------------------------------------------------------------------------
// Radio choice card (`.opt`): green border + tint + filled check when chosen
// ---------------------------------------------------------------------------

export function ChoiceCard({
  title,
  detail,
  icon,
  chosen,
  onPress,
  trailing,
  testID,
}: {
  title: string;
  detail?: string;
  icon?: keyof typeof Ionicons.glyphMap;
  chosen: boolean;
  onPress: () => void;
  trailing?: ReactNode;
  testID?: string;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected: chosen }}
      style={({ pressed }) => [
        styles.choice,
        {
          borderColor: chosen ? tokens.success.text : tokens.border,
          borderWidth: chosen ? 2 : 1,
          backgroundColor: chosen ? tokens.success.background : tokens.surface,
          opacity: pressed && !chosen ? 0.9 : 1,
        },
      ]}
    >
      {icon !== undefined ? (
        <View style={[styles.choiceIcon, { backgroundColor: tokens.surfaceHover }]}>
          <Ionicons name={icon} size={22} color={tokens.text} />
        </View>
      ) : null}
      <View style={styles.choiceBody}>
        <Text style={[styles.choiceTitle, { color: tokens.text }]} numberOfLines={1}>
          {title}
        </Text>
        {detail !== undefined ? (
          <Text style={[styles.choiceDetail, { color: tokens.textSecondary }]} numberOfLines={1}>
            {detail}
          </Text>
        ) : null}
      </View>
      {trailing}
      <View
        style={[
          styles.choiceCheck,
          {
            borderColor: chosen ? tokens.success.text : tokens.border,
            backgroundColor: chosen ? tokens.success.text : "transparent",
          },
        ]}
      >
        {chosen ? <Ionicons name="checkmark" size={15} color={tokens.surface} /> : null}
      </View>
    </Pressable>
  );
}

// ---------------------------------------------------------------------------
// Sheet: scrim + bottom panel, grab handle, 24px top corners
// ---------------------------------------------------------------------------

export function Sheet({
  visible,
  onClose,
  title,
  subtitle,
  children,
  tint,
  testID,
}: {
  visible: boolean;
  onClose: () => void;
  title?: string;
  subtitle?: string;
  children: ReactNode;
  /** The note composer's private tint — the sheet takes the note surface. */
  tint?: "note";
  testID?: string;
}) {
  const { tokens } = useTheme();
  const note = tint === "note";
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
        <KeyboardAvoidingView
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          pointerEvents="box-none"
          style={styles.sheetRoot}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Close"
            onPress={onClose}
            style={[styles.scrim, { backgroundColor: scrimFor(tokens.mode) }]}
          />
          <View
            testID={testID}
            style={[
              styles.sheet,
              {
                backgroundColor: note
                  ? tokens.neutral.background
                  : tokens.surface,
                borderTopColor: tokens.border,
              },
            ]}
          >
            <View style={[styles.grab, { backgroundColor: tokens.border }]} />
            {title !== undefined ? (
              <Text style={[styles.sheetTitle, { color: tokens.text }]}>{title}</Text>
            ) : null}
            {subtitle !== undefined ? (
              <Text style={[styles.sheetSubtitle, { color: tokens.textSecondary }]}>{subtitle}</Text>
            ) : null}
            {children}
          </View>
        </KeyboardAvoidingView>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Header: back disc 44 + title + trailing (`.hd`, `.hd2`)
// ---------------------------------------------------------------------------

export function PackHeader({
  title,
  detail,
  onBack,
  backTestID = "pack-header-back",
  trailing,
  center = false,
}: {
  title: ReactNode;
  detail?: string;
  onBack?: () => void;
  backTestID?: string;
  trailing?: ReactNode;
  center?: boolean;
}) {
  const { tokens } = useTheme();
  return (
    <View style={styles.headerRow}>
      {onBack !== undefined ? (
        <Pressable
          testID={backTestID}
          onPress={onBack}
          hitSlop={8}
          accessibilityRole="button"
          accessibilityLabel="Back"
          style={({ pressed }) => [
            styles.backDisc,
            { backgroundColor: pressed ? tokens.surfaceHover : "transparent" },
          ]}
        >
          <Ionicons name="chevron-back" size={22} color={tokens.text} />
        </Pressable>
      ) : null}
      <View style={styles.headerBody}>
        {typeof title === "string" ? (
          <Text style={[center ? styles.headerTitleCenter : styles.headerTitle, { color: tokens.text }]} numberOfLines={1}>
            {title}
          </Text>
        ) : (
          title
        )}
        {detail !== undefined ? (
          <Text style={[styles.headerDetail, { color: tokens.textSecondary }]}>{detail}</Text>
        ) : null}
      </View>
      {trailing}
    </View>
  );
}

// ---------------------------------------------------------------------------
// Withhold-safe icons: single icon name mapping for pack drawings
// ---------------------------------------------------------------------------

export const PACK_ICONS = {
  scan: "scan-outline",
  today: "today-outline",
  board: "grid-outline",
  time: "time-outline",
  more: "ellipsis-horizontal",
  bell: "notifications-outline",
  tools: "construct-outline",
  kit: "file-tray-full-outline",
  asset: "desktop-outline",
  stock: "cube-outline",
  location: "location-outline",
  car: "car-outline",
  users: "people-outline",
  calendar: "calendar-outline",
  check: "checkmark",
  alert: "alert-circle",
  clock: "time-outline",
  lock: "lock-closed",
  mail: "mail-outline",
  phone: "call-outline",
  monitor: "desktop-outline",
  paperclip: "attach-outline",
  shield: "shield-checkmark-outline",
  eye: "eye-outline",
  sliders: "options-outline",
  search: "search-outline",
  plus: "add",
  close: "close",
  back: "chevron-back",
  chev: "chevron-forward",
  info: "information-circle-outline",
  logout: "log-out-outline",
  keyboard: "keypad-outline",
  bolt: "flashlight-outline",
  ext: "open-outline",
  kanban: "swap-horizontal-outline",
  box: "archive-outline",
  pin: "location-outline",
  pause: "pause",
  play: "play",
  stop: "stop",
  settings: "settings-outline",
} as const;

// ---------------------------------------------------------------------------
// Styles — pack geometry only
// ---------------------------------------------------------------------------

const styles = StyleSheet.create({
  pill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    height: 24,
    paddingHorizontal: 10,
    borderRadius: shape.radiusPill,
    alignSelf: "flex-start",
  },
  pillDot: {
    width: 7,
    height: 7,
    borderRadius: 4,
  },
  pillText: {
    fontFamily: type.sansSemiBold,
    fontSize: 12,
  },
  pri: {
    minWidth: 28,
    height: 22,
    paddingHorizontal: 6,
    borderRadius: 6,
    alignItems: "center",
    justifyContent: "center",
  },
  priText: {
    fontFamily: type.sansBold,
    fontSize: 12,
  },
  heading: {
    gap: 2,
  },
  h1: {
    fontFamily: type.sansBold,
    fontSize: 26,
    letterSpacing: -0.4,
  },
  h1Detail: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  lab: {
    fontFamily: type.sansMedium,
    fontSize: 12,
    letterSpacing: 0.8,
    textTransform: "uppercase",
  },
  primaryButton: {
    minHeight: pack.primaryHeight,
    borderRadius: pack.radiusPrimary,
    paddingHorizontal: 16,
    alignItems: "center",
    justifyContent: "center",
    flexDirection: "row",
    gap: 8,
  },
  primaryButtonText: {
    fontFamily: type.sansSemiBold,
    fontSize: 16,
  },
  secondaryButton: {
    minHeight: pack.secondaryHeight,
    borderRadius: pack.radiusControl,
    borderWidth: 1,
    paddingHorizontal: 14,
    alignItems: "center",
    justifyContent: "center",
    flexDirection: "row",
    gap: 8,
    backgroundColor: "transparent",
  },
  secondaryButtonText: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  linkButton: {
    minHeight: pack.secondaryHeight,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
    borderRadius: pack.radiusControl,
  },
  linkButtonText: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
    textDecorationLine: "underline",
  },
  facts: {
    borderWidth: 1,
    overflow: "hidden",
  },
  factRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  factLabel: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  factValue: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
    textAlign: "right",
    flexShrink: 1,
  },
  factStack: {
    flex: 1,
    gap: 1,
  },
  factStackTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  factStackDetail: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  segment: {
    flexDirection: "row",
    gap: 4,
    padding: 4,
    borderRadius: 12,
  },
  segmentOption: {
    flex: 1,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 8,
  },
  segmentOptionMix: {
    minHeight: pack.rowHeight,
  },
  segmentOptionSmall: {
    minHeight: 36,
  },
  segmentText: {
    fontFamily: type.sansMedium,
    fontSize: 14,
  },
  segmentTextSmall: {
    fontSize: 13,
  },
  choice: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    padding: 12,
    borderRadius: pack.radiusSheet - 10,
    minHeight: 72,
  },
  choiceIcon: {
    width: 48,
    height: 48,
    borderRadius: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  choiceBody: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  choiceTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  choiceDetail: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  choiceCheck: {
    width: 24,
    height: 24,
    borderRadius: 12,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
  sheetRoot: {
    flex: 1,
    justifyContent: "flex-end",
  },
  scrim: {
    position: "absolute" as const,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
  },
  sheet: {
    borderTopLeftRadius: pack.radiusSheet,
    borderTopRightRadius: pack.radiusSheet,
    paddingHorizontal: 16,
    paddingTop: 10,
    paddingBottom: 30,
    gap: 14,
    maxHeight: "86%",
  },
  grab: {
    width: 40,
    height: 5,
    borderRadius: shape.radiusPill,
    alignSelf: "center",
    marginBottom: 2,
  },
  sheetTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
  },
  sheetSubtitle: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 18,
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: pack.secondaryHeight,
  },
  headerBody: {
    flex: 1,
    minWidth: 0,
  },
  headerTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
  },
  headerTitleCenter: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
    textAlign: "center",
  },
  headerDetail: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  backDisc: {
    width: pack.secondaryHeight,
    height: pack.secondaryHeight,
    borderRadius: pack.radiusControl,
    alignItems: "center",
    justifyContent: "center",
  },
});

// ---------------------------------------------------------------------------
// Pinned footer container (action bar / camera bottom stack)
// ---------------------------------------------------------------------------

export function PinnedBar({ children, style, testID }: { children: ReactNode; style?: StyleProp<ViewStyle>; testID?: string }) {
  const { tokens, mode } = useTheme();
  return (
    <View
      testID={testID}
      style={[
        {
          position: "absolute" as const,
          left: 0,
          right: 0,
          bottom: 0,
          padding: 12,
          paddingBottom: 30,
          gap: 10,
          borderTopWidth: 1,
          flexDirection: "row",
          backgroundColor: mode === "dark" ? chrome.navDark : tokens.surface,
          borderTopColor: tokens.border,
        },
        style,
      ]}
    >
      {children}
    </View>
  );
}

void StyleSheet.hairlineWidth;

/** Standard tab-strip paddings a tab screen applies under its list. */
export function useTabScreenPadding(): { bottom: number } {
  return { bottom: bottomReserve.tabs };
}

/** Where the pinned action bar sits, safe-area aware callers add insets. */
export function actionReserve(): number {
  return bottomReserve.actions;
}

void ScrollView;
void useState;
