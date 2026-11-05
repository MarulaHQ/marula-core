// Accepting an authored draft saves the card with the spec (F103).
//
// Purpose: pin the one thing the authoring page promises after the card became
// storable — an accepted draft sends its card on create AND on the full-replace
// update, unmodified, so the window, the refresh and the size the reviewer
// approved are what the definition keeps. Before this, the accept path sent the
// spec alone and the card was silently dropped.
//
// Non-purpose: no rendering (the page suite covers the row) and no backend
// contract (the Rust flow suite covers storage and validation).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { acceptReportDraft, type ReportDraft } from "../../lib/report-authoring";

const POST = vi.fn();
const PUT = vi.fn();

vi.mock("../../lib/api", () => ({
  api: {
    api: {
      POST: (...args: unknown[]) => POST(...args),
      PUT: (...args: unknown[]) => PUT(...args),
    },
  },
  apiBaseUrl: () => "http://api.test",
  loadTokens: () => ({
    access_token: "test-token",
    refresh_token: "test-refresh",
    session_id: "test-session",
    expires_in: 3600,
  }),
}));

function draft(): ReportDraft {
  return {
    name: "Tickets by status",
    spec: {
      view: "tickets",
      filters: [],
      group_by: ["status_name"],
      measures: [{ agg: "count" }],
      viz: "bar",
    },
    card: {
      size: { span: "half", height: "tall" },
      window_days: 7,
      refresh: { every_minutes: 60, reach_days: 0 },
    },
    shape: "chart",
    reads: "Tickets by status over the last seven days.",
  };
}

beforeEach(() => {
  POST.mockReset();
  PUT.mockReset();
  POST.mockResolvedValue({ data: { id: "definition-new" }, error: undefined });
  PUT.mockResolvedValue({ data: { id: "definition-existing" }, error: undefined });
});

describe("acceptReportDraft", () => {
  it("sends the card on create", async () => {
    const authored = draft();
    const outcome = await acceptReportDraft({ draft: authored, name: "Open tickets" });

    expect(POST).toHaveBeenCalledTimes(1);
    const [path, init] = POST.mock.calls[0] as [string, { body: Record<string, unknown> }];
    expect(path).toBe("/api/v1/report-definitions");
    expect(init.body.card).toEqual(authored.card);
    expect(init.body.spec).toEqual(authored.spec);
    expect(outcome).toEqual({ ok: true, id: "definition-new", created: true });
  });

  it("sends the card on the full-replace update", async () => {
    const authored = draft();
    const outcome = await acceptReportDraft({
      draft: authored,
      name: "Open tickets",
      currentId: "definition-existing",
    });

    expect(PUT).toHaveBeenCalledTimes(1);
    const [path, init] = PUT.mock.calls[0] as [
      string,
      { params: { path: { id: string } }; body: Record<string, unknown> },
    ];
    expect(path).toBe("/api/v1/report-definitions/{id}");
    expect(init.params.path.id).toBe("definition-existing");
    expect(init.body.card).toEqual(authored.card);
    expect(outcome).toEqual({ ok: true, id: "definition-existing", created: false });
  });
});
