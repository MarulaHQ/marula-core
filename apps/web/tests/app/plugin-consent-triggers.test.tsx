// Focused test: the plugin consent screen's surface profile — Wave 43
// lane 5 spec item (C): declared TRIGGER surfaces and the event types they
// emit render beside the endpoints/scopes and UI-surface blocks.
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ConsentSummary } from "../../app/settings/plugins/consent";

function artifact() {
  return {
    artifact_kind: "plugin",
    artifact_digest: "sha256:abc",
    source_kind: "marketplace_registry",
    source_url: "registry://rmm",
    fetched_ref: "v1.2.3",
    declared_capabilities: ["automation.triggers"],
  };
}

describe("consent screen — declared trigger surfaces (Wave 43 lane 5)", () => {
  it("renders each trigger with its emitted event type, plus a type summary", () => {
    render(
      <ConsentSummary
        artifact={artifact()}
        connectorProfile={{ webhook_endpoints: ["https://rmm.example/hook"], scopes: ["devices:read"] }}
        surfaceProfile={{
          widgets: [],
          actions: [],
          render_kinds: [],
          field_validators: [],
          triggers: [
            {
              id: "device_offline",
              label: "Device offline",
              description: "the appliance stopped checking in",
              emitted_event_type: "rmm.device_offline",
            },
            {
              id: "backup_failed",
              label: "Backup failed",
              description: "a scheduled backup exited non-zero",
              emitted_event_type: "rmm.backup_failed",
            },
          ],
        }}
      />,
    );

    // Triggers render with their declarations, beside the connector block.
    expect(screen.getByText(/Device offline — the appliance stopped checking in/)).toBeInTheDocument();
    expect(screen.getByText(/emits “rmm\.device_offline”/)).toBeInTheDocument();
    expect(screen.getByText(/Backup failed/)).toBeInTheDocument();
    expect(screen.getByText("webhook → https://rmm.example/hook")).toBeInTheDocument();

    // The emitted-type summary names each type once.
    const summary = screen.getByTestId("emitted-event-types");
    expect(summary.textContent).toContain("rmm.device_offline");
    expect(summary.textContent).toContain("rmm.backup_failed");

    // The trigger capability carries its governance note.
    expect(screen.getByText(/Automation triggers — the plugin may declare event types/)).toBeInTheDocument();
  });

  it("renders read nodes with their declared guardrails", () => {
    render(
      <ConsentSummary
        artifact={{ ...artifact(), declared_capabilities: ["automation.reads"] }}
        surfaceProfile={{
          triggers: [],
          reads: [
            {
              id: "list_devices",
              label: "List devices",
              description: "the org's appliance inventory",
              guardrails: {
                max_calls_per_run: 10,
                min_interval_secs: 30,
                max_output_bytes: 262144,
                may_return_secrets: false,
              },
            },
          ],
        }}
      />,
    );
    expect(screen.getByText(/List devices — the org's appliance inventory/)).toBeInTheDocument();
    // The caps are what consent approves, so they ride the declaration.
    expect(screen.getByText(/≤10 calls\/run/)).toBeInTheDocument();
    expect(screen.getByText(/≥30s apart/)).toBeInTheDocument();
    expect(screen.getByText(/≤262144 bytes back/)).toBeInTheDocument();
    expect(screen.getByText(/secrets never/)).toBeInTheDocument();
    expect(screen.getByText(/Read nodes — the plugin may fetch EXTERNAL data/)).toBeInTheDocument();
  });

  it("renders no trigger block for a plugin that declares none", () => {
    render(
      <ConsentSummary
        artifact={artifact()}
        surfaceProfile={{
          widgets: [{ id: "w1", title: "CSAT panel", slot: "dashboard", kind: "metric" }],
          actions: [],
          render_kinds: [],
          field_validators: [],
          triggers: [],
        }}
      />,
    );
    expect(screen.queryByText(/Automation triggers:/)).not.toBeInTheDocument();
    expect(screen.queryByTestId("emitted-event-types")).not.toBeInTheDocument();
    // The other surface sections still render.
    expect(screen.getByText(/Widgets:/)).toBeInTheDocument();
  });
});
