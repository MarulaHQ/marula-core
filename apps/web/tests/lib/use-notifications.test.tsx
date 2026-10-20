// useNotifications live leg: a per-user SSE frame bumps the badge only for its
// addressee, only on `notification.created`, and only once per notification.
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AuthProvider } from "../../lib/api";
import type { InstantEvent } from "../../lib/instant/events";
import {
  __resetInstantRuntimeForTests,
  instantRuntime,
} from "../../lib/instant/runtime";
import { useNotifications } from "../../lib/use-notifications";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const OTHER_USER_ID = "00000000-0000-4000-8000-0000000000ff";
const UNREAD_PATH = "/api/v1/notifications/unread-count";

function frame(
  type: string,
  entityId: string,
  recipient: string | null,
): InstantEvent {
  return {
    id: `evt-${entityId}`,
    type,
    entity: "notification",
    entity_id: entityId,
    occurred_at: "2026-10-20T09:00:00Z",
    payload: null,
    channel: `org:${ME_FIXTURE.org_id}`,
    seq: 1,
    recipient_user_id: recipient,
  };
}

function Probe() {
  const { unread } = useNotifications();
  return <span data-testid="unread">{unread}</span>;
}

function installApi() {
  const mock = new MockApi();
  // The stub server still reports an empty inbox: the refetch is the
  // authority, and it runs after the optimistic bump.
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", UNREAD_PATH, () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({
    json: { items: [], next_cursor: null },
  }));
  mock.install();
  return mock;
}

async function mount(mock: MockApi) {
  render(
    <AuthProvider>
      <Probe />
    </AuthProvider>,
  );
  // Auth restore resolves asynchronously; wait for the first inbox refetch so
  // "bumped by a frame" is never confused with "not loaded yet".
  await waitFor(() => {
    expect(mock.callsTo("GET", UNREAD_PATH)).toHaveLength(1);
  });
  expect(screen.getByTestId("unread").textContent).toBe("0");
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  __resetInstantRuntimeForTests();
});

afterEach(() => {
  cleanup();
});

describe("useNotifications — per-user live frames", () => {
  it("bumps the badge optimistically for this user's notification.created, then refetches", async () => {
    const mock = installApi();
    await mount(mock);

    act(() => {
      instantRuntime().bus.publish(
        frame("notification.created", "n1", ME_FIXTURE.user_id),
      );
    });

    // Synchronously after dispatch the badge already moved: the bump did not
    // wait for the network, which is what keeps a badge feeling instant.
    expect(screen.getByTestId("unread").textContent).toBe("1");
    // The refetch then lands and stays authoritative (the stub server has not
    // committed this notification, so it settles back to 0).
    await waitFor(() => {
      expect(screen.getByTestId("unread").textContent).toBe("0");
    });
    expect(mock.callsTo("GET", UNREAD_PATH)).toHaveLength(2);
  });

  it("ignores a notification.created addressed to another user", async () => {
    const mock = installApi();
    await mount(mock);
    const before = mock.callsTo("GET", UNREAD_PATH).length;

    act(() => {
      instantRuntime().bus.publish(
        frame("notification.created", "n2", OTHER_USER_ID),
      );
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByTestId("unread").textContent).toBe("0");
    // A frame for someone else neither bumps the badge nor triggers a refetch.
    expect(mock.callsTo("GET", UNREAD_PATH)).toHaveLength(before);
  });

  it("does not bump for notification.read but still refetches", async () => {
    const mock = installApi();
    await mount(mock);
    const before = mock.callsTo("GET", UNREAD_PATH).length;

    act(() => {
      instantRuntime().bus.publish(
        frame("notification.read", "n1", ME_FIXTURE.user_id),
      );
    });

    // Marking something read must never raise an unread badge.
    expect(screen.getByTestId("unread").textContent).toBe("0");
    await waitFor(() => {
      expect(mock.callsTo("GET", UNREAD_PATH)).toHaveLength(before + 1);
    });
    expect(screen.getByTestId("unread").textContent).toBe("0");
  });

  it("bumps once per entity_id when a frame is re-delivered", async () => {
    const mock = installApi();
    await mount(mock);

    act(() => {
      instantRuntime().bus.publish(
        frame("notification.created", "n3", ME_FIXTURE.user_id),
      );
    });
    expect(screen.getByTestId("unread").textContent).toBe("1");
    await waitFor(() => {
      expect(screen.getByTestId("unread").textContent).toBe("0");
    });
    const afterFirst = mock.callsTo("GET", UNREAD_PATH).length;

    act(() => {
      instantRuntime().bus.publish(
        frame("notification.created", "n3", ME_FIXTURE.user_id),
      );
    });
    await act(async () => {
      await Promise.resolve();
    });

    // At-least-once delivery: the same notification does not bump again.
    expect(screen.getByTestId("unread").textContent).toBe("0");
    expect(mock.callsTo("GET", UNREAD_PATH)).toHaveLength(afterFirst);
  });
});
