// Design tokens for the native app — the ONLY file allowed to declare hexes.
//
// Purpose: carry the docs/theme.md Tangerine palette into React Native as a
// typed semantic map for both modes (light canvas #E8EBED, blue-slate dark
// #1C2433, tangerine interactive #E05D38, PSA status pairs). Typography
// uses the real Inter / JetBrains Mono faces loaded in app/_layout.tsx.
// Motion stays within the theme law: transform/opacity only, <=120ms.
//
// Non-purpose: no component styling and no theme packs (the pack schema is
// a web/backend surface; mobile ships the pinned default until the app needs
// org presets).

import { createContext, useContext, useMemo, type ReactNode } from "react";
import { useColorScheme } from "react-native";
import React from "react";

export interface StatusPair {
  text: string;
  background: string;
}

export interface ThemeTokens {
  mode: "light" | "dark";
  background: string;
  surface: string;
  surfaceHover: string;
  overlay: string;
  text: string;
  textSecondary: string;
  textMuted: string;
  border: string;
  primary: string;
  primaryText: string;
  destructive: string;
  destructiveText: string;
  success: StatusPair;
  warning: StatusPair;
  danger: StatusPair;
  neutral: StatusPair;
}

export const lightTokens: ThemeTokens = {
  mode: "light",
  background: "#E8EBED",
  surface: "#FFFFFF",
  surfaceHover: "#F3F4F6",
  overlay: "#FFFFFF",
  text: "#333333",
  textSecondary: "#4B5563",
  textMuted: "#6B7280",
  border: "#DCDFE2",
  primary: "#E05D38",
  primaryText: "#FFFFFF",
  destructive: "#EF4444",
  destructiveText: "#FFFFFF",
  success: { text: "#15803D", background: "#DCFCE7" },
  warning: { text: "#B45309", background: "#FEF3C7" },
  danger: { text: "#B91C1C", background: "#FEE2E2" },
  neutral: { text: "#4B5563", background: "#E5E7EB" },
};

export const darkTokens: ThemeTokens = {
  mode: "dark",
  background: "#1C2433",
  surface: "#2A3040",
  surfaceHover: "#313A4D",
  overlay: "#262B38",
  text: "#E5E5E5",
  textSecondary: "#A3A3A3",
  textMuted: "#A3A3A3",
  border: "#3D4354",
  primary: "#E05D38",
  primaryText: "#FFFFFF",
  destructive: "#EF4444",
  destructiveText: "#FFFFFF",
  success: { text: "#4ADE80", background: "#14532D" },
  warning: { text: "#FBBF24", background: "#78350F" },
  danger: { text: "#F87171", background: "#7F1D1D" },
  // One step above the card surface so the pill shape survives in dark
  // (vision review finding 9).
  neutral: { text: "#B4BAC6", background: "#333B4E" },
};

/** Shape + type scale, identical in both modes (not theme-restylable). */
export const shape = {
  radiusControl: 12,
  radiusCard: 12,
  radiusPill: 999,
  controlHeight: 48,
  rowHeight: 44,
  cardPadding: 16,
  screenPadding: 16,
} as const;

export const type = {
  sans: "Inter_400Regular",
  sansMedium: "Inter_500Medium",
  sansSemiBold: "Inter_600SemiBold",
  sansBold: "Inter_700Bold",
  mono: "JetBrainsMono_400Regular",
  monoMedium: "JetBrainsMono_500Medium",
} as const;


/** Informational pill pair: entity kinds, possession, and context labels.
 * Neutral on purpose — Ember is interactive-only and the status palette is
 * green/amber/red, so a blue family would be a second accent (vision review
 * finding 2, 2026-09-26). Selection is shown with a ring, never a fill. */
export function infoPair(tokens: ThemeTokens): StatusPair {
  return tokens.neutral;
}

export const motion = {
  fast: 100,
  standard: 120,
} as const;

export type ThemeMode = "system" | "light" | "dark";

interface ThemeValue {
  tokens: ThemeTokens;
  mode: ThemeMode;
  setMode(mode: ThemeMode): void;
}

const ThemeContext = createContext<ThemeValue>({
  tokens: lightTokens,
  mode: "system",
  setMode: () => undefined,
});

export function ThemeProvider({
  children,
  mode,
  onModeChange,
}: {
  children: ReactNode;
  mode: ThemeMode;
  onModeChange: (mode: ThemeMode) => void;
}) {
  const system = useColorScheme();
  const value = useMemo<ThemeValue>(() => {
    const resolved = mode === "system" ? (system === "dark" ? "dark" : "light") : mode;
    return {
      tokens: resolved === "dark" ? darkTokens : lightTokens,
      mode,
      setMode: onModeChange,
    };
  }, [mode, system, onModeChange]);
  return React.createElement(ThemeContext.Provider, { value }, children);
}

export function useTheme(): ThemeValue {
  return useContext(ThemeContext);
}
