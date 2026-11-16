// The auth-ready gate's deadline (lib/api.tsx).
//
// Purpose: a settled auth restore must leave no timer behind, and a restore
// that never settles must still release its waiters at the timeout. The two
// legs are declared in this order on purpose: the module resolves its gate
// once, so the timeout leg has to run while the gate is still unsettled.
//
// Non-purpose: the provider's own restore logic (token rotation, /me), which
// page-level suites already cover through real clients.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";

import { authReady, AuthProvider } from "../../lib/api";

beforeEach(() => {
  // Only the gate's deadline is faked. Date, microtasks and React's scheduler
  // stay real so the provider's restore promise chain runs normally.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("authReady", () => {
  it("resolves at its timeout when auth never settles, leaving no timer", async () => {
    let settled = false;
    const waiting = authReady(5_000).then(() => {
      settled = true;
    });
    expect(vi.getTimerCount()).toBe(1);

    await vi.advanceTimersByTimeAsync(4_999);
    expect(settled).toBe(false);

    await vi.advanceTimersByTimeAsync(1);
    await waiting;
    expect(settled).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("clears its timer when auth settles first", async () => {
    const ready = authReady(60_000);
    let settled = false;
    void ready.then(() => {
      settled = true;
    });
    expect(vi.getTimerCount()).toBe(1);

    // No tokens and no installed fetch route: the cookie-driven refresh is
    // rejected, which is the provider's anonymous settle — the same path a
    // signed-out page takes, so this is the real notifyAuthSettled call and
    // not a harness shortcut.
    render(
      <AuthProvider>
        <span>child</span>
      </AuthProvider>,
    );

    // `waitFor` advances fake time only up to its own 1s budget, well short of
    // the 60s deadline, so a resolution here proves the settle leg won.
    await vi.waitFor(() => expect(settled).toBe(true));
    await expect(ready).resolves.toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });
});
