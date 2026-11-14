/**
 * ESLint flat config for @marula-core/native (Expo).
 *
 * `fixupConfigRules` is load-bearing on ESLint 10. eslint-config-expo brings
 * eslint-plugin-react@7.37.5, whose React-version detection calls
 * `context.getFilename()` — removed in ESLint 10 — so every react/* rule it
 * enables throws `contextOrFilename.getFilename is not a function` and takes
 * the run down with exit 2. @eslint/compat backfills the rule-context methods
 * ESLint 10 removed, and it is the bridge the ESLint team ships for exactly
 * this case.
 *
 * Measured 2026-11-15 on this branch: `eslint .` reports 6 warnings and 0
 * errors under ESLint 10.14.0 with the shim, identical to ESLint 9.39.5
 * without it. A probe file still reports react/jsx-key, react/display-name and
 * react/no-children-prop, so the rules run rather than silently no-op.
 *
 * Drop the shim when eslint-plugin-react publishes ESLint 10 support
 * (jsx-eslint/eslint-plugin-react#4022) or when eslint-config-expo moves off
 * that plugin.
 */
import { fixupConfigRules } from "@eslint/compat";
import expoConfig from "eslint-config-expo/flat.js";

export default [
  ...fixupConfigRules(expoConfig),
  {
    ignores: ["dist/*", ".expo/*", "node_modules/*", "coverage/*"],
  },
  {
    files: ["**/*.ts", "**/*.tsx"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector: "Literal[value=/#[0-9a-fA-F]{3,8}\\b/]",
          message:
            "Raw hex colors are banned outside native/src/ui/tokens.ts (docs/theme.md §9). Use the token map.",
        },
        {
          selector: "TemplateElement[value.raw=/#[0-9a-fA-F]{3,8}\\b/]",
          message:
            "Raw hex colors are banned outside native/src/ui/tokens.ts (docs/theme.md §9). Use the token map.",
        },
      ],
    },
  },
  {
    files: ["src/ui/tokens.ts"],
    rules: {
      "no-restricted-syntax": "off",
    },
  },
];
