// Documentation completeness — the score itself moved server-side
// (GET /api/v1/clients/{id}/completeness); the client page renders the
// backend view verbatim. What remains client-side is the score pill's
// tone mapping; this suite keeps that mapping AA-contract-checked and
// pins the backend view shape the page renders.
import { describe, expect, it } from "vitest";
import { scoreTone } from "../../lib/docs-completeness";
import type { components } from "@marula-core/sdk";

describe("scoreTone", () => {
  it("maps scores to AA-safe semantic tones", () => {
    expect(scoreTone(100)).toBe("success");
    expect(scoreTone(75)).toBe("success");
    expect(scoreTone(74)).toBe("warning");
    expect(scoreTone(40)).toBe("warning");
    expect(scoreTone(39)).toBe("danger");
    expect(scoreTone(0)).toBe("danger");
  });
});

describe("backend completeness contract", () => {
  it("the generated schema carries the completeness view the page renders", () => {
    const view: components["schemas"]["ClientCompletenessView"] = {
      score: 62,
      basis: "1 site(s) · 2 contact(s) · n/a asset(s)",
      gaps: [{ key: "assets", label: "No assets", hint: "Add at least the key endpoints." }],
    };
    expect(view.score).toBe(62);
    expect(view.gaps[0]!.key).toBe("assets");
  });
});
