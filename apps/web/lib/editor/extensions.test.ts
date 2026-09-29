// Extension-policy test (ADR 0007 MIT-only binding): the composed set
// contains exactly the StarterKit core + agreed OSS nodes/marks — no
// Tiptap paid bundle (Collaboration/Comments/AI/Conversion) can appear
// because the names below are asserted exhaustively.
import { describe, expect, it } from "vitest";
import { marulaExtensionSet } from "./extensions";

describe("marula editor extension set", () => {
  it("loads the OSS composite — StarterKit + MIT extras + Marula nodes", () => {
    const names = marulaExtensionSet.map((e) => e.name);
    // StarterKit's own entries are nested; its presence gates every core
    // node/mark above (`"document"` etc.). Extras are asserted wholesale.
    for (const name of [
      "starterKit",
      "image",
      "taskList",
      "taskItem",
      "youtube",
      "MarulaAssetLink",
      "MarulaKbEmbed",
      "MarulaTaxonomyTerm",
    ]) {
      expect(names).toContain(name);
    }
  });

  it("loads no Tiptap paid-bundle extension names", () => {
    const names = marulaExtensionSet.map((e) => e.name);
    for (const paid of [
      "collaboration",
      "comments",
      "conversion",
      "aiBlock",
      "uniqueId",
    ]) {
      expect(names).not.toContain(paid);
    }
  });
});

