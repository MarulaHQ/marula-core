// MovementDialog — context-embedding extensions (Wave 32 lane 6,
// `p32-stock-flows-context`): the scan fill, the ticket picker for
// stockroom dispatch, and the fixed-ticket ticket-side mode.
//
// Load-bearing assertions: a scan fills the form and NEVER records a
// movement; the ticket picker appears only when a stock/receiving context
// offers ticket options, never on the receive leg, and never when the
// ticket is already fixed.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import type { components } from "@marula-core/sdk";
import { AuthProvider } from "../../lib/api";
import { MovementDialog } from "../../components/stock/movement-dialog";
import { MockApi, ME_FIXTURE, seedAuthTokens } from "../helpers/mock-api";

const ITEM = {
  id: "00000000-0000-4000-8000-000000000010",
  name: "Cat6 patch cable (1 m)",
  kind: "consumable",
  unit: "each",
  sku: "C6-1M",
  part_number: null,
  active: true,
  is_tool: false,
  currency: "USD",
  last_unit_cost_minor: null,
} as unknown as components["schemas"]["StockItemView"];

const LOCATION = {
  id: "00000000-0000-4000-8000-000000000020",
  name: "Main shelf",
  kind: "shelf",
  active: true,
} as unknown as components["schemas"]["StockLocationView"];

const RESOLUTION = {
  input: "C6-1M",
  kind: "stock_item",
  entity_id: ITEM.id,
  stock_item_id: ITEM.id,
  serial_id: null,
  location_id: null,
  device_asset_id: null,
  display_name: "Cat6 patch cable (1 m)",
  secondary: "C6-1M",
  deep_link: `meridian://stock-item/${ITEM.id}`,
  is_tool: false,
  item_kind: "consumable",
  serial_status: null,
  candidates: [],
};

const TICKET_OPTIONS = [
  { id: "00000000-0000-4000-8000-000000000030", label: "T-42 · Printer down" },
];

function mockBackend(): MockApi {
  const mock = new MockApi();
  seedAuthTokens();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("POST", "/api/v1/scan/resolve", () => ({ json: RESOLUTION }));
  mock.install();
  return mock;
}

function typeBurst(target: HTMLElement, text: string) {
  for (const key of text) {
    fireEvent.keyDown(target, { key });
    if (target instanceof HTMLInputElement) {
      fireEvent.change(target, { target: { value: target.value + key } });
    }
  }
  fireEvent.keyDown(target, { key: "Enter" });
}

function renderDialog(overrides: Partial<ComponentProps<typeof MovementDialog>> = {}) {
  const onRecorded = vi.fn();
  render(
    <AuthProvider>
      <MovementDialog
        open
        onOpenChange={() => {}}
        items={[ITEM]}
        locations={[LOCATION]}
        scanSurface="ticket-parts"
        onRecorded={onRecorded}
        {...overrides}
      />
    </AuthProvider>,
  );
  return { onRecorded };
}

describe("MovementDialog context embedding", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("offers the ticket picker only where a ticket is not already fixed", () => {
    mockBackend();
    renderDialog({ initialKind: "consume", ticketOptions: TICKET_OPTIONS });
    expect(screen.getByTestId("movement-ticket")).toBeInTheDocument();
    expect(screen.queryByTestId("movement-ticket-anchor")).toBeNull();
  });

  it("hides the picker on a fixed-ticket surface and anchors the movement", () => {
    mockBackend();
    renderDialog({ initialKind: "consume", ticketId: TICKET_OPTIONS[0]?.id });
    expect(screen.queryByTestId("movement-ticket")).toBeNull();
    expect(screen.getByTestId("movement-ticket-anchor")).toBeInTheDocument();
  });

  it("does not offer a ticket picker on the receive leg", () => {
    mockBackend();
    renderDialog({ initialKind: "receive", ticketOptions: TICKET_OPTIONS });
    expect(screen.queryByTestId("movement-ticket")).toBeNull();
  });

  it("fills the item from a scan and records nothing on its own", async () => {
    const mock = mockBackend();
    renderDialog({ initialKind: "consume" });
    const input = screen.getByTestId("scan-input");
    input.focus();
    typeBurst(input, "C6-1M");
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/scan/resolve")).toHaveLength(1);
    });
    // No movement was written by the scan itself.
    expect(mock.callsTo("POST", "/api/v1/stock/consume")).toHaveLength(0);
    expect(mock.callsTo("POST", "/api/v1/stock/receive")).toHaveLength(0);
    // The scanned item is staged: submitting without a location fails on
    // the location step, not on "Pick an item."
    fireEvent.click(screen.getByTestId("movement-save"));
    await waitFor(() => {
      expect(screen.getByTestId("movement-error").textContent).toContain("source location");
    });
    expect(mock.callsTo("POST", "/api/v1/stock/consume")).toHaveLength(0);
  });
});
