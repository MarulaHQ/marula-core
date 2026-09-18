// Integration test: /settings/export (org-export D1 admin) over the real
// SDK client on a mocked fetch router — owner/admin builds the bundle and
// gets the signed download; non-admins get an honest typed "not available"
// state; a typed server rejection surfaces verbatim.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import ExportSettingsPage from "../../app/settings/export/page";
import { AuthProvider } from "../../lib/api";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/export",
}));

const EXPORT_CREATED = {
  bundle_version: 1,
  export_id: "e2e00000-0000-4000-8000-000000000002",
  download_url:
    "/api/v1/org/exports/o1/e2/download?expires=4200000000&signature=sig",
  expires_at: "2035-05-20T00:00:00.000Z",
  sha256: "a".repeat(64),
  size_bytes: 48213,
};

let mock: MockApi;

function setupApi(me: typeof ME_FIXTURE) {
  mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("POST", "/api/v1/org/exports", () => ({ json: EXPORT_CREATED }));
  mock.install();
}

const ADMIN_ME = {
  ...ME_FIXTURE,
  permissions: ["special:manage_users"],
};
const MEMBER_ME = {
  ...ME_FIXTURE,
  permissions: ["tickets:view"],
};

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/export", () => {
  it("owner/admin: generate → result card with size/sha/bundle version + signed download anchor", async () => {
    setupApi(ADMIN_ME);
    render(
      <AuthProvider>
        <ExportSettingsPage />
      </AuthProvider>,
    );
    expect(await screen.findByRole("heading", { name: "Export" })).toBeInTheDocument();
    fireEvent.click(await screen.findByTestId("export-create"));
    const result = await screen.findByTestId("export-result", {}, { timeout: 3000 });
    expect(result).toHaveTextContent("bundle v1");
    expect(result).toHaveTextContent("47.1 KiB");
    expect(result).toHaveTextContent("a".repeat(32));
    const anchor = screen.getByTestId("export-download") as HTMLAnchorElement;
    expect(anchor.getAttribute("href")).toContain(EXPORT_CREATED.download_url);
    expect(anchor.getAttribute("download")).not.toBeNull();
  });

  it("non-admin: no dead button; honest gated state naming the role limit", async () => {
    setupApi(MEMBER_ME);
    render(
      <AuthProvider>
        <ExportSettingsPage />
      </AuthProvider>,
    );
    expect(await screen.findByTestId("settings-not-available")).toHaveTextContent(
      /org data export is limited to org admins/,
    );
    expect(screen.queryByTestId("export-create")).toBeNull();
    expect(screen.queryByRole("button", { name: "Generate export" })).toBeNull();
  });

  it("typed 403 copy surfaces verbatim when the server rejects", async () => {
    // Simulate an admin whose grant was just revoked server-side.
    const mock2 = new MockApi();
    mock2.on("GET", "/api/v1/me", () => ({ json: ADMIN_ME }));
    mock2.on("POST", "/api/v1/org/exports", () => ({
      status: 403,
      json: { error: { reason: "forbidden", message: "org data export is limited to org admins" } },
    }));
    mock2.install();
    render(
      <AuthProvider>
        <ExportSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId("export-create"));
    const error = await screen.findByTestId("export-error", {}, { timeout: 3000 });
    // The server's own 403 message names the org-admin limit; the app maps
    // the typed `reason: forbidden` onto the FW reason-code copy.
    expect(error).toHaveTextContent(/permission for this action/);
  });
});
