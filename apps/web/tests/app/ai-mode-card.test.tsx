// The one AI control: three states, three different promises.
//
// Off runs nothing, Shadow runs the models and records what they would have
// said, Live lets each surface act on its own mode. The control must send the
// word, not a boolean, or a shadow org is indistinguishable from a live one.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import { AiModeCard } from "../../app/settings/ai/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/ai",
}));

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("the AI mode control", () => {
  it("draws all three states and marks the stored one", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.install();
    render(
      <AuthProvider>
        <AiModeCard mode="shadow" onChanged={vi.fn()} />
      </AuthProvider>,
    );
    const radios = await screen.findAllByRole("radio");
    expect(radios.map((r) => r.textContent)).toEqual(["Off", "Shadow", "Live"]);
    expect(screen.getByRole("radio", { name: "Shadow" })).toHaveAttribute("aria-checked", "true");
    cleanup();
  });

  it("sends the word and reports the new state", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("PUT", "/api/v1/settings/ai/decision/features", () => ({
      json: {
        features_enabled: true,
        mode: "shadow",
        mutation: { seq: 1, version: 1, updated_at: "2026-11-16T00:00:00Z", before: null, after: null },
      },
    }));
    mock.install();
    const onChanged = vi.fn();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <AiModeCard mode="off" onChanged={onChanged} />
      </AuthProvider>,
    );
    await user.click(await screen.findByRole("radio", { name: "Shadow" }));
    await waitFor(() =>
      expect(mock.callsTo("PUT", "/api/v1/settings/ai/decision/features").length).toBe(1),
    );
    const body = mock.lastBody<{ mode: string; idempotency_key: string }>(
      "PUT",
      "/api/v1/settings/ai/decision/features",
    );
    expect(body.mode).toBe("shadow");
    expect(body.idempotency_key).toBeTruthy();
    await waitFor(() => expect(onChanged).toHaveBeenCalledWith("shadow"));
    cleanup();
  });

  it("reports a refused save, keeps the stored mode and lets the same control retry", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    let attempts = 0;
    mock.on("PUT", "/api/v1/settings/ai/decision/features", () => {
      attempts += 1;
      if (attempts === 1) {
        return {
          status: 409,
          json: { error: { reason: "version_conflict", message: "the mode moved under you" } },
        };
      }
      return {
        json: {
          features_enabled: true,
          mode: "live",
          mutation: { seq: 2, version: 2, updated_at: "2026-11-16T00:00:00Z", before: null, after: null },
        },
      };
    });
    mock.install();
    const onChanged = vi.fn();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <AiModeCard mode="off" onChanged={onChanged} />
      </AuthProvider>,
    );

    await user.click(await screen.findByRole("radio", { name: "Live" }));
    // A refused save is said out loud, and the prop the parent owns is not
    // advanced: the control must not claim a state the server does not hold.
    expect(await screen.findByRole("alert")).toHaveTextContent(/could not be saved/i);
    expect(onChanged).not.toHaveBeenCalled();

    // The control stays usable, so the retry is the same click.
    await user.click(screen.getByRole("radio", { name: "Live" }));
    await waitFor(() => expect(onChanged).toHaveBeenCalledWith("live"));
    expect(mock.callsTo("PUT", "/api/v1/settings/ai/decision/features").length).toBe(2);
    cleanup();
  });
});
