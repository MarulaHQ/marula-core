/**
 * Mobile tokens against the handoff kit's generated tokens (wave41 lane 20).
 *
 * `packages/ui-native/src/generated.ts` is written from the same
 * `tokens/tokens.json` the web tokens come from, so this test is what stops
 * the native theme drifting from the kit. It caught three drifted values when
 * it was added: the light hover surface and both danger pairs.
 */
import { describe, expect, it } from "vitest";
import { themes } from "@marula-core/ui-native";
import { darkTokens, lightTokens, type ThemeTokens } from "../src/ui/tokens";

/** Native field -> kit token key. `textMuted` has no kit equivalent (third level). */
const PAIRS: Array<[keyof ThemeTokens, string]> = [
  ["background", "bg"],
  ["surface", "surf"],
  ["surfaceHover", "surf2"],
  ["overlay", "surf"],
  ["text", "fg"],
  ["textSecondary", "sub"],
  ["border", "bd"],
  ["primary", "pr"],
  ["primaryText", "prf"],
  ["destructive", "red"],
  ["destructiveText", "redf"],
];

const norm = (v: string) => v.toUpperCase();

describe("native tokens equal the kit's generated tokens", () => {
  it.each(PAIRS)("light: %s equals kit %s", (field, key) => {
    expect(norm(lightTokens[field] as string)).toBe(norm((themes.light as Record<string, string>)[key]!));
  });

  it.each(PAIRS)("dark: %s equals kit %s", (field, key) => {
    expect(norm(darkTokens[field] as string)).toBe(norm((themes.dark as Record<string, string>)[key]!));
  });

  it("light status pairs equal the kit's", () => {
    expect(norm(lightTokens.success.text)).toBe(norm(themes.light.gtext));
    expect(norm(lightTokens.success.background)).toBe(norm(themes.light.gsoft));
    expect(norm(lightTokens.warning.text)).toBe(norm(themes.light.progf));
    expect(norm(lightTokens.warning.background)).toBe(norm(themes.light.progb));
    expect(norm(lightTokens.danger.text)).toBe(norm(themes.light.stBrF!));
    expect(norm(lightTokens.danger.background)).toBe(norm(themes.light.stBrB!));
  });

  it("dark status pairs equal the kit's", () => {
    expect(norm(darkTokens.success.text)).toBe(norm(themes.dark.gtext));
    expect(norm(darkTokens.success.background)).toBe(norm(themes.dark.gsoft));
    expect(norm(darkTokens.warning.text)).toBe(norm(themes.dark.progf));
    expect(norm(darkTokens.warning.background)).toBe(norm(themes.dark.progb));
    expect(norm(darkTokens.danger.text)).toBe(norm(themes.dark.stBrF!));
    expect(norm(darkTokens.danger.background)).toBe(norm(themes.dark.stBrB!));
  });
});
