// Integration test: /login register mode — the org_slug_taken 409 renders
// friendly copy from the central reason map and offers one-click accept of
// the envelope's `suggestion` (Wave 13 lane-1 follow-up, web leg).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import LoginPage from "../../app/login/page";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/login",
}));

function setupApi(): { mock: MockApi } {
  const mock = new MockApi();
  mock.on("POST", "/api/v1/auth/register", (ctx) => {
    const body = ctx.body as { org_slug?: string } | undefined;
    if (body?.org_slug === "acme-it") {
      return {
        status: 409,
        json: {
          error: {
            code: 409,
            reason: "org_slug_taken",
            message: "that workspace address is already taken",
            suggestion: "acme-it-2",
          },
        },
      };
    }
    return {
      json: {
        access_token: "a",
        refresh_token: "r",
        session_id: "s",
        expires_in: 900,
      },
    };
  });
  mock.install();
  return { mock };
}

beforeEach(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
});

describe("/login — register mode slug collision", () => {
  it("renders the typed reason copy and one-click accepts the suggested slug", async () => {
    const { mock } = setupApi();
    const user = userEvent.setup();
    render(<LoginPage />);

    // Switch to the Create org segment.
    await user.click(screen.getByRole("tab", { name: "Create org" }));

    await user.type(screen.getByPlaceholderText("Acme IT"), "Acme IT");
    await user.type(screen.getByPlaceholderText("acme-it"), "acme-it");
    await user.type(screen.getByPlaceholderText("Dana Owner"), "Dana Owner");
    await user.type(screen.getByPlaceholderText("you@company.com"), "dana@acme.test");
    // The password field's <Label> is nested, not `for`-associated — select
    // it by input type instead.
    const passwordInput = document.querySelector(
      'input[type="password"]',
    ) as HTMLInputElement;
    await user.type(passwordInput, "correct-horse-battery");

    await user.click(screen.getByRole("button", { name: "Create organization" }));

    // Friendly copy — the fixed human line, not raw JSON or the reason code.
    await screen.findByText("That workspace address is already taken.");
    // The suggestion surfaces with a one-click accept.
    const accept = await screen.findByRole("button", { name: /Use acme-it-2/ });
    await user.click(accept);

    // The slug field now carries the suggestion and the retry re-registers.
    await waitFor(() => {
      const calls = mock.callsTo("POST", "/api/v1/auth/register");
      expect(calls).toHaveLength(2);
    });
    expect(
      (mock.lastBody<{ org_slug: string }>("POST", "/api/v1/auth/register") as { org_slug: string })
        .org_slug,
    ).toBe("acme-it-2");
    cleanup();
  });
});
