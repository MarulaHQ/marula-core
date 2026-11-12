// Unit test: ConnectionPresence — the top bar's connection line.
//
// The line always says something (the mockup draws "● Online" at rest), it
// names a degraded state in words rather than colour alone, and while the
// outbox holds anything it reports the depth so the person can judge whether
// their last edits are safe.
import { describe, expect, it, beforeEach } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ConnectionPresence } from "../../components/instant/connection-presence";
import { instantRuntime, __resetInstantRuntimeForTests } from "../../lib/instant/runtime";
import { seedAuthTokens } from "../helpers/mock-api";

function setOnline(online: boolean): void {
  Object.defineProperty(window.navigator, "onLine", {
    configurable: true,
    value: online,
  });
}

beforeEach(() => {
  __resetInstantRuntimeForTests();
  setOnline(true);
  window.localStorage.removeItem("marula-instant-outbox");
  return () => {
    __resetInstantRuntimeForTests();
    cleanup();
  };
});

describe("ConnectionPresence", () => {
  it("reads Online on a healthy connection", () => {
    render(<ConnectionPresence />);
    expect(screen.getByTestId("connection-presence").textContent).toBe("Online");
    expect(screen.getByTestId("connection-presence").getAttribute("data-state")).toBe("online");
  });

  it("reports the queued depth while offline", () => {
    setOnline(false);
    const runtime = instantRuntime();
    runtime.queue.enqueue({
      key: "k1",
      method: "POST",
      path: "/api/v1/tickets/t1/status",
      body: { idempotency_key: "k1" },
      describe: "Set status Waiting on MAR-12",
      entityId: "t1",
    });
    render(<ConnectionPresence />);
    expect(screen.getByText("Offline · 1 queued")).toBeTruthy();
    expect(screen.getByTestId("connection-presence").getAttribute("data-state")).toBe("degraded");
  });

  it("says Offline with an empty queue", () => {
    setOnline(false);
    render(<ConnectionPresence />);
    expect(screen.getByTestId("connection-presence").textContent).toBe("Offline");
  });

  it("says Reconnecting when the live leg is in use but down", () => {
    seedAuthTokens();
    const runtime = instantRuntime();
    const off = runtime.stream.subscribeOrg();
    render(<ConnectionPresence />);
    expect(screen.getByText("Reconnecting…")).toBeTruthy();
    off();
  });
});
