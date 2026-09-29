// Unit test: ConnectionBanner — offline/reconnecting recovery surfacing
// beyond IF6 (Wave 13 lane 2). The banner must stay quiet on a healthy
// connection, report queued depth while offline, and disappear on return.
import { describe, expect, it, beforeEach } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ConnectionBanner } from "../../components/instant/connection-banner";
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

describe("ConnectionBanner", () => {
  it("renders nothing on a healthy connection with an empty queue", () => {
    const { container } = render(<ConnectionBanner />);
    expect(container.textContent).toBe("");
  });

  it("reports the queued depth while offline (changes will replay)", () => {
    setOnline(false);
    const runtime = instantRuntime();
    runtime.queue.enqueue({
      key: "k1",
      method: "POST",
      path: "/api/v1/tickets/t1/status",
      body: { idempotency_key: "k1" },
      describe: "Set status Waiting on MER-12",
      entityId: "t1",
    });
    render(<ConnectionBanner />);
    expect(
      screen.getByText("Offline — 1 change will sync when you're back"),
    ).toBeTruthy();
  });

  it("reports reconnecting when the live leg is down but the queue is empty", () => {
    // No channels and no queue: the banner stays quiet (stream not in use).
    const quiet = render(<ConnectionBanner />);
    expect(quiet.container.textContent).toBe("");
    quiet.unmount();
    // Simulate a lost wire after use: a live-leg subscriber plus seeded
    // tokens puts the live leg into its connecting/reconnecting state.
    seedAuthTokens();
    const runtime = instantRuntime();
    const off = runtime.stream.subscribeOrg();
    render(<ConnectionBanner />);
    expect(screen.getByText("Reconnecting…")).toBeTruthy();
    off();
  });
});
