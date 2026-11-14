/**
 * StatusPill — the connector status vocabulary. The pill is four words; the
 * mappings (raw API string → state, several connections → worst state) are the
 * part that must not drift between the roster, a chip and the detail panel.
 */
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  StatusPill,
  statusOfConnection,
  statusOfConnector,
} from "../../src/components/status-pill";

describe("statusOfConnection", () => {
  it("maps the raw contract strings onto the four states", () => {
    expect(statusOfConnection("connected")).toBe("connected");
    expect(statusOfConnection("degraded")).toBe("needs_attention");
    expect(statusOfConnection("auth_failed")).toBe("needs_attention");
    expect(statusOfConnection("needs_attachment")).toBe("needs_attention");
    expect(statusOfConnection("disabled")).toBe("disabled");
    expect(statusOfConnection("not_connected")).toBe("not_connected");
  });

  it("treats an unknown status as not proven healthy", () => {
    expect(statusOfConnection("negotiating")).toBe("not_connected");
  });
});

describe("statusOfConnector", () => {
  it("reads a connector with nothing bound as not connected", () => {
    expect(statusOfConnector([])).toBe("not_connected");
  });

  it("lets one failing connection outrank healthy ones", () => {
    expect(
      statusOfConnector([{ status: "connected" }, { status: "connected" }, { status: "auth_failed" }]),
    ).toBe("needs_attention");
  });

  it("reads connected when at least one connection works and none failed", () => {
    expect(statusOfConnector([{ status: "disabled" }, { status: "connected" }])).toBe("connected");
  });

  it("reads disabled when every connection is switched off", () => {
    expect(statusOfConnector([{ status: "disabled" }])).toBe("disabled");
  });
});

describe("StatusPill", () => {
  it("renders the word and the state, never colour alone", () => {
    render(<StatusPill status="needs_attention" />);
    const pill = screen.getByTestId("status-pill");
    expect(pill).toHaveTextContent("Needs attention");
    expect(pill).toHaveAttribute("data-status", "needs_attention");
  });
});
