// Portal CSAT surface — Wave 47 lane A1 (`p47-portal-session-hardening`).
//
// The CSAT survey is tokenized and single-use: the view never burns the
// token, and the response must ride the SAME token. A submit that posts the
// ticket id instead 404s, which leaves every survey unanswerable — a
// signed-in surface that looks alive and silently works for nobody, found
// while this lane proved the surface in a real browser.
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import PortalCsatPage from "../../app/portal/csat/page";
import { PortalAuthProvider, clearPortalTokens } from "../../lib/portal-api";
import { setFetchRouter } from "../helpers/fetch-router";

const TOKEN = "survey-token-abc";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn(), prefetch: vi.fn() }),
  usePathname: () => "/portal/csat",
  useSearchParams: () => new URLSearchParams(`ticket=${"survey-token-abc"}`),
}));

const VIEW = {
  ticket_id: "t1",
  ticket_number: "DEMO-1001",
  ticket_subject: "VPN drops every hour",
  status: "sent",
  expires_at: new Date(Date.now() + 86_400_000).toISOString(),
};
const RESPONDED = { ...VIEW, status: "responded" };

const PROFILE = {
  id: "u1",
  org_id: "o1",
  client_id: "c1",
  email: "ops@acme.com",
  display_name: "Ops User",
  role: "contact",
};

const seen: Array<{ method: string; path: string }> = [];

function jsonResponse(status: number, json: unknown): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers({ "content-type": "application/json" }),
    json: async () => json,
    text: async () => JSON.stringify(json),
  } as unknown as Response;
}

beforeEach(() => {
  clearPortalTokens();
  seen.length = 0;
  setFetchRouter(async (input, init) => {
    const url =
      typeof input === "string" ? new URL(input) : new URL((input as Request).url);
    const method = (init?.method ?? "GET").toUpperCase();
    seen.push({ method, path: url.pathname });
    if (method === "GET" && url.pathname === "/api/v1/portal/me") {
      return jsonResponse(200, PROFILE);
    }
    if (method === "GET" && url.pathname === `/api/v1/portal/csat/${TOKEN}`) {
      return jsonResponse(200, VIEW);
    }
    if (method === "POST" && url.pathname === `/api/v1/portal/csat/${TOKEN}`) {
      return jsonResponse(200, RESPONDED);
    }
    return jsonResponse(500, { error: { message: `unmatched ${method} ${url.pathname}` } });
  });
});

afterEach(() => {
  setFetchRouter(null);
});

describe("portal CSAT surface (Wave 47 lane A1)", () => {
  it("submits the survey under the token the view was opened with", async () => {
    render(
      <PortalAuthProvider>
        <PortalCsatPage />
      </PortalAuthProvider>,
    );

    // The survey form renders for the signed-in contact.
    await screen.findByTestId("portal-csat-5");

    fireEvent.click(screen.getByTestId("portal-csat-5"));
    fireEvent.change(screen.getByLabelText(/Anything you'd like to tell us/i), {
      target: { value: "Sam was quick and explained what he found." },
    });
    fireEvent.click(screen.getByTestId("portal-csat-send"));

    // The answer is recorded — the thank-you state replaces the form.
    await waitFor(() =>
      expect(screen.getByTestId("portal-csat-thanks")).toBeInTheDocument(),
    );

    const posted = seen.filter((c) => c.method === "POST" && c.path.startsWith("/api/v1/portal/csat/"));
    expect(posted).toHaveLength(1);
    expect(posted[0]?.path).toBe(`/api/v1/portal/csat/${TOKEN}`);
    cleanup();
  });
});
