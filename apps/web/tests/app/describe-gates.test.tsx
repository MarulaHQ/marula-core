// Integration test: every AI admission gate, as the entries render it.
//
// Purpose: the six gates the row names (AI off for the org, the surface off, no
// grant, a cap, no credential, no allowlisted model) are six different states,
// and each must arrive as a stated reason plus the one remedy — never as a
// disabled button with no explanation and never as a fabricated draft.
//
// One panel, one mapping: the /reports and /dashboards entries both render
// `DescribeDraftPanel`, and its refusal state reads `aiGateCopy` from
// `lib/error-copy`. A new gate that the map does not name still lands on the
// generic row rather than a blank, which the last case pins.
//
// Non-purpose: the server's admission order and reason codes, which
// `crates/core-api/src/routes/ai_policy.rs` owns and its tests cover.
import { describe, expect, it, beforeEach } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DescribeDraftPanel } from "../../components/reports/describe-draft-panel";
import { MockApi, seedAuthTokens } from "../helpers/mock-api";

const AUTHOR_PATH = "/api/v1/reports/author";

/** reason → the sentence a person must be able to read on the surface. */
const GATES: Array<{ reason: string; title: RegExp; remedy: RegExp }> = [
  {
    reason: "ai_features_disabled",
    title: /AI is switched off for this organization/i,
    remedy: /Settings, AI/i,
  },
  {
    reason: "ai_surface_disabled",
    title: /This AI surface is switched off/i,
    remedy: /enable the surface/i,
  },
  {
    reason: "ai_surface_not_granted",
    title: /No grant covers you on this surface/i,
    remedy: /grant it under Settings, AI/i,
  },
  {
    reason: "ai_hard_cap_reached",
    title: /AI usage has reached its cap for this period/i,
    remedy: /raise the cap/i,
  },
  {
    reason: "ai_no_credential",
    title: /No model credential is available/i,
    remedy: /provider key under Settings, AI, credentials/i,
  },
  {
    reason: "ai_model_not_allowed",
    title: /not on this organization's allowlist/i,
    remedy: /allowlisted model/i,
  },
];

function setupRefusal(reason: string) {
  const mock = new MockApi();
  mock.on("POST", AUTHOR_PATH, () => ({
    status: 403,
    json: {
      error: { reason, message: `the server's own sentence for ${reason}` },
    },
  }));
  mock.install();
  return mock;
}

async function generateOnce() {
  render(
    <DescribeDraftPanel surface="reports" onEdit={() => {}} />,
  );
  const box = screen.getByLabelText("What should this report answer?");
  fireEvent.change(box, { target: { value: "tickets by priority" } });
  fireEvent.click(screen.getByRole("button", { name: "Generate a draft" }));
}

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
});

describe("describe-it entry: typed not-available states", () => {
  for (const gate of GATES) {
    it(`states ${gate.reason} with a remedy and no draft`, async () => {
      const mock = setupRefusal(gate.reason);
      await generateOnce();

      const notice = await screen.findByTestId("ai-gate-notice", {}, { timeout: 5000 });
      expect(notice).toHaveAttribute("data-reason", gate.reason);
      expect(notice).toHaveTextContent(gate.title);
      expect(notice).toHaveTextContent(gate.remedy);

      // The row still says what state it is in, but there is nothing to accept.
      const row = screen.getByTestId("report-draft-row");
      expect(row).toHaveAttribute("data-status", "refused");
      const accept = screen.getByRole("button", { name: "Accept and save" });
      expect(accept).toBeDisabled();
      fireEvent.click(accept);
      expect(mock.callsTo("POST", "/api/v1/report-definitions")).toHaveLength(0);
    });
  }

  it("gives an unnamed AI reason the generic row rather than a blank", async () => {
    setupRefusal("ai_something_new");
    await generateOnce();

    const notice = await screen.findByTestId("ai-gate-notice", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "ai_something_new");
    expect(notice).toHaveTextContent(/AI is not available for this organization right now/i);
    expect(notice).toHaveTextContent(/Settings, AI/i);
  });

  it("keeps the server's sentence for a refusal that is not an AI gate", async () => {
    setupRefusal("forbidden");
    await generateOnce();

    const notice = await screen.findByTestId("ai-gate-notice", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "forbidden");
    expect(notice).toHaveTextContent(/the server's own sentence for forbidden/i);
    await waitFor(() =>
      expect(screen.getByTestId("report-draft-row")).toHaveAttribute("data-status", "refused"),
    );
  });
});
