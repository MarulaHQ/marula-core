import expoConfig from "eslint-config-expo/flat.js";

export default [
  ...expoConfig,
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
