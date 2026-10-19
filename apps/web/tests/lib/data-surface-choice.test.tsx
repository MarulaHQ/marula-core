import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useSavedChoice } from "../../lib/data-surface-prefs";

const ALLOWED = ["mine", "unassigned", "all"] as const;

describe("useSavedChoice", () => {
  it("uses the fallback until the document resolves, then applies the saved choice", () => {
    const { result, rerender } = renderHook(
      ({ ready, saved }: { ready: boolean; saved: string | undefined }) =>
        useSavedChoice(ready, saved, ALLOWED, "mine"),
      { initialProps: { ready: false, saved: undefined as string | undefined } },
    );
    expect(result.current[0]).toBe("mine");
    rerender({ ready: true, saved: "all" });
    expect(result.current[0]).toBe("all");
  });

  it("ignores a saved choice this surface cannot render", () => {
    const { result } = renderHook(() => useSavedChoice(true, "decisions", ALLOWED, "mine"));
    expect(result.current[0]).toBe("mine");
  });

  it("does not undo a click when the document updates later", () => {
    const { result, rerender } = renderHook(
      ({ saved }: { saved: string | undefined }) =>
        useSavedChoice(true, saved, ALLOWED, "mine"),
      { initialProps: { saved: "all" as string | undefined } },
    );
    act(() => result.current[1]("unassigned"));
    expect(result.current[0]).toBe("unassigned");
    rerender({ saved: "all" });
    expect(result.current[0]).toBe("unassigned");
  });
});
