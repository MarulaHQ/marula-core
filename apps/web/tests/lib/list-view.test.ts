import { describe, expect, it } from "vitest";
import { showsEmptyRegister } from "../../lib/list-view";

describe("an empty register", () => {
  it("may say nothing is there only when the read succeeded", () => {
    expect(showsEmptyRegister(0, null)).toBe(true);
  });

  it("stays silent when the read failed, because the error already speaks", () => {
    expect(showsEmptyRegister(0, "Could not load the register — try reloading.")).toBe(false);
  });

  it("is never empty while rows are on screen", () => {
    expect(showsEmptyRegister(3, null)).toBe(false);
    expect(showsEmptyRegister(3, "stale")).toBe(false);
  });
});
