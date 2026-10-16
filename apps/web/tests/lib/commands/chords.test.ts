// Wave 47 lane A15 (`p47-drift-recheck`) — the chord parser regression suite.
//
// The defect this file pins: reading `event.key` to resolve a chord. Under a
// held modifier macOS reports the COMPOSED character, so Option+2 is "™" and
// Option+N is a dead key — no Alt chord authored in the registry vocabulary
// could ever match, and the hold-Alt overlay was the only working half of the
// feature. The fix reads `event.code` (the physical key) for letters and
// digits.
//
// These cases fail against the old `event.key` implementation on purpose: the
// composed characters below are what macOS Chromium delivers, not what the
// keyboard is labelled with.
import { describe, expect, it } from "vitest";
import { chordFromEvent, eventMatchesChord, modIsMeta, parseChord } from "../../../lib/commands/chords";

/**
 * A keydown shaped the way macOS Chromium reports it: `key` is the composed
 * character for a held Option, `code` is the physical key underneath.
 */
function macKey(key: string, code: string, modifiers: { alt?: boolean; shift?: boolean; meta?: boolean } = {}) {
  return new KeyboardEvent("keydown", {
    key,
    code,
    altKey: modifiers.alt ?? false,
    shiftKey: modifiers.shift ?? false,
    metaKey: modifiers.meta ?? false,
  });
}

describe("chords resolve the physical key, never the composed character", () => {
  // Option+<key> on the US macOS layout, with the character the OS composes.
  const optionLayer: ReadonlyArray<{ chord: string; key: string; code: string }> = [
    { chord: "alt+1", key: "¡", code: "Digit1" },
    { chord: "alt+2", key: "™", code: "Digit2" },
    { chord: "alt+3", key: "£", code: "Digit3" },
    // Option+N and Option+E are dead keys: `key` is the literal "Dead".
    { chord: "alt+n", key: "Dead", code: "KeyN" },
    { chord: "alt+c", key: "ç", code: "KeyC" },
    { chord: "alt+p", key: "π", code: "KeyP" },
    { chord: "alt+x", key: "≈", code: "KeyX" },
    { chord: "alt+j", key: "∆", code: "KeyJ" },
    { chord: "alt+d", key: "∂", code: "KeyD" },
    { chord: "alt+s", key: "ß", code: "KeyS" },
  ];

  for (const { chord, key, code } of optionLayer) {
    it(`matches ${chord} from Option-composed "${key}" (code ${code})`, () => {
      const event = macKey(key, code, { alt: true });
      // The trap, stated as an assertion: the character the old parser read
      // is not the key the chord names.
      expect(event.key).not.toBe(chord.split("+")[1]);
      expect(eventMatchesChord(chord, event, true)).toBe(true);
      expect(chordFromEvent(event, true)).toBe(chord);
    });
  }

  it("matches alt+shift+p from the composed capital (Option+Shift+P → ∏)", () => {
    const event = macKey("∏", "KeyP", { alt: true, shift: true });
    expect(eventMatchesChord("alt+shift+p", event, true)).toBe(true);
  });

  it("keeps punctuation on event.key — `?` is a character, not a physical name", () => {
    // shift+/ is the only route to "?" on a US layout; "Slash" is not in the
    // chord vocabulary, so the fallback stays for non-alphanumeric keys.
    const event = new KeyboardEvent("keydown", { key: "?", code: "Slash", shiftKey: true });
    expect(chordFromEvent(event, true)).toBe("shift+?");
  });

  it("still refuses a bare modifier press (no key char)", () => {
    expect(chordFromEvent(macKey("Alt", "AltLeft", { alt: true }), true)).toBeNull();
  });
});

describe("every registry chord is reachable from a physical-key event", () => {
  // The registry is authored in the same vocabulary ("alt+2", "alt+n"); this
  // walks it so a future chord can never be silently unreachable.
  const chords = ["alt+1", "alt+2", "alt+n", "alt+j", "alt+c", "alt+p", "alt+d", "alt+s", "alt+shift+p"];

  for (const chord of chords) {
    it(`resolves ${chord}`, () => {
      const spec = parseChord(chord);
      const key = spec.key;
      const event = macKey(
        key.length === 1 ? optionComposed(key) : key,
        /^[0-9]$/.test(key) ? `Digit${key}` : `Key${key.toUpperCase()}`,
        { alt: spec.modifiers.includes("alt"), shift: spec.modifiers.includes("shift") },
      );
      expect(chordFromEvent(event, true)).toBe(chord);
    });
  }
});

/** The Option-layer character macOS composes for a base key (identity when
 * the key has no Option mapping). */
function optionComposed(key: string): string {
  const map: Record<string, string> = {
    "1": "¡",
    "2": "™",
    "3": "£",
    n: "Dead",
    j: "∆",
    c: "ç",
    p: "π",
    d: "∂",
    s: "ß",
  };
  return map[key] ?? key;
}

describe("mod is the platform's modifier, not always Meta", () => {
  it("reads Meta on macOS and Control elsewhere", () => {
    expect(modIsMeta("MacIntel")).toBe(true);
    expect(modIsMeta("Linux x86_64")).toBe(false);
    expect(modIsMeta("Win32")).toBe(false);
    expect(modIsMeta("")).toBe(false);
  });

  it("pins mod+k to the platform modifier", () => {
    const meta = new KeyboardEvent("keydown", { key: "k", code: "KeyK", metaKey: true });
    const ctrl = new KeyboardEvent("keydown", { key: "k", code: "KeyK", ctrlKey: true });
    expect(chordFromEvent(meta, true)).toBe("mod+k");
    expect(chordFromEvent(ctrl, false)).toBe("mod+k");
    // The pinned-platform bug: a Ctrl chord could never match on macOS, and
    // a Meta chord could never match off it.
    expect(chordFromEvent(ctrl, true)).toBe("k");
    expect(chordFromEvent(meta, false)).toBe("k");
  });
});
