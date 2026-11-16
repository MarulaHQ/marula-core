// /kb/networks/[id] — the subnets form on one network.
//
// Purpose: pin the rule this page has to hold when the person moves while a
// subnet is being saved. A POST started on network A completes after a move to
// network B; its follow-up reload must not run, because it would advance the
// page's request token, throw B's pending answer away, and draw A again.
//
// Non-purpose: the layout of the record frame, which the pack review holds.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { AuthProvider } from "../../lib/api";
import NetworkPage from "../../app/kb/networks/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import { currentFetchRouter, setFetchRouter } from "../helpers/fetch-router";

const route = vi.hoisted(() => ({ id: "net-a" }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: route.id }),
  usePathname: () => "/kb/networks",
}));

function network(id: string, name: string) {
  return {
    id,
    client_id: "c1",
    site_id: null,
    name,
    kind: "lan",
    gateway: "10.0.0.1",
    dns: null,
    vlan_zone: "core",
    notes: null,
    fields: {},
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    subnets: [],
    reservations: [],
    links: [],
  };
}

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
  route.id = "net-a";
  setFetchRouter(null);
});

function Harness() {
  const [id, setId] = useState(route.id);
  return (
    <AuthProvider>
      <button
        type="button"
        onClick={() => {
          route.id = "net-b";
          setId("net-b");
        }}
      >
        open the other network
      </button>
      <NetworkPage />
    </AuthProvider>
  );
}

describe("/kb/networks/[id]", () => {
  it("does not reload the network a subnet was submitted for after a move", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", "/api/v1/knowledge/networks/net-a", () => ({
      json: network("net-a", "Front desk LAN"),
    }));
    mock.on("GET", "/api/v1/knowledge/networks/net-b", () => ({
      json: network("net-b", "Back office VLAN"),
    }));
    mock.on("POST", "/api/v1/knowledge/networks/net-a/subnets", () => ({
      json: { id: "subnet-1" },
    }));
    mock.install();

    // Hold the subnet POST so the move to network B happens while it runs.
    let release = (): void => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const delegate = currentFetchRouter();
    setFetchRouter(async (input, init) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const method = (
        typeof input === "object" && "method" in input && input.method !== undefined
          ? input.method
          : (init?.method ?? "GET")
      ).toUpperCase();
      if (method === "POST" && url.includes("/subnets")) await gate;
      if (delegate === null) throw new Error("no MockApi router installed");
      return delegate(input, init);
    });

    render(<Harness />);
    expect((await screen.findAllByText("Front desk LAN")).length).toBeGreaterThan(0);

    fireEvent.change(screen.getByLabelText("CIDR"), { target: { value: "10.0.30.0/24" } });
    fireEvent.click(screen.getByRole("button", { name: "Add subnet" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Adding…" })).toBeTruthy(),
    );

    fireEvent.click(screen.getByRole("button", { name: "open the other network" }));
    expect((await screen.findAllByText("Back office VLAN")).length).toBeGreaterThan(0);

    // The POST lands after the move. Its reload belongs to the network it was
    // started under, so it must not fetch that network's detail again.
    await act(async () => {
      release();
    });
    expect(mock.callsTo("POST", "/api/v1/knowledge/networks/net-a/subnets")).toHaveLength(1);
    expect(mock.callsTo("GET", "/api/v1/knowledge/networks/net-a")).toHaveLength(1);
    expect(screen.getAllByText("Back office VLAN").length).toBeGreaterThan(0);
    expect(screen.queryAllByText("Front desk LAN")).toHaveLength(0);
  });
});
