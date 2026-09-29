/**
 * The `cn` contract (wave41 D9).
 *
 * The previous implementation was a hand-rolled `twMerge(clsx(...))` on
 * `tailwind-merge@^2.6.0` — the Tailwind **v3** class table — while the app
 * runs `tailwindcss@^4.1.0`. Names the v3 table does not know about silently
 * failed to de-duplicate, so the FIRST class won instead of the LAST. The
 * theme swap rewrites every variant in every primitive at once, which is
 * exactly the condition that surfaces the bug, so the fix is pinned here
 * rather than left to a comment.
 */
import { describe, expect, it } from "vitest";

import { cn } from "../src/lib/cn";

describe("cn", () => {
  it("keeps the last of two conflicting classes", () => {
    expect(cn("h-8", "h-11")).toBe("h-11");
    expect(cn("p-6", "p-2")).toBe("p-2");
  });

  it("de-duplicates the v4 class names the v3 table never knew", () => {
    // v4 conflict groups the 2.x table did not have. `size-*` sets both axes,
    // so it supersedes `h-*`/`w-*` — tailwind-merge 2.x kept `h-11 size-11`
    // and let the stale one win. `ring-3` and `outline-hidden` are v4
    // spellings, and both are about to appear in every primitive.
    expect(cn("h-9", "h-11", "size-11")).toBe("size-11");
    expect(cn("w-9", "w-11", "size-11")).toBe("size-11");
    expect(cn("ring-1", "ring-3")).toBe("ring-3");
    expect(cn("outline-solid", "outline-hidden")).toBe("outline-hidden");
  });

  it("merges conditional class values and flattens arrays", () => {
    const showHint = false;
    expect(
      cn("rounded-md", ["border", showHint && "hidden", { "border-destructive": true }]),
    ).toBe("rounded-md border border-destructive");
  });

  it("leaves non-conflicting classes alone", () => {
    expect(cn("flex items-center", "rounded-full")).toBe("flex items-center rounded-full");
  });
});
