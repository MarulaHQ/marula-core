// AiSetupStep — the org wizard's AI setup step (Wave 72 lane W72-1).
//
// The load-bearing assertions are the CONTRACT, not the words:
//   • the gate — a member without owner/admin standing sees one honest
//     sentence and no provider cards; the seal surface is owner/admin
//     server-side and the client gate says so before the API has to;
//   • presence-only keys — a saved key is never rendered back anywhere:
//     the input clears, no GET renders a key, the card reads "Key sealed";
//   • the honest failure — a refused test renders the server's message
//     verbatim in the danger pair, never a generic "something went wrong";
//   • unconfigured providers are not selectable defaults — a catalog pair
//     whose provider has no key stays visible but disabled;
//   • the skip persists nothing — "Not now" records no mutation at all;
//   • Done completes the step only while the org master switch is on (the
//     server gate mirrors ai.features_enabled); with AI off the save stands
//     and the exit records a skip, never a 409;
//   • the chat default rides the org pick (model-picks/general_chat), the
//     decision default rides the per-surface pin, and the report names
//     what saved and what did not.
//
// Non-purpose: no backend bundling (MockApi fabricates the wire shapes;
// the routes' contract tests live in the Rust suites).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { AiSetupStep } from "../../components/onboarding/ai-setup-step";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

// useRequireAuth pulls useRouter, which needs the app router a test render
// does not mount — the same stub every page-level test in this suite installs.
const push = vi.fn();
const replace = vi.fn();
vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace, push }),
  usePathname: () => "/",
}));

const ADMIN_ME = {
  ...ME_FIXTURE,
  permissions: [...ME_FIXTURE.permissions, "special:manage_users", "admin:edit"],
};

type StepProps = Parameters<typeof AiSetupStep>[0];

/** Seeds tokens, installs the MockApi (with the admin /me answer), runs the
 *  caller's route seeds, THEN renders — so no fetch can race a registration. */
function renderStep(seed?: (mock: MockApi) => void, props?: Partial<StepProps>): {
  mock: MockApi;
} {
  seedAuthTokens();
  const mock = new MockApi().install();
  mock.on("GET", "/api/v1/me", () => ({ json: ADMIN_ME }));
  seed?.(mock);
  render(
    <AuthProvider>
      <AiSetupStep
        onBack={props?.onBack ?? (() => {})}
        backDisabled={props?.backDisabled ?? false}
        onSkip={props?.onSkip ?? (() => {})}
        onDone={props?.onDone ?? (() => {})}
      />
    </AuthProvider>,
  );
  return { mock };
}

/** The three reads the step opens with, over an org with nothing set up. */
function seedEmptyOrg(mock: MockApi): void {
  mock.on("GET", "/api/v1/settings/ai/status", () => ({
    json: { active_provider: "none", active_model: "none", configured_providers: [] },
  }));
  mock.on("GET", "/api/v1/settings/ai/matrix", () => ({
    json: {
      caps: [],
      catalog: [],
      credentials: [],
      grants: [],
      personal_credentials_enabled: false,
      surfaces: [],
    },
  }));
  mock.on("GET", "/api/v1/settings/ai/decision", () => ({
    json: { features_enabled: false, mode: "off", surfaces: [] },
  }));
  mock.on("GET", "/api/v1/ai/me/surfaces", () => ({
    json: { surfaces: [], windows: [null, null] },
  }));
}

beforeEach(() => {
  window.localStorage.clear();
});

function seedCatalog(mock: MockApi, configured: string[]): void {
  seedEmptyOrg(mock);
  mock.on("GET", "/api/v1/settings/ai/status", () => ({
    json: { active_provider: "none", active_model: "none", configured_providers: configured },
  }));
  mock.on("GET", "/api/v1/settings/ai/matrix", () => ({
    json: {
      caps: [],
      catalog: [
        {
          id: "c1",
          provider: "openai",
          model: "gpt-4o-mini",
          label: "gpt-4o-mini",
          cost_weight: 1,
          enabled: true,
          capabilities: ["completion"],
        },
        {
          id: "c2",
          provider: "typesafe",
          model: "jev-1.13.0",
          label: "jev-1.13.0",
          cost_weight: 1,
          enabled: true,
          capabilities: ["decision"],
        },
        {
          id: "c3",
          provider: "ollama",
          model: "llama3",
          label: "llama3",
          cost_weight: 1,
          enabled: true,
          capabilities: ["completion"],
        },
      ],
      credentials: [],
      grants: [],
      personal_credentials_enabled: false,
      surfaces: [],
    },
  }));
}

describe("AiSetupStep gate", () => {
  it("shows the owner/admin sentence and no cards to a non-admin", async () => {
    seedAuthTokens();
    const mock = new MockApi().install();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    seedEmptyOrg(mock);
    const onSkip = vi.fn();
    render(
      <AuthProvider>
        <AiSetupStep onBack={() => {}} backDisabled={false} onSkip={onSkip} onDone={() => {}} />
      </AuthProvider>,
    );
    const gate = await screen.findByTestId("ai-setup-gate");
    expect(gate.textContent).toContain("owner/admin");
    expect(screen.queryByTestId("ai-setup-chat-cards")).toBeNull();
    // The exit works from the gate too.
    fireEvent.click(screen.getByTestId("ai-setup-notnow"));
    expect(onSkip).toHaveBeenCalledOnce();
    // And the gate recorded nothing.
    expect(mock.calls.filter((c) => c.method !== "GET")).toHaveLength(0);
  });
});

describe("AiSetupStep providers", () => {
  it("renders the backed providers and the honest disabled ones", async () => {
    const { } = renderStep(seedEmptyOrg);
    await screen.findByTestId("ai-setup");
    for (const id of ["openai", "anthropic", "ollama", "custom"]) {
      expect(screen.getByTestId(`ai-setup-provider-${id}`)).toBeInTheDocument();
    }
    // The gap providers are visible but disabled, with the honest line.
    for (const id of ["gemini", "openrouter"]) {
      const card = screen.getByTestId(`ai-setup-provider-${id}`) as HTMLButtonElement;
      expect(card.disabled).toBe(true);
      expect(card.textContent).toContain("Not available yet");
    }
    // The decision engine names Jev; the rest of the brief's list is visible
    // but disabled for the same honest reason.
    expect(screen.getByTestId("ai-setup-decision-typesafe")).toBeInTheDocument();
    for (const id of ["openrouter", "openai_decisions", "gemini_decisions", "custom_decisions"]) {
      const card = screen.getByTestId(`ai-setup-decision-${id}`) as HTMLButtonElement;
      expect(card.disabled).toBe(true);
    }
  });

  it("reveals a masked key inline with its provider, and the draft never renders back once saved", async () => {
    const { mock } = renderStep((m) => {
      seedEmptyOrg(m);
      m.on("PUT", "/api/v1/settings/ai/byok", (ctx) => {
        const body = ctx.body as { provider: string; api_key: string; idempotency_key: string };
        expect(body.provider).toBe("openai");
        expect(body.api_key).toBe("sk-test-123");
        expect(typeof body.idempotency_key).toBe("string");
        return {
          json: { status: { provider: body.provider, has_key: true, updated_at: null }, mutation: { version: 1 } },
        };
      });
      m.on("POST", "/api/v1/settings/ai/test", () => ({
        json: { provider: "openai", ok: true, message: "credentials verified and provider ready", latency_ms: 12 },
      }));
    });

    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-provider-openai"));
    const input = screen.getByTestId("ai-setup-key-input") as HTMLInputElement;
    expect(input.type).toBe("password");
    fireEvent.change(input, { target: { value: "sk-test-123" } });
    // The reveal toggle flips the input type and nothing else.
    fireEvent.click(screen.getByTestId("ai-setup-key-toggle"));
    expect((screen.getByTestId("ai-setup-key-input") as HTMLInputElement).type).toBe("text");
    fireEvent.click(screen.getByTestId("ai-setup-key-save"));

    // Presence only: the draft is gone, the key text appears nowhere.
    await waitFor(() =>
      expect((screen.getByTestId("ai-setup-key-input") as HTMLInputElement).value).toBe(""),
    );
    expect(document.body.textContent).not.toContain("sk-test-123");
    // The test-connection ran on its own and reported honestly.
    await screen.findByText(/Connected: credentials verified and provider ready/);
    // Deselect the card: presence now reads as a badge, not a key.
    fireEvent.click(screen.getByTestId("ai-setup-provider-openai"));
    await waitFor(() =>
      expect(screen.getByTestId("ai-setup-provider-openai").textContent).toContain("Key sealed"),
    );
  });

  it("renders the honest failure message verbatim when the test fails", async () => {
    const { } = renderStep((m) => {
      seedEmptyOrg(m);
      m.on("PUT", "/api/v1/settings/ai/byok", () => ({
        json: { status: { provider: "openai", has_key: true, updated_at: null }, mutation: { version: 1 } },
      }));
      m.on("POST", "/api/v1/settings/ai/test", () => ({
        json: {
          provider: "openai",
          ok: false,
          message: "stored secret key failed decryption (tampered or rotated)",
          latency_ms: null,
        },
      }));
    });

    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-provider-openai"));
    fireEvent.change(screen.getByTestId("ai-setup-key-input"), { target: { value: "sk-bad" } });
    fireEvent.click(screen.getByTestId("ai-setup-key-save"));
    await screen.findByText(/Failed: stored secret key failed decryption/);
  });
});

describe("AiSetupStep defaults", () => {

  it("keeps a default pointing at an unconfigured provider unselectable", async () => {
    renderStep((m) => seedCatalog(m, []));
    await screen.findByTestId("ai-setup");
    const openaiRow = screen.getByTestId(
      "ai-setup-chat-default-openai-gpt-4o-mini",
    ) as HTMLButtonElement;
    expect(openaiRow.disabled).toBe(true);
    expect(openaiRow.textContent).toContain("provider not configured");
    // Ollama needs no key, so its pair is selectable on a bare org.
    const ollamaRow = screen.getByTestId("ai-setup-chat-default-ollama-llama3") as HTMLButtonElement;
    expect(ollamaRow.disabled).toBe(false);
    // The decision pair is visible but holds the credential note.
    const jevRow = screen.getByTestId(
      "ai-setup-decision-default-typesafe-jev-1.13.0",
    ) as HTMLButtonElement;
    expect(jevRow.disabled).toBe(true);
    expect(jevRow.textContent).toContain("no credential yet");
  });

  it("saves the chat default through the org pick and reports honestly", async () => {
    let picked: { provider: string; model: string } | null = null;
    const onDone = vi.fn();
    const onSkip = vi.fn();
    renderStep(
      (m) => {
        seedCatalog(m, []);
        m.on("PUT", "/api/v1/settings/ai/model-picks/general_chat", (ctx) => {
          picked = ctx.body as { provider: string; model: string };
          return {
            json: { mutation: { version: 1 }, pick: { surface_key: "general_chat", scope: "org" } },
          };
        });
      },
      { onDone, onSkip },
    );
    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-chat-default-ollama-llama3"));
    fireEvent.click(screen.getByTestId("ai-setup-save"));

    await screen.findByTestId("ai-setup-report");
    expect(picked).toEqual({ provider: "ollama", model: "llama3", idempotency_key: expect.any(String) });
    expect(screen.getByTestId("ai-setup-report").textContent).toContain(
      "Chat default set to ollama / llama3.",
    );
    // AI is off in this org, so the server would refuse to complete the
    // step. The report says so, and the exit records the skip.
    expect(screen.getByTestId("ai-setup-report").textContent).toContain("AI stays off");
    fireEvent.click(screen.getByTestId("ai-setup-done"));
    expect(onSkip).toHaveBeenCalledOnce();
    expect(onDone).not.toHaveBeenCalled();
  });

  it("writes the org pick even when the operator's personal pick matches it", async () => {
    let wrote = false;
    renderStep((m) => {
      seedCatalog(m, []);
      m.on("GET", "/api/v1/ai/me/surfaces", () => ({
        json: {
          surfaces: [
            { surface_key: "general_chat", picked: { provider: "ollama", model: "llama3" } },
          ],
          windows: [null, null],
        },
      }));
      m.on("PUT", "/api/v1/settings/ai/model-picks/general_chat", () => {
        wrote = true;
        return { json: { mutation: { version: 1 } } };
      });
    });
    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-chat-default-ollama-llama3"));
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    await screen.findByTestId("ai-setup-report");
    // The skip-on-match guard compared against the PERSONAL pick, which
    // never reflects the org default, so the write was silently dropped.
    expect(wrote).toBe(true);
  });

  it("completes the step when AI is on", async () => {
    const onDone = vi.fn();
    renderStep(
      (m) => {
        seedEmptyOrg(m);
        m.on("GET", "/api/v1/settings/ai/decision", () => ({
          json: { features_enabled: true, mode: "off", surfaces: [] },
        }));
      },
      { onDone },
    );
    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    await screen.findByTestId("ai-setup-report");
    fireEvent.click(screen.getByTestId("ai-setup-done"));
    expect(onDone).toHaveBeenCalledOnce();
  });

  it("treats a single failed read as recoverable, never an endless spinner", async () => {
    renderStep((m) => {
      seedEmptyOrg(m);
      m.on("GET", "/api/v1/settings/ai/decision", () => {
        throw new Error("decision read down");
      });
    });
    const errorCard = await screen.findByText(/could not load/);
    expect(errorCard).toBeInTheDocument();
    expect(screen.getByText("Try again")).toBeInTheDocument();
    expect(screen.getAllByTestId("ai-setup-notnow").length).toBeGreaterThan(0);
  });

  it("pins the decision default on every non-forked decision surface", async () => {
    const pinned: string[] = [];
    renderStep((m) => {
      seedEmptyOrg(m);
      m.on("GET", "/api/v1/settings/ai/matrix", () => ({
        json: {
          caps: [],
          catalog: [
            {
              id: "c2",
              provider: "typesafe",
              model: "jev-1.13.0",
              label: "jev-1.13.0",
              cost_weight: 1,
              enabled: true,
              capabilities: ["decision"],
            },
          ],
          credentials: [
            {
              id: "cred1",
              provider: "typesafe",
              kind: "api_key",
              label: "Jev",
              owner_user_id: null,
              status: "active",
              credential_meta: {},
            },
          ],
          grants: [],
          personal_credentials_enabled: false,
          surfaces: [],
        },
      }));
      m.on("GET", "/api/v1/settings/ai/decision", () => ({
        json: {
          features_enabled: false,
          mode: "off",
          surfaces: [
            {
              surface_key: "decision_ticket_triage",
              label: "Ticket triage decisions",
              description: "",
              mode: "shadow",
              enabled: false,
              configured: false,
              customized: false,
              decision_model_curated: true,
              internal: false,
              starter_version: 1,
              latest_starter_version: 1,
              model_provider: "openai",
              model_id: "gpt-4o-mini",
            },
            {
              surface_key: "decision_dispatch",
              label: "Dispatch decisions",
              description: "",
              mode: "shadow",
              enabled: false,
              configured: false,
              customized: true,
              decision_model_curated: true,
              internal: false,
              starter_version: 1,
              latest_starter_version: 1,
              model_provider: "anthropic",
              model_id: "claude-3",
            },
          ],
        },
      }));
      m.on("PUT", "/api/v1/settings/ai/decision/decision_ticket_triage/config", (ctx) => {
        const body = ctx.body as { model_provider: string; model_id: string };
        expect(body.model_provider).toBe("typesafe");
        expect(body.model_id).toBe("jev-1.13.0");
        pinned.push("decision_ticket_triage");
        return { json: { mutation: { version: 1 }, status: {} } };
      });
    });
    await screen.findByTestId("ai-setup");
    // The decision pair is eligible now that the org credential row exists.
    const jevRow = screen.getByTestId(
      "ai-setup-decision-default-typesafe-jev-1.13.0",
    ) as HTMLButtonElement;
    expect(jevRow.disabled).toBe(false);
    // The forked surface already carries its own pinned pair, so the row
    // opens on it as the org's de facto default.
    expect(screen.getByTestId("ai-setup-decision-current").textContent).toContain("openai/gpt-4o-mini");
    fireEvent.click(jevRow);
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    await screen.findByTestId("ai-setup-report");
    expect(pinned).toEqual(["decision_ticket_triage"]);
    expect(screen.getByTestId("ai-setup-report").textContent).toContain("1 forked decision surface keeps");
  });
});

describe("AiSetupStep exits", () => {
  it("'Not now' persists nothing", async () => {
    const onSkip = vi.fn();
    const { mock } = renderStep(
      (m) => {
        seedEmptyOrg(m);
      },
      { onSkip },
    );
    await screen.findByTestId("ai-setup");
    // Even with a provider picked and a key typed, the skip seals nothing.
    fireEvent.click(screen.getByTestId("ai-setup-provider-openai"));
    fireEvent.change(screen.getByTestId("ai-setup-key-input"), { target: { value: "sk-never-saved" } });
    const notNow = screen.getAllByTestId("ai-setup-notnow")[0];
    if (notNow === undefined) throw new Error("no Not-now button rendered");
    fireEvent.click(notNow);
    expect(onSkip).toHaveBeenCalledOnce();
    expect(mock.calls.filter((c) => c.method === "PUT" || c.method === "POST")).toHaveLength(0);
    expect(document.body.textContent).not.toContain("sk-never-saved");
  });

  it("gives each radio group one keyboard entry point, selection following focus", async () => {
    // Two eligible chat defaults (an OpenAI key is sealed, Ollama needs
    // none), so the arrow has somewhere real to go.
    renderStep((m) => seedCatalog(m, ["openai"]));
    await screen.findByTestId("ai-setup");

    // Nothing selected yet: the first backed decision provider carries the
    // group's single tab stop (the bug had every button at -1).
    const jev = screen.getByTestId("ai-setup-decision-typesafe") as HTMLButtonElement;
    expect(jev.tabIndex).toBe(0);

    const group = screen.getByTestId("ai-setup-chat-defaults");
    const radios = Array.from(
      group.querySelectorAll<HTMLButtonElement>('[role="radio"]:not([disabled])'),
    );
    expect(radios.length).toBe(2);
    const first = radios[0];
    const second = radios[1];
    if (first === undefined || second === undefined) throw new Error("two radios expected");

    first.focus();
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: "ArrowDown" });

    // Focus moved AND selection followed; a missing handler fails this.
    expect(document.activeElement).toBe(second);
    expect(second.getAttribute("aria-checked")).toBe("true");
  });

  it("drops a rendered report as soon as a pending value changes", async () => {
    renderStep((m) => seedCatalog(m, []));
    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    await screen.findByTestId("ai-setup-report");

    // Editing the switch after the report must return the primary to Save:
    // Done would otherwise skip the unsaved change.
    fireEvent.click(screen.getByTestId("ai-setup-master-switch"));
    await waitFor(() => expect(screen.queryByTestId("ai-setup-report")).toBeNull());
    expect(screen.getByTestId("ai-setup-save")).toBeInTheDocument();
  });

  it("treats an inline-sealed provider as sealed, not as a missing key", async () => {
    renderStep((m) => {
      seedEmptyOrg(m);
      m.on("GET", "/api/v1/settings/ai/status", () => ({
        json: { active_provider: "openai", active_model: "none", configured_providers: ["openai"] },
      }));
    });
    await screen.findByTestId("ai-setup");
    // Pick the provider whose key is already sealed and leave the draft empty.
    fireEvent.click(screen.getByTestId("ai-setup-provider-openai"));
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    const report = await screen.findByTestId("ai-setup-report");
    expect(report.textContent).toContain("already sealed");
    expect(report.textContent).not.toContain("no key was pasted");
  });

  it("offers a retry when the report carries a failed outcome", async () => {
    renderStep((m) => {
      seedCatalog(m, []);
      m.on("PUT", "/api/v1/settings/ai/model-picks/general_chat", () => {
        throw new Error("org pick refused");
      });
    });
    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-chat-default-ollama-llama3"));
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    const report = await screen.findByTestId("ai-setup-report");
    expect(report.textContent).toContain("was refused");
    expect(screen.getByTestId("ai-setup-done").textContent).toContain("Save again");
  });

  it("reports nothing-picked honestly and, with AI off, records the skip", async () => {
    const onDone = vi.fn();
    const onSkip = vi.fn();
    renderStep(seedEmptyOrg, { onDone, onSkip });
    await screen.findByTestId("ai-setup");
    fireEvent.click(screen.getByTestId("ai-setup-save"));
    await screen.findByTestId("ai-setup-report");
    expect(screen.getByTestId("ai-setup-report").textContent).toContain(
      "Nothing was picked, so nothing was saved.",
    );
    fireEvent.click(screen.getByTestId("ai-setup-done"));
    expect(onSkip).toHaveBeenCalledOnce();
    expect(onDone).not.toHaveBeenCalled();
  });
});
