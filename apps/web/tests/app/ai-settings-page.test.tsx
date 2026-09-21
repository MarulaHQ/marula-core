// Integration test: the /settings/ai BYOK card (this lane extracted the
// legacy presence-only surface into ByokLegacyCard) — set/replace through
// the IF1 envelope, connection test result, removal. Key material never
// renders back.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import { ByokLegacyCard } from "../../components/settings/byok-legacy-card";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/ai",
}));

/** Providers reported as configured; handlers mutate this list. */
const configured: string[] = [];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/settings/ai/status", () => ({
    json: {
      active_provider: "anthropic",
      active_model: "claude-opus-4-1",
      configured_providers: configured,
    },
  }));
  mock.on("PUT", "/api/v1/settings/ai/byok", ({ body }) => {
    const b = body as { provider: string; api_key: string };
    if (!configured.includes(b.provider)) configured.push(b.provider);
    return {
      json: {
        provider: b.provider,
        has_key: true,
        updated_at: "2026-09-03T00:00:00Z",
        mutation: { after: null, before: null, seq: 2, updated_at: "2026-09-03T00:00:00Z", version: 2 },
      },
    };
  });
  mock.on("DELETE", "/api/v1/settings/ai/byok/anthropic", () => {
    const idx = configured.indexOf("anthropic");
    if (idx >= 0) configured.splice(idx, 1);
    return { json: { provider: "anthropic", has_key: false, updated_at: null } };
  });
  mock.on("POST", "/api/v1/settings/ai/test", ({ body }) => ({
    json: {
      provider: (body as { provider: string }).provider,
      ok: true,
      message: "handshake ok",
      latency_ms: 240,
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  configured.length = 0;
  configured.push("anthropic");
});

describe("/settings/ai integration", () => {
  it("shows sealed-key presence only, never key material", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ByokLegacyCard />
      </AuthProvider>,
    );
    await screen.findByText("anthropic");
    expect(screen.getByText("key sealed · never displayed")).toBeInTheDocument();
    expect(screen.getByText(/active: anthropic/)).toBeInTheDocument();
    expect(screen.getAllByText("not configured").length).toBeGreaterThan(0);
    cleanup();
  });

  it("saves a key with an idempotency key and presence reconciles optimistically", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ByokLegacyCard />
      </AuthProvider>,
    );
    await screen.findByText("anthropic");
    await user.type(screen.getByLabelText("openai API key"), "sk-live-abc123");
    const openaiRow = screen.getByLabelText("openai API key").closest("div[class*='border'], div") as HTMLElement;
    await user.click(within(openaiRow).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.callsTo("PUT", "/api/v1/settings/ai/byok").length).toBe(1));
    const body = mock.lastBody<{ idempotency_key: string; provider: string; api_key: string }>(
      "PUT",
      "/api/v1/settings/ai/byok",
    );
    expect(body.idempotency_key).toBeTruthy();
    expect(body.provider).toBe("openai");
    expect(body.api_key).toBe("sk-live-abc123");
    // The input clears and presence flips; the material never renders back.
    await waitFor(() => expect(screen.getAllByText("key sealed · never displayed").length).toBe(2));
    expect(screen.queryByText("sk-live-abc123")).not.toBeInTheDocument();
    cleanup();
  });

  it("tests the connection and surfaces the ok result", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ByokLegacyCard />
      </AuthProvider>,
    );
    await screen.findByText("anthropic");
    const anthropicRow = screen.getByLabelText("anthropic API key").closest("div[class*='border'], div") as HTMLElement;
    await user.click(within(anthropicRow).getByRole("button", { name: "Test" }));
    await screen.findByText(/Connected: handshake ok \(240 ms\)/);
    cleanup();
  });

  it("removes a configured key", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ByokLegacyCard />
      </AuthProvider>,
    );
    await screen.findByText("anthropic");
    const anthropicRow = screen.getByLabelText("anthropic API key").closest("div[class*='border'], div") as HTMLElement;
    await user.click(within(anthropicRow).getByRole("button", { name: "Remove" }));
    await waitFor(() =>
      expect(mock.callsTo("DELETE", "/api/v1/settings/ai/byok/anthropic").length).toBe(1),
    );
    await waitFor(() => {
      const rowAfter = screen.getByLabelText("anthropic API key").closest(
        "div[class*='flex flex-wrap']",
      ) as HTMLElement;
      expect(within(rowAfter).getByText("not configured")).toBeInTheDocument();
    });
    cleanup();
  });
});
