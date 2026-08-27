// Integration test: /settings/api-keys — scoped key creation with
// show-once raw material, listing columns (prefix/scopes/last-used), and
// revocation through its confirm dialog.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import ApiKeysSettingsPage from "../../app/settings/api-keys/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/api-keys",
}));

const KEY_ID = "60000000-0000-4000-8000-000000000001";

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/api-keys", () => ({
    json: {
      keys: [
        {
          id: KEY_ID,
          name: "NinjaOne sync",
          prefix: "mrd_ab12cd34",
          scopes: ["tickets:read"],
          created_at: "2026-01-01T00:00:00Z",
          last_used_at: null,
          revoked_at: null,
        },
      ],
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/api-keys integration", () => {
  it("lists keys with scope tags and a never-used last-used column", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ApiKeysSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("NinjaOne sync");
    expect(screen.getAllByText("tickets:read").length).toBeGreaterThan(0);
    expect(screen.getByText("never")).toBeInTheDocument();
    expect(screen.getByText("active")).toBeInTheDocument();
    cleanup();
  });

  it("creates a scoped key, requires at least one scope, shows raw material once", async () => {
    const user = userEvent.setup();
    const mock = setupApi();
    mock.on("POST", "/api/v1/api-keys", ({ body }) => {
      expect(body).toEqual({ name: "Hudu export", scopes: ["clients:read"] });
      return {
        status: 201,
        json: {
          key: {
            id: KEY_ID + "-2",
            name: "Hudu export",
            prefix: "mrd_ef12gh34",
            scopes: ["clients:read"],
            created_at: "2026-02-02T00:00:00Z",
            last_used_at: null,
            revoked_at: null,
          },
          raw_key: "mrd_ef12gh34deadbeefdeadbeef",
        },
      };
    });

    render(
      <AuthProvider>
        <ApiKeysSettingsPage />
      </AuthProvider>,
    );

    const submit = await screen.findByRole("button", { name: "Create key" });
    await user.type(screen.getByLabelText("Key name"), "Hudu export");
    // Still gated until a scope is picked.
    expect(submit).toBeDisabled();
    fireEvent.click(screen.getByLabelText("clients:read"));
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);

    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/api-keys").length).toBe(1);
    });
    expect(await screen.findByText("mrd_ef12gh34deadbeefdeadbeef")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("mrd_ef12gh34deadbeefdeadbeef")).not.toBeInTheDocument();
    cleanup();
  });

  it("revokes through confirmation and re-renders as revoked", async () => {
    const mock = new MockApi();
    let revoked = false;
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", "/api/v1/api-keys", () => ({
      json: {
        keys: [
          {
            id: KEY_ID,
            name: "NinjaOne sync",
            prefix: "mrd_ab12cd34",
            scopes: ["tickets:read"],
            created_at: "2026-01-01T00:00:00Z",
            last_used_at: "2026-03-03T10:11:12Z",
            revoked_at: revoked ? "2026-04-04T00:00:00Z" : null,
          },
        ],
      },
    }));
    mock.on("DELETE", `/api/v1/api-keys/${KEY_ID}`, () => {
      revoked = true;
      return { status: 204, json: null };
    });
    mock.install();

    render(
      <AuthProvider>
        <ApiKeysSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("NinjaOne sync");
    expect(screen.getByText("active")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
    const confirmButton = await screen.findByRole("button", { name: "Revoke now" });
    fireEvent.click(confirmButton);
    await screen.findByText("revoked");
    expect(mock.callsTo("DELETE", `/api/v1/api-keys/${KEY_ID}`).length).toBe(1);
    // Revoked keys offer no further action.
    expect(screen.queryByRole("button", { name: "Revoke" })).not.toBeInTheDocument();
    cleanup();
  });
});
