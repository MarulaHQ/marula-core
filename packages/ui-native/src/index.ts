/**
 * @marula-core/ui-native — the handoff kit's design values for React Native.
 *
 * `generated.ts` is written by `packages/ui/tokens/generate.mjs` from
 * `tokens/tokens.json`, the same file the web tokens come from, so the two
 * platforms cannot drift. Do not edit it by hand; run
 * `pnpm --filter @marula-core/ui tokens`.
 *
 * The app's own token module (`native/src/ui/tokens.ts`) builds its React
 * Native theme from these values; `native/tests/token-parity.test.ts` pins it.
 */
export * from "./generated";
