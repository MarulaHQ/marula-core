// ScanBar — the shared keyboard-wedge affordance (Wave 32 lane 5,
// `p32-labels-scanning`).
//
// The load-bearing assertions are the FOCUS LAW: a scan that arrives while
// a form field has focus must route, must not submit the form, must not
// move focus, and must not leave scanner garbage in the field.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { ScanBar } from "../../components/scan/scan-bar";
import { MockApi, ME_FIXTURE, seedAuthTokens } from "../helpers/mock-api";

const STOCK_ITEM = {
  input: "C6-1M",
  kind: "stock_item",
  entity_id: "00000000-0000-4000-8000-000000000010",
  stock_item_id: "00000000-0000-4000-8000-000000000010",
  serial_id: null,
  location_id: null,
  device_asset_id: null,
  display_name: "Cat6 patch cable (1 m)",
  secondary: "C6-1M",
  deep_link: "marula://stock-item/00000000-0000-4000-8000-000000000010",
  is_tool: false,
  item_kind: "consumable",
  serial_status: null,
  candidates: [],
};

function mockBackend(response: unknown = STOCK_ITEM): MockApi {
  const mock = new MockApi();
  seedAuthTokens();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("POST", "/api/v1/scan/resolve", () => ({ json: response }));
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

function renderBar(onScan: (event: unknown) => void) {
  return render(
    <AuthProvider>
      <ScanBar surface="ticket-parts" onScan={onScan} />
    </AuthProvider>,
  );
}

describe("ScanBar", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("routes a burst typed into its own field and clears the input", async () => {
    mockBackend();
    const onScan = vi.fn();
    renderBar(onScan);
    const input = screen.getByTestId("scan-input");
    input.focus();
    typeBurst(input, "C6-1M");
    await waitFor(() => {
      expect(onScan).toHaveBeenCalledTimes(1);
    });
    const event = onScan.mock.calls[0]?.[0] as { resolution: { kind: string } };
    expect(event.resolution.kind).toBe("stock_item");
    expect((input as HTMLInputElement).value).toBe("");
    expect(screen.getByTestId("scan-chip")).toBeInTheDocument();
  });

  it("does NOT submit a form, steal focus, or pollute the field when a scan arrives mid-form", async () => {
    mockBackend();
    const onScan = vi.fn();
    const onSubmit = vi.fn((event: React.FormEvent) => event.preventDefault());
    render(
      <AuthProvider>
        <form onSubmit={onSubmit}>
          <input data-testid="foreign-field" defaultValue="ticket note: " />
          <ScanBar surface="ticket-parts" onScan={onScan} />
        </form>
      </AuthProvider>,
    );
    const field = screen.getByTestId("foreign-field") as HTMLInputElement;
    field.focus();
    typeBurst(field, "C6-1M");
    await waitFor(() => {
      expect(onScan).toHaveBeenCalledTimes(1);
    });
    expect(onSubmit).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe("ticket note: ");
    expect(screen.getByTestId("scan-chip-summary").textContent).toContain("Cat6 patch cable");
  });

  it("reports an unknown code honestly instead of routing it", async () => {
    mockBackend({
      ...STOCK_ITEM,
      input: "NOPE-404",
      kind: "unknown",
      entity_id: null,
      stock_item_id: null,
      display_name: "",
      secondary: null,
      deep_link: null,
    });
    const onScan = vi.fn();
    renderBar(onScan);
    const input = screen.getByTestId("scan-input");
    input.focus();
    // Unknown text arrives slowly (manual entry), then Enter.
    const { keyDown, change } = { keyDown: fireEvent.keyDown, change: fireEvent.change };
    keyDown(input, { key: "N" });
    change(input, { target: { value: "NOPE-404" } });
    keyDown(input, { key: "Enter" });
    await waitFor(() => {
      expect(screen.getByTestId("scan-notice")).toBeInTheDocument();
    });
    expect(onScan).not.toHaveBeenCalled();
    expect(screen.getByTestId("scan-notice").textContent).toContain("NOPE-404");
  });

  it("offers candidates for an ambiguous code and only routes a picked one", async () => {
    const ambiguous = {
      ...STOCK_ITEM,
      entity_id: null,
      stock_item_id: null,
      candidates: [
        { kind: "stock_item", entity_id: "a", display_name: "Cable 1 m", secondary: "C6-1M" },
        { kind: "stock_item", entity_id: "b", display_name: "Cable 2 m", secondary: "C6-1M" },
      ],
    };
    const mock = mockBackend(ambiguous);
    const onScan = vi.fn();
    renderBar(onScan);
    const input = screen.getByTestId("scan-input");
    input.focus();
    typeBurst(input, "C6-1M");
    await waitFor(() => {
      expect(screen.getAllByTestId("scan-candidate")).toHaveLength(2);
    });
    expect(onScan).not.toHaveBeenCalled();
    fireEvent.click(screen.getAllByTestId("scan-candidate")[1] as HTMLElement);
    await waitFor(() => {
      expect(onScan).toHaveBeenCalledTimes(1);
    });
    expect(mock.callsTo("POST", "/api/v1/scan/resolve")).toHaveLength(1);
  });
});
