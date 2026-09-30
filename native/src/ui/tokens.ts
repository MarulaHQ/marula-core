// Design tokens for the native app — the ONLY file allowed to declare hexes.
//
// Purpose: carry the docs/theme.md Marula palette into React Native as a typed
// semantic map for both modes. Typography uses the real Figtree / JetBrains
// Mono faces loaded in app/_layout.tsx. Motion stays within the theme law:
// transform/opacity only, <=120ms.
//
// The mobile default is the CREAM canvas (#FBF7EC, the `marula-warm` column),
// not the Neutral one: the spec picks cream for mobile and Neutral for
// web/desktop. The dark column is shared with the web app — dark is dark
// everywhere in Marula, so the two platforms agree on it.
//
// Tangerine is retired. Nothing here carries its ramp.
//
// Non-purpose: no component styling and no theme packs (the pack schema is
// a web/backend surface; mobile ships the pinned default until the app needs
// org presets).

import { createContext, useContext, useMemo, type ReactNode } from "react";
import { useColorScheme } from "react-native";
import React from "react";

/**
 * The preset id this app's canvas corresponds to, mirroring
 * PLATFORM_PRESET_DEFAULT.mobile in packages/ui/src/theme-tokens.ts. Declared
 * here because the native app cannot import the web package, and pinned by the
 * frontend preset test so the two cannot drift.
 */
export const PLATFORM_PRESET_ID = "marula-warm";

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
  background: "#FBF7EC",
  surface: "#FFFFFF",
  surfaceHover: "#F7F3E8",
  overlay: "#FFFFFF",
  text: "#2B2A33",
  textSecondary: "#5F584F",
  textMuted: "#6B6357",
  border: "#E6DCC2",
  // The primary FLIPS on mode (spec §4.2): ink with gold text in light, gold
  // with ink text in dark. Both clear 4.5:1, so the mobile CTA needs no
  // large-text exemption — the Tangerine pair it replaces did.
  primary: "#2B2A33",
  primaryText: "#E8B83A",
  destructive: "#B42318",
  destructiveText: "#FFFFFF",
  success: { text: "#3B5222", background: "#E4EDCB" },
  warning: { text: "#6B4308", background: "#FBEBBF" },
  danger: { text: "#B42318", background: "#FDE8E5" },
  neutral: { text: "#5C4631", background: "#F3EAD2" },
};

export const darkTokens: ThemeTokens = {
  mode: "dark",
  background: "#2B2A33",
  surface: "#34333D",
  surfaceHover: "#3A3944",
  overlay: "#34333D",
  text: "#F6EFD9",
  textSecondary: "#BEB5A2",
  textMuted: "#B0A894",
  border: "#43424D",
  primary: "#E8B83A",
  primaryText: "#2B2A33",
  destructive: "#FB8A80",
  destructiveText: "#2B2A33",
  success: { text: "#D6E5A8", background: "#2E3A25" },
  warning: { text: "#F2CD6B", background: "#4A3A1A" },
  danger: { text: "#FB8A80", background: "#3B1A18" },
  // One step above the card surface so the pill shape survives in dark
  // (vision review finding 9).
  neutral: { text: "#E2CFA9", background: "#3A342A" },
};

/** Shape + type scale, identical in both modes (not theme-restylable). */
export const shape = {
  radiusControl: 10,
  radiusCard: 16,
  radiusPill: 999,
  // 44px is the mobile minimum tap target in every density (spec §6), so it is
  // not a density value here. The web density ladder is a desktop concern.
  controlHeight: 48,
  rowHeight: 48,
  cardPadding: 24,
  screenPadding: 16,
} as const;

export const type = {
  sans: "Figtree_400Regular",
  sansMedium: "Figtree_500Medium",
  sansSemiBold: "Figtree_600SemiBold",
  sansBold: "Figtree_700Bold",
  mono: "JetBrainsMono_400Regular",
  monoMedium: "JetBrainsMono_500Medium",
  serif: "Fraunces_500Medium",
} as const;

/**
 * The pack's NON-FLIPPING chrome inks (docs/theme.md §5, the mobile pack):
 * the timer pill / tab-bar raised button are ink in every theme, the gold
 * tick and clock ride on them, and the dark tab surface is one step below
 * the background. These are chrome constants, not theme-restylable tokens —
 * named here once so screens never re-declare a hex.
 */
export const chrome = {
  /** Ink surface (light-mode primary); non-flipping on the timer pill. */
  ink: "#2B2A33",
  /** Gold accent (the pack's tabact / clock / leaf mark). */
  gold: "#E8B83A",
  /** Gold readable as TEXT on light surfaces (--m-gold-text). */
  goldText: "#9A5B0F",
  /** The pack's gold TINT ground (--acc): the running card, chosen segments. */
  goldTintLight: "#F7E3A8",
  /** ...and its dark-mode ground (--acc in dark is the raised surface). */
  goldTintDark: "#3A3944",
  /** Cream on ink. */
  cream: "#F6EFD9",
  /** Dark tab-bar surface (--nav in dark). */
  navDark: "#222129",
  /** Dark raised-disc surface (--surf2 in dark; the pill's stop disc). */
  surfaceHoverDark: "#3A3944",
  /** Camera disc still (the scan screen is dark in both themes). */
  camInk: "#1B1A21",
  /** The outlined control border on the camera screen. */
  camLine: "#6B6660",
  /** Avatar leaf green (the pack's profile avatar). */
  leaf: "#4A6A2A",
} as const;


/** Informational pill pair: entity kinds, possession, and context labels.
 * Neutral on purpose. The retired Ember family is gone entirely (spec §4.6),
 * and the status palette is already green/amber/red, so a fourth hue here
 * would be a second accent (vision review finding 2, 2026-09-26). Selection is
 * shown with a ring, never a fill. */
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
