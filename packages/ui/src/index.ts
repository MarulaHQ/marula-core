"use client";
/**
 * @marula-core/ui — Marula design system barrel.
 * Purpose: single public entry point for tokens API, theming, and components.
 * Non-purpose: stylesheets ship separately (`@marula-core/ui/tokens.css` +
 * `components.css`) and must be imported by the host app once at its root.
 */

export { cn } from "./lib/cn";
export * from "./tokens";
export * from "./theme";
export type {
  FixedRole,
  PackTheme,
  TokenRole,
  TokenValues,
  ThemePreset,
} from "./theme-tokens";
export {
  BUILTIN_PRESETS,
  MARULA,
  MARULA_WARM,
  HIGH_CONTRAST,
  DEFAULT_PRESET,
  TOKEN_ROLES,
  FIXED_ROLES,
  SHIFTABLE_AXIS,
  UNSHIFTABLE_ROLES,
  isValidTokenValue,
  isBuiltinPreset,
  presetById,
  presetCss,
  presetViolation,
  assertPresetIsLegal,
  builtinPresetCss,
  THEME_TOKEN_SCHEMA_VERSION,
} from "./theme-tokens";
export {
  applyPackStyles,
  removePackStyles,
  packStyleId,
  setThemePreset,
  resolveInitialPreset,
  PRESET_STORAGE_KEY,
  PACK_STYLE_PREFIX,
  type PackTokens,
} from "./theme-pack-apply";

export * from "./components/button";
export * from "./components/input";
export * from "./components/field";
export * from "./components/checkbox";
export * from "./components/tag";
export * from "./components/card";
export * from "./components/separator";
export * from "./components/kbd";
export * from "./components/spinner";
export * from "./components/empty-state";
export * from "./components/table";
export * from "./components/select";
export * from "./components/dialog";
export * from "./components/tooltip";

export * from "./editor/toolbar";
export * from "./editor/block-menu";
export * from "./editor/prose";
