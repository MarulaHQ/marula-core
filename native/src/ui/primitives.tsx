// UI primitives — minimal, token-driven, mobile-native.
//
// Purpose: the small set of controls the foundation screens need, built
// once so every surface inherits the token palette, 44pt touch targets,
// press feedback, and the honest-state patterns (empty, offline, error).
//
// Non-purpose: no design system breadth. Lane 2/3 extend this file rather
// than forking styles.

import React, { createContext, useContext, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type StyleProp,
  type TextInputProps,
  type ViewStyle,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { shape, type, useTheme, type StatusPair } from "./tokens";

/** The tab shell renders notice chrome (offline banner, timer bar) above its
 * screens. When that chrome is visible it clears the status bar itself, so
 * the screens below must not add the top inset a second time. */
export const ShellTopContext = createContext(false);

export function Screen({
  children,
  scroll = true,
  padded = true,
  testID,
}: {
  children: ReactNode;
  scroll?: boolean;
  padded?: boolean;
  testID?: string;
}) {
  const { tokens } = useTheme();
  const insets = useSafeAreaInsets();
  const shellHandlesTop = useContext(ShellTopContext);
  const body = (
    <View
      testID={testID}
      style={[
        styles.screen,
        {
          backgroundColor: tokens.background,
          padding: padded ? shape.screenPadding : 0,
          // Headers are off in both navigators, so every screen owns its own
          // safe area: without the top inset a heading (and a back control)
          // draws under the status bar and the Dynamic Island. The tab shell
          // says when it already cleared that band with its own chrome.
          paddingTop: (shellHandlesTop ? 0 : insets.top) + (padded ? shape.screenPadding : 0),
          paddingBottom: insets.bottom + shape.screenPadding,
        },
      ]}
    >
      {children}
    </View>
  );
  if (!scroll) return body;
  return (
    <KeyboardAvoidingView
      style={{ flex: 1, backgroundColor: tokens.background }}
      behavior={Platform.OS === "ios" ? "padding" : undefined}
      keyboardVerticalOffset={insets.top}
    >
      <ScrollView
        style={{ backgroundColor: tokens.background }}
        contentContainerStyle={{ flexGrow: 1 }}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode={Platform.OS === "ios" ? "interactive" : "on-drag"}
      >
        {body}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

export function Card({
  children,
  style,
  onPress,
  accessibilityLabel,
  testID,
}: {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
  onPress?: () => void;
  accessibilityLabel?: string;
  testID?: string;
}) {
  const { tokens } = useTheme();
  const cardStyle: StyleProp<ViewStyle> = [
    styles.card,
    {
      backgroundColor: tokens.surface,
      borderColor: tokens.border,
    },
    style,
  ];
  if (onPress === undefined) return <View testID={testID} style={cardStyle}>{children}</View>;
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      style={({ pressed }) => [...cardStyle, pressed ? styles.pressed : null]}
    >
      {children}
    </Pressable>
  );
}

export function SectionHeader({
  title,
  action,
}: {
  title: string;
  action?: ReactNode;
}) {
  const { tokens } = useTheme();
  return (
    <View style={styles.sectionHeader}>
      <Text style={[styles.sectionTitle, { color: tokens.textMuted }]}>
        {title.toUpperCase()}
      </Text>
      {action}
    </View>
  );
}

export function Pill({ pair, label }: { pair: StatusPair; label: string }) {
  return (
    <View style={[styles.pill, { backgroundColor: pair.background }]}>
      <Text style={[styles.pillText, { color: pair.text }]}>{label}</Text>
    </View>
  );
}

export type ButtonVariant = "primary" | "secondary" | "destructive" | "ghost";

export function Button({
  label,
  onPress,
  variant = "primary",
  disabled = false,
  loading = false,
  fullWidth = false,
  testID,
}: {
  label: string;
  onPress: () => void;
  variant?: ButtonVariant;
  disabled?: boolean;
  loading?: boolean;
  fullWidth?: boolean;
  testID?: string;
}) {
  const { tokens } = useTheme();
  const palette = {
    primary: { background: tokens.primary, text: tokens.primaryText, border: "transparent" },
    secondary: { background: tokens.surface, text: tokens.text, border: tokens.border },
    destructive: {
      background: tokens.destructive,
      text: tokens.destructiveText,
      border: "transparent",
    },
    ghost: { background: "transparent", text: tokens.primary, border: "transparent" },
  }[variant];
  const blocked = disabled || loading;
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      disabled={blocked}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: blocked, busy: loading }}
      hitSlop={8}
      style={({ pressed }) => [
        styles.button,
        fullWidth ? styles.fullWidth : null,
        {
          backgroundColor: palette.background,
          borderColor: palette.border,
          opacity: blocked ? 0.5 : pressed ? 0.88 : 1,
          transform: [{ scale: pressed && !blocked ? 0.98 : 1 }],
        },
      ]}
    >
      {loading ? (
        <ActivityIndicator color={palette.text} />
      ) : (
        <Text style={[styles.buttonText, { color: palette.text }]}>{label}</Text>
      )}
    </Pressable>
  );
}

export function Field({
  label,
  hint,
  error,
  ...inputProps
}: TextInputProps & { label?: string; hint?: string; error?: string }) {
  const { tokens } = useTheme();
  const [focused, setFocused] = useState(false);
  return (
    <View style={styles.field}>
      {label !== undefined ? (
        <Text style={[styles.fieldLabel, { color: tokens.textSecondary }]}>{label}</Text>
      ) : null}
      <TextInput
        {...inputProps}
        allowFontScaling
        onFocus={(event) => {
          setFocused(true);
          inputProps.onFocus?.(event);
        }}
        onBlur={(event) => {
          setFocused(false);
          inputProps.onBlur?.(event);
        }}
        placeholderTextColor={tokens.textMuted}
        style={[
          styles.input,
          {
            color: tokens.text,
            backgroundColor: tokens.surface,
            borderColor: error !== undefined ? tokens.danger.text : focused ? tokens.primary : tokens.border,
          },
        ]}
      />
      {hint !== undefined && error === undefined ? (
        <Text style={[styles.fieldHint, { color: tokens.textMuted }]}>{hint}</Text>
      ) : null}
      {error !== undefined ? (
        <Text style={[styles.fieldHint, { color: tokens.danger.text }]}>{error}</Text>
      ) : null}
    </View>
  );
}

export function ListRow({
  title,
  subtitle,
  trailing,
  onPress,
  testID,
}: {
  title: string;
  subtitle?: string;
  trailing?: ReactNode;
  onPress?: () => void;
  testID?: string;
}) {
  const { tokens } = useTheme();
  const content = (
    <View style={[styles.row, { borderBottomColor: tokens.border }]}>
      <View style={styles.rowBody}>
        <Text style={[styles.rowTitle, { color: tokens.text }]} numberOfLines={1}>
          {title}
        </Text>
        {subtitle !== undefined ? (
          <Text style={[styles.rowSubtitle, { color: tokens.textMuted }]} numberOfLines={2}>
            {subtitle}
          </Text>
        ) : null}
      </View>
      {trailing}
    </View>
  );
  if (onPress === undefined) return content;
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={subtitle !== undefined ? `${title}, ${subtitle}` : title}
      style={({ pressed }) => (pressed ? styles.pressed : null)}
    >
      {content}
    </Pressable>
  );
}

export function EmptyState({
  message,
  actionLabel,
  onAction,
}: {
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.empty, { borderColor: tokens.border }]}>
      <Text style={[styles.emptyText, { color: tokens.textMuted }]}>{message}</Text>
      {actionLabel !== undefined && onAction !== undefined ? (
        <Button label={actionLabel} onPress={onAction} variant="secondary" />
      ) : null}
    </View>
  );
}

export function ErrorNote({ text, onRetry }: { text: string; onRetry?: () => void }) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.errorNote, { backgroundColor: tokens.danger.background }]}>
      <Text style={[styles.errorText, { color: tokens.danger.text }]}>! {text}</Text>
      {onRetry !== undefined ? (
        <Pressable onPress={onRetry} accessibilityRole="button" hitSlop={8}>
          <Text style={[styles.errorRetry, { color: tokens.danger.text }]}>Retry</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

/** The good-news sibling of ErrorNote: a committed write, an adopted
 * version, a queue that drained. Same shape, success paint — a confirmation
 * must never wear the danger palette. */
export function SuccessNote({ text }: { text: string }) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.errorNote, { backgroundColor: tokens.success.background }]}>
      <Text style={[styles.errorText, { color: tokens.success.text }]}>{text}</Text>
    </View>
  );
}

/** A token-painted switch. React Native's Switch renders its own platform
 * colors (teal on web) that sit outside the Tangerine token set, so the ON
 * state is drawn here with primary + a white knob. */
export function Toggle({
  value,
  onValueChange,
  disabled = false,
  label,
  testID,
}: {
  value: boolean;
  onValueChange: (value: boolean) => void;
  disabled?: boolean;
  label: string;
  testID?: string;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      testID={testID}
      onPress={() => onValueChange(!value)}
      disabled={disabled}
      accessibilityRole="switch"
      accessibilityLabel={label}
      accessibilityState={{ checked: value, disabled }}
      hitSlop={8}
      style={({ pressed }) => [
        styles.toggleTrack,
        {
          backgroundColor: disabled ? tokens.border : value ? tokens.primary : tokens.border,
          opacity: pressed ? 0.9 : 1,
        },
      ]}
    >
      <View
        style={[
          styles.toggleKnob,
          {
            backgroundColor: tokens.surface,
            alignSelf: value ? "flex-end" : "flex-start",
          },
        ]}
      />
    </Pressable>
  );
}

export function Loading({ label = "Loading" }: { label?: string }) {
  const { tokens } = useTheme();
  return (
    <View style={styles.loading}>
      <ActivityIndicator color={tokens.primary} />
      <Text style={[styles.fieldHint, { color: tokens.textMuted }]}>{label}…</Text>
    </View>
  );
}

export function Mono({ children, muted = false }: { children: ReactNode; muted?: boolean }) {
  const { tokens } = useTheme();
  return (
    <Text
      style={[
        styles.mono,
        { color: muted ? tokens.textMuted : tokens.textSecondary },
      ]}
    >
      {children}
    </Text>
  );
}

const styles = StyleSheet.create({
  screen: {
    flexGrow: 1,
    gap: 12,
  },
  card: {
    borderRadius: shape.radiusCard,
    borderWidth: 1,
    padding: shape.cardPadding,
    gap: 8,
  },
  pressed: {
    opacity: 0.9,
  },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingTop: 4,
  },
  sectionTitle: {
    fontFamily: type.sansMedium,
    fontSize: 11,
    letterSpacing: 0.8,
  },
  pill: {
    borderRadius: shape.radiusPill,
    paddingHorizontal: 10,
    paddingVertical: 4,
    alignSelf: "flex-start",
  },
  pillText: {
    fontFamily: type.sansBold,
    fontSize: 11,
  },
  button: {
    minHeight: shape.controlHeight,
    borderRadius: shape.radiusControl,
    borderWidth: 1,
    paddingHorizontal: 16,
    alignItems: "center",
    justifyContent: "center",
  },
  fullWidth: {
    alignSelf: "stretch",
  },
  buttonText: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  field: {
    gap: 6,
  },
  fieldLabel: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  input: {
    minHeight: shape.controlHeight,
    borderRadius: shape.radiusControl,
    borderWidth: 1,
    paddingHorizontal: 12,
    fontFamily: type.sans,
    fontSize: 16,
  },
  fieldHint: {
    fontFamily: type.sans,
    fontSize: 12,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: shape.rowHeight,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  rowBody: {
    flex: 1,
    gap: 2,
  },
  rowTitle: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  rowSubtitle: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  empty: {
    borderWidth: 1,
    borderStyle: "dashed",
    borderRadius: shape.radiusCard,
    padding: 20,
    gap: 12,
    alignItems: "flex-start",
  },
  emptyText: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  errorNote: {
    borderRadius: shape.radiusControl,
    padding: 12,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  errorText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
    flex: 1,
  },
  errorRetry: {
    fontFamily: type.sansSemiBold,
    fontSize: 13,
    textDecorationLine: "underline",
  },
  toggleTrack: {
    width: 52,
    height: 32,
    borderRadius: 999,
    padding: 3,
    justifyContent: "center",
  },
  toggleKnob: {
    width: 26,
    height: 26,
    borderRadius: 13,
  },
  loading: {
    alignItems: "center",
    gap: 8,
    paddingVertical: 24,
  },
  mono: {
    fontFamily: type.mono,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
});
