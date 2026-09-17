// Note formatter tests — mirrors of the server-side B1–B6/L rules
// (docs/notes-tasks-workbar.md §2.4). The core guarantee is B6 idempotence,
// property-based here just like the server's proptest.
import { describe, expect, it } from "vitest";

import { formatNoteBody, initialsOf } from "./note-format";

describe("formatNoteBody", () => {
  it("bullets every line canonically (B2)", () => {
    expect(formatNoteBody("fixed printer\ndiagnosed cable")).toBe(
      "- Fixed printer\n- Diagnosed cable",
    );
  });

  it("normalizes typed markers (B2)", () => {
    expect(formatNoteBody("- a\n* b\n• c\n– d")).toBe("- A\n- B\n- C\n- D");
  });

  it("preserves ordered numerals (B2 exception)", () => {
    expect(formatNoteBody("1. boot scan\n2) run diags")).toBe(
      "1. Boot scan\n2) Run diags",
    );
  });

  it("keeps up to three indent spaces (B3)", () => {
    expect(formatNoteBody("  - sub item")).toBe("  - Sub item");
    expect(formatNoteBody("    four spaces")).toBe("- Four spaces");
  });

  it("capitalizes the first letter only (B4)", () => {
    expect(formatNoteBody("dns forwarded after TTL refresh")).toBe(
      "- Dns forwarded after TTL refresh",
    );
  });

  it("never adds terminal punctuation (B5)", () => {
    expect(formatNoteBody("works")).toBe("- Works");
  });

  it("collapses blank lines (B1)", () => {
    expect(formatNoteBody("\n\n\na\n\n\n\n\nb\n\n")).toBe("- A\n- B");
    expect(formatNoteBody("")).toBe("");
  });

  it("passes scheme links verbatim (L1) and markdown links untouched (L2)", () => {
    expect(formatNoteBody("https://example.com/x?y=1 &z")).toBe(
      "- https://example.com/x?y=1 &z",
    );
    expect(formatNoteBody("[docs](https://example.com/docs)")).toBe(
      "- [docs](https://example.com/docs)",
    );
  });

  it("prefixes bare-host URL lines with https (L3)", () => {
    expect(formatNoteBody("www.example.com/status")).toBe(
      "- https://www.example.com/status",
    );
    expect(formatNoteBody("example.com hosted here")).toBe(
      "- Example.com hosted here",
    );
  });

  it("is idempotent on every sample (B6)", () => {
    const samples = [
      "- a\n* b\n1. c\nhttps://example.com\n\n\n\nd",
      "**Completed:** Fix DNS forwarding",
      "•  mixed   bullets\n  - sub\n2) second",
      "",
      "\n\n\nsingle",
    ];
    for (const s of samples) {
      const once = formatNoteBody(s);
      expect(formatNoteBody(once)).toBe(once);
    }
  });

  it("property: idempotence for generated drafts (B6)", () => {
    // Deterministic pseudo-random drafts.
    const tokens = [" a", "- x", "1. n", "https://e.com", "", "  - s", "• y"];
    for (let seed = 0; seed < 200; seed++) {
      let draft = "";
      let r = seed * 2654435761 % 2 ** 31;
      for (let i = 0; i < 6; i++) {
        r = (r * 1103515245 + 12345) % 2 ** 31;
        draft += tokens[r % tokens.length] + "\n";
      }
      const once = formatNoteBody(draft);
      expect(formatNoteBody(once)).toBe(once);
    }
  });
});

describe("initialsOf", () => {
  it("first+last initials, single-name first-two", () => {
    expect(initialsOf("Jordan Reyes")).toBe("JR");
    expect(initialsOf("Amara")).toBe("AM");
    expect(initialsOf(undefined)).toBe("··");
  });
});
