// Unit test: ConnectionPresence — the top bar's connection line.
//
// The line always says something (the mockup draws "● Online" at rest), it
// names a degraded state in words rather than colour alone, and it claims
// "Reconnecting" only after this page has held an open stream — a bar that
// cries reconnect on every first paint teaches people to ignore it.
import { describe, expect, it, beforeEach } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ConnectionPresence, presenceOf } from "../../components/instant/connection-presence";
import { __resetInstantRuntimeForTests } from "../../lib/instant/runtime";

const base = {
  online: true,
  status: "open" as const,
  pending: 0,
  resyncing: false,
  everOpen: true,
};

beforeEach(() => {
  __resetInstantRuntimeForTests();
  return () => cleanup();
});

describe("presenceOf", () => {
  it("reads Online on a healthy connection", () => {
    expect(presenceOf(base)).toEqual({ tone: "healthy", label: "Online" });
  });

  it("stays quiet during the first connect instead of claiming a reconnect", () => {
    expect(presenceOf({ ...base, status: "connecting", everOpen: false })).toEqual({
      tone: "healthy",
      label: "Online",
    });
  });

  it("reports the queued depth while offline", () => {
    expect(presenceOf({ ...base, online: false, pending: 1 })).toEqual({
      tone: "attention",
      label: "Offline · 1 queued",
    });
  });

  it("says Offline with an empty queue", () => {
    expect(presenceOf({ ...base, online: false, pending: 0 })).toEqual({
      tone: "attention",
      label: "Offline",
    });
  });

  it("says Reconnecting once a stream that had opened drops", () => {
    expect(presenceOf({ ...base, status: "connecting", everOpen: true })).toEqual({
      tone: "attention",
      label: "Reconnecting…",
    });
  });

  it("names a resync as catching up, which explains the longer wait", () => {
    expect(presenceOf({ ...base, resyncing: true, status: "connecting" })).toEqual({
      tone: "attention",
      label: "Catching up…",
    });
  });

  it("keeps reporting a pending outbox after the wire has closed", () => {
    expect(presenceOf({ ...base, status: "closed", pending: 2 })).toEqual({
      tone: "attention",
      label: "Reconnecting…",
    });
  });
});

describe("ConnectionPresence", () => {
  it("renders the word, not only the dot", () => {
    render(<ConnectionPresence />);
    expect(screen.getByTestId("connection-presence").textContent).toBe("Online");
    expect(screen.getByTestId("connection-presence").getAttribute("data-state")).toBe("online");
  });
});
