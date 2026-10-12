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
export {
  DENSITY_STEPS,
  DENSITY_STEP_SPEC,
  DENSITY_STORAGE_KEY,
  DEFAULT_DENSITY,
  applyDensity,
  isDensity,
  resolveInitialDensity,
  useDensity,
  type Density,
} from "./density";
export type {
  FixedRole,
  Platform,
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
  defaultPresetFor,
  PLATFORM_PRESET_DEFAULT,
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
export * from "./components/badge";
export * from "./components/assistant-card";
export * from "./components/card";
export * from "./components/separator";
export * from "./components/kbd";
export * from "./components/spinner";
export * from "./components/empty-state";
export * from "./components/table";
export * from "./components/select";
export * from "./components/dialog";
export * from "./components/tooltip";
export * from "./components/status-stepper";
export * from "./components/composer-frame";
export * from "./components/toaster";
export * from "./components/density-switch";
export * from "./components/data-toolbar";
export * from "./components/view-switch";
export * from "./components/column-chooser";
export * from "./components/filter-bar";
export * from "./components/export-control";
export * from "./components/surface-header";
export * from "./components/surface-columns";

export * from "./editor/toolbar";
export * from "./editor/block-menu";
export * from "./editor/prose";
