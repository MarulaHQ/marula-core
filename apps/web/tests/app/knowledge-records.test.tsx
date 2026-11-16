// The rack elevation and the expiry wording, as components.
//
// The elevation is a drawing, so the test that matters is that the drawing and
// its table say the same thing: every placed device appears in the accessible
// table with its unit range, and the drawing's own label counts the units used.
import { describe, expect, it, beforeEach } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { RackElevation } from "../../components/knowledge/rack-elevation";
import {
  daysUntil,
  dueWords,
  ExpiryTable,
} from "../../components/knowledge/expiry-table";

const DEVICES = [
  {
    id: "d1",
    rack_id: "r1",
    asset_id: null,
    name: "SW-CORE01",
    device_type: "network" as const,
    face: "front" as const,
    unit_start: 41,
    unit_height: 1,
    model: "Aruba 6100",
    serial: null,
    status: "online" as const,
    power_notes: null,
    network_notes: null,
  },
  {
    id: "d2",
    rack_id: "r1",
    asset_id: null,
    name: "ACME-HV01",
    device_type: "compute" as const,
    face: "front" as const,
    unit_start: 17,
    unit_height: 2,
    model: "Dell R650",
    serial: null,
    status: "attention" as const,
    power_notes: null,
    network_notes: null,
  },
];

function isoDaysFromNow(days: number): string {
  const date = new Date();
  date.setDate(date.getDate() + days);
  // Local fields, not `toISOString()`: `daysUntil` counts local calendar days,
  // and a UTC slice shifts the fixture by one when the runner is past 16:00
  // local time (or behind UTC).
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

describe("RackElevation", () => {
  beforeEach(cleanup);

  it("places devices and states the used units", () => {
    render(<RackElevation units={42} devices={DEVICES} label="Rack A" />);
    expect(
      screen.getByRole("img", { name: /Rack A, front view\. 2 devices, 3 of 42 units used\./ }),
    ).toBeTruthy();
    // The accessible table is the alternative to the drawing.
    expect(screen.getAllByText("SW-CORE01").length).toBeGreaterThan(0);
    expect(screen.getByText("U17 to U18")).toBeTruthy();
  });

  it("only draws the face it was asked for", () => {
    render(<RackElevation units={42} devices={DEVICES} face="rear" label="Rack A" />);
    expect(screen.queryAllByText("SW-CORE01")).toHaveLength(0);
  });
});

describe("expiry wording", () => {
  beforeEach(cleanup);

  it("counts whole days and names the state", () => {
    expect(daysUntil(isoDaysFromNow(0))).toBe(0);
    expect(dueWords(isoDaysFromNow(0)).text).toBe("Due today");
    expect(dueWords(isoDaysFromNow(12)).text).toBe("Due in 12 days");
    expect(dueWords(isoDaysFromNow(12)).tone).toBe("warning");
    expect(dueWords(isoDaysFromNow(-3)).text).toBe("Expired 3 days ago");
    expect(dueWords(isoDaysFromNow(-3)).tone).toBe("danger");
    expect(dueWords(isoDaysFromNow(120)).tone).toBe("neutral");
  });

  it("tells red rows from blush rows in words", () => {
    render(
      <ExpiryTable
        rows={[
          {
            kind: "domain_tls",
            title: "acme.example",
            client_id: "c1",
            client_name: "Acme",
            due_on: isoDaysFromNow(9),
            owner: "DigiCert",
            source_id: "s1",
            provenance: "certificate record",
          },
          {
            kind: "asset_warranty",
            title: "ACME-DC01",
            client_id: "c1",
            client_name: "Acme",
            due_on: isoDaysFromNow(-2),
            owner: "Dell",
            source_id: "s2",
            provenance: "asset field",
          },
        ]}
      />,
    );
    expect(screen.getByText("Due in 9 days")).toBeTruthy();
    expect(screen.getByText("Expired 2 days ago")).toBeTruthy();
    // Provenance is on screen, so a typed warranty never reads like a registry.
    expect(screen.getByText("asset field")).toBeTruthy();
  });
});
