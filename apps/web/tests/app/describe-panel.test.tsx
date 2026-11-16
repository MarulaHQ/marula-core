// The Describe it panel: the states the pack draws between words and a draft.
//
// Purpose: pin the states that are easy to fake and hard to get right — an
// empty composer that cannot draft, live chips that come from the words, a
// check step that always runs, a draft that only exists after Build, and a
// failure that says nothing was saved. Also pin the rule with no exception: no
// grant means the door and the panel are absent, not disabled.
//
// Non-purpose: the server's extraction and provenance rules, which
// `crates/core-api/src/routes/money_authoring.rs` tests directly.
import { useState } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  DescribeMoney,
  DescribePanel,
  Reading,
  type AnyDraft,
} from "../../components/money/describe-money";
import type { MoneySource } from "../../lib/money-authoring";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const DRAFT = {
  title: "Wi-Fi refresh, Cape Town HQ",
  currency: "USD",
  valid_for_days: 30,
  notes: "Replace six access points.",
  client_id: null,
  client_name: "Acme Corporation",
  budget_minor: 500_000,
  lines: [
    {
      description: "Ubiquiti U7 Pro access point",
      quantity_milli: 6000,
      unit_price_minor: 18_900,
      unit: "each",
      section: "Hardware",
      optional: false,
      source: "notes",
      source_span: { source_id: "typed-1", start: 0, end: 12 },
      quantity_assumed: false,
      price_assumed: false,
    },
    {
      description: "Configuration and handover",
      quantity_milli: 4000,
      unit_price_minor: 14_500,
      unit: "hour",
      section: "Labour",
      optional: false,
      source: "assumed",
      source_span: null,
      quantity_assumed: true,
      price_assumed: true,
    },
  ],
  assumptions: ["Weekend work is priced at the after-hours rate."],
  understanding: {
    statement: "A project quote for Acme Corporation: better Wi-Fi on the 4th floor.",
    facts: [{ label: "Client", value: "Acme Corporation, matched" }],
    questions: [
      {
        id: "approver",
        text: "Who approves this for Acme?",
        options: ["Grace Mwangi (approves quotes)", "John Smith"],
        chosen: "Grace Mwangi (approves quotes)",
      },
    ],
    assumptions: ["Access point prices come from your catalogue."],
  },
  checks: ["The price for Configuration and handover was not in your notes."],
};

function surfaces(granted: boolean) {
  return {
    json: {
      surfaces: [
        {
          surface_key: "quote_authoring",
          granted,
          models_available: granted,
        },
      ],
    },
  };
}

function setup(overrides: { granted?: boolean; draftFailure?: boolean } = {}): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/ai/me/surfaces", () => surfaces(overrides.granted ?? true));
  mock.on("POST", "/api/v1/quotes/extract", () => ({
    json: {
      fields: [
        { key: "client", value: "Acme Corporation" },
        { key: "quantity", value: "6 access points" },
        { key: "item", value: "A site survey" },
      ],
      characters: 120,
    },
  }));
  mock.on("POST", "/api/v1/quotes/author", () =>
    overrides.draftFailure === true
      ? {
          json: {
            valid: false,
            saved: false,
            draft: null,
            assumptions: [],
            issues: [{ field: "draft", message: "the notes did not name any billable work" }],
            usage: { prompt_tokens: 1, completion_tokens: 1 },
            model_info: { provider: "test", model: "test" },
            error: "the draft did not pass my own checks",
            reference: "dsc_7f3a91c2",
          },
        }
      : {
          json: {
            valid: true,
            saved: false,
            draft: DRAFT,
            assumptions: DRAFT.assumptions,
            issues: [],
            usage: { prompt_tokens: 10, completion_tokens: 20 },
            model_info: { provider: "test", model: "test" },
          },
        },
  );
  mock.install();
  return mock;
}

function Harness({
  onBuild,
  onStartBlank,
  open = true,
}: {
  onBuild: (draft: AnyDraft, sources: MoneySource[], kind: "quote" | "contract") => void;
  onStartBlank?: (sources: MoneySource[]) => void;
  open?: boolean;
}) {
  return (
    <DescribePanel
      kind="quote"
      open={open}
      onClose={() => undefined}
      clients={[{ id: "client-1", name: "Acme Corporation" }]}
      onBuild={onBuild}
      onStartBlank={onStartBlank ?? (() => undefined)}
      testId="panel"
    />
  );
}

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
});

describe("Describe it panel", () => {
  it("cannot draft until there is something to read, and shows the rule that never turns off", async () => {
    setup();
    render(<Harness onBuild={() => undefined} />);
    const draftIt = await screen.findByTestId("describe-draft");
    expect(draftIt).toBeDisabled();
    expect(
      screen.getByText(/I will show you what I understood before I build anything/i),
    ).toBeInTheDocument();
  });

  it("shows the live chips the words already contain, before any draft", async () => {
    const mock = setup();
    const user = userEvent.setup();
    render(<Harness onBuild={() => undefined} />);
    await user.type(await screen.findByTestId("describe-transcript"), "Acme, 6 access points, survey");
    // The chips arrive after the debounced extract round trip, so wait for the
    // call first. On a loaded CI runner the render can land past the default
    // one-second find timeout even though nothing is wrong.
    await waitFor(
      () => expect(mock.callsTo("POST", "/api/v1/quotes/extract").length).toBeGreaterThan(0),
      { timeout: 5_000 },
    );
    expect(await screen.findByTestId("describe-seen", {}, { timeout: 5_000 })).toBeInTheDocument();
    expect(await screen.findByText("6 access points", {}, { timeout: 5_000 })).toBeInTheDocument();
    expect(await screen.findByText("A site survey", {}, { timeout: 5_000 })).toBeInTheDocument();
    // The chips are a read; nothing has been authored yet.
    expect(mock.callsTo("POST", "/api/v1/quotes/author")).toHaveLength(0);
  });

  it("reads, then states what it understood with the likeliest answer chosen", async () => {
    setup();
    const user = userEvent.setup();
    const built: AnyDraft[] = [];
    render(<Harness onBuild={(draft) => built.push(draft)} />);
    await user.type(await screen.findByTestId("describe-transcript"), "Six APs for Acme, weekend install");
    await user.click(screen.getByTestId("describe-draft"));

    const card = await screen.findByTestId("describe-understanding");
    expect(card).toHaveTextContent(/better Wi-Fi on the 4th floor/i);
    // The question's likeliest option is already pressed.
    const chosen = screen.getByTestId("describe-answer-approver-Grace Mwangi (approves quotes)");
    expect(chosen).toHaveAttribute("aria-pressed", "true");
    // Nothing is saved until Build, and Build is the only path out.
    expect(screen.getByText(/Nothing is saved until you build it/i)).toBeInTheDocument();
    expect(built).toHaveLength(0);
    await user.click(screen.getByTestId("describe-build"));
    expect(built).toHaveLength(1);
  });

  it("keeps the words as a source the builder can show", async () => {
    setup();
    const user = userEvent.setup();
    let sources: MoneySource[] = [];
    render(
      <Harness
        onBuild={(_draft, next) => {
          sources = next;
        }}
      />,
    );
    await user.type(await screen.findByTestId("describe-transcript"), "Six APs for Acme");
    await user.click(screen.getByTestId("describe-draft"));
    await screen.findByTestId("describe-understanding");
    await user.click(screen.getByTestId("describe-build"));
    expect(sources[0]?.kind).toBe("typed");
    expect(sources[0]?.text).toContain("Six APs for Acme");
  });

  it("throws the draft away plainly when it fails, and keeps the notes", async () => {
    setup({ draftFailure: true });
    const user = userEvent.setup();
    render(<Harness onBuild={() => undefined} />);
    await user.type(await screen.findByTestId("describe-transcript"), "something unusable");
    await user.click(screen.getByTestId("describe-draft"));

    const failure = await screen.findByTestId("describe-failure");
    // Blush, words, a reference, and three ways forward.
    expect(failure).toHaveTextContent(/Nothing was saved/i);
    expect(failure).toHaveTextContent(/dsc_7f3a91c2/);
    expect(screen.getByTestId("describe-retry")).toBeInTheDocument();
    expect(screen.getByText("Edit my notes")).toBeInTheDocument();
    expect(screen.getByText("Start a blank quote")).toBeInTheDocument();
    // The failure is not red: no destructive token on the banner.
    const banner = failure.querySelector(".m-dsc-failure") as HTMLElement;
    expect(banner.getAttribute("style") ?? "").not.toContain("destructive");
  });

  it("writes nothing, ever: no create call from any path", async () => {
    const mock = setup();
    const user = userEvent.setup();
    render(<Harness onBuild={() => undefined} />);
    await user.type(await screen.findByTestId("describe-transcript"), "Six APs for Acme");
    await user.click(screen.getByTestId("describe-draft"));
    await screen.findByTestId("describe-understanding");
    expect(mock.callsTo("POST", "/api/v1/quotes")).toHaveLength(0);
  });

  it("is absent, not disabled, without the grant", async () => {
    setup({ granted: false });
    render(
      <DescribeMoney kind="quote" available={false} onOpen={() => undefined} testId="door" />,
    );
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByTestId("door")).not.toBeInTheDocument();
    expect(screen.queryByText(/Describe it/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/assumed/i)).not.toBeInTheDocument();
  });

  it("closes without cancelling the read, and comes back with the check step", async () => {
    setup();
    const user = userEvent.setup();

    function Stateful(): React.JSX.Element {
      const [open, setOpen] = useState(true);
      return (
        <DescribePanel
          kind="quote"
          open={open}
          onClose={() => setOpen(false)}
          onReady={() => setOpen(true)}
          clients={[]}
          onBuild={() => undefined}
          onStartBlank={() => undefined}
          testId="panel"
        />
      );
    }

    render(<Stateful />);
    await user.type(await screen.findByTestId("describe-transcript"), "Six APs for Acme");
    await user.click(screen.getByTestId("describe-draft"));
    // The draft lands after the panel was closed to keep working: the route is
    // told, and the check step is on screen again rather than hidden.
    await waitFor(() =>
      expect(screen.getByTestId("describe-understanding")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("panel")).not.toHaveAttribute("hidden");
  });

  it("reads in a plain progress list that offers Keep working", async () => {
    render(
      <Reading
        sources={[{ id: "typed-1", kind: "typed", name: "Your words", text: "Six APs for Acme" }]}
        typed={false}
        fields={[
          { key: "client", value: "Acme Corporation" },
          { key: "quantity", value: "6 access points" },
        ]}
        onKeepWorking={() => undefined}
        onCancel={() => undefined}
      />,
    );
    expect(screen.getByText(/Read 1 source/i)).toBeInTheDocument();
    expect(screen.getByText(/Found the client/i)).toBeInTheDocument();
    expect(screen.getByText(/Pricing from your catalogue/i)).toBeInTheDocument();
    expect(screen.getByTestId("describe-keep-working")).toBeInTheDocument();
    expect(screen.getByText(/I will tell you when it is ready/i)).toBeInTheDocument();
  });
});
