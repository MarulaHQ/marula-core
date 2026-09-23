// Help panel (Wave 26 lane 2): route-keyed guide listing from the docs
// pipeline manifest, audience filtering, search, and the "Was this
// helpful?" write through the typed settings seam.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { HelpPanel } from "../../components/help/help-panel";
import { MockApi, ME_FIXTURE, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  usePathname: () => "/tickets",
}));

function mockBackend(): MockApi {
  const mock = new MockApi();
  seedAuthTokens();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/settings/user", () => ({ json: [] }));
  mock.on("GET", "/api/v1/settings/org", () => ({ json: [] }));
  mock.on("PUT", "/api/v1/settings/user", () => ({ json: { key: "k", value: "v" } }));
  mock.install();
  return mock;
}

async function renderPanel() {
  const mock = mockBackend();
  render(
    <AuthProvider>
      <HelpPanel />
    </AuthProvider>,
  );
  fireEvent.click(screen.getByTestId("help-button"));
  await screen.findByTestId("help-panel");
  return mock;
}

describe("HelpPanel", () => {
  beforeEach(() => {
    cleanup();
  });

  it("opens on the route-keyed guides for the current surface", async () => {
    await renderPanel();
    await waitFor(() => {
      expect(screen.getAllByTestId("help-doc-entry").length).toBeGreaterThan(0);
    });
    // /tickets maps to the ticket guides; staff-work-a-ticket leads.
    expect(screen.getByText("How to work a ticket")).toBeTruthy();
    expect(screen.getByTestId("help-surface-label").textContent).toContain("tickets");
  });

  it("hides admin-only guides from members", async () => {
    const mock = mockBackend();
    mock.on("GET", "/api/v1/me", () => ({
      json: { ...ME_FIXTURE, permissions: ["tickets:create"] },
    }));
    render(
      <AuthProvider>
        <HelpPanel />
      </AuthProvider>,
    );
    fireEvent.click(screen.getByTestId("help-button"));
    await screen.findByTestId("help-panel");
    await waitFor(() => {
      expect(screen.getAllByTestId("help-doc-entry").length).toBeGreaterThan(0);
    });
    // Admin workflow-categories guide is filtered out for a plain member.
    expect(screen.queryByText("Set workflow categories")).toBeNull();
  });

  it("searches every indexed guide once two characters are typed", async () => {
    await renderPanel();
    fireEvent.change(screen.getByTestId("help-search"), {
      target: { value: "invoice" },
    });
    await waitFor(() => {
      expect(screen.getByText(/first invoice/i)).toBeTruthy();
    });
  });

  it("records was-this-helpful feedback through the settings seam", async () => {
    const mock = await renderPanel();
    await waitFor(() => {
      expect(screen.getAllByTestId("help-feedback-up").length).toBeGreaterThan(0);
    });
    fireEvent.click(screen.getAllByTestId("help-feedback-up")[0]!);
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user").length).toBeGreaterThan(0);
    });
    const body = mock.lastBody<{ key: string; value: string }>(
      "PUT",
      "/api/v1/settings/user",
    )!;
    expect(body.key).toBe("help.doc_feedback");
    const log = JSON.parse(body.value) as { doc: string; up: boolean }[];
    expect(log.length).toBe(1);
    expect(log[0]!.up).toBe(true);
  });
});
