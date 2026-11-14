"use client";
/**
 * @marula-core/ui — Marula design system barrel.
 * Purpose: single public entry point for tokens API, theming, and components.
 * Non-purpose: stylesheets ship separately (`@marula-core/ui/tokens.css` +
 * `components.css`) and must be imported by the host app once at its root.
 */

export { cn } from "./lib/cn";
export { useFocusTrap } from "./lib/focus-trap";
export * from "./lib/time";
export * from "./lib/number";
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
export * from "./components/time";
export * from "./components/checkbox";
export * from "./components/badge";
export * from "./components/assistant-card";
export * from "./components/card";
export * from "./components/separator";
export * from "./components/kbd";
export * from "./components/spinner";
export * from "./components/skeleton";
export * from "./components/empty-state";
export * from "./components/table";
export * from "./components/plan-table";
export * from "./components/inspector";
export * from "./components/bottom-sheet";
export * from "./components/impact-preview";
export * from "./components/select";
export * from "./components/menu";
export * from "./components/dialog";
export * from "./components/confirm-dialog";
export * from "./components/schema-form";
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
export * from "./components/surface-footer";
export * from "./components/bulk-action-bar";
export * from "./components/status-pill";
export * from "./components/capability-chip";
export * from "./components/linked-records-strip";
// The project timeline. `gantt-scale` is exported beside the chart because a
// surface that formats a date without it would format it differently.
export * from "./components/gantt-chart";
export * from "./components/gantt-table";
export * from "./components/gantt-scale";

export * from "./editor/toolbar";
export * from "./editor/block-menu";
export * from "./editor/prose";

// The kit's toggle is re-exported from the one barrel so a surface never has
// to know whether a primitive came from `./components` or `./kit`.
export { Switch } from "./kit/ui/switch";
