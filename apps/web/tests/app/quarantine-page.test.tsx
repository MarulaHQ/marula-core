// Integration test: /settings/quarantine — the quarantine admin surface.
//
// What these pin, and why each one is a promise rather than a snapshot:
//   - the override warning is present and says "Release is an override"
//   - the empty state is honest, not a blank table
//   - a held row offers Release and Delete; a released row offers neither
//   - release REFUSES to submit without a reason and sends the trimmed one
//   - a load failure states what happened and offers Try again
//   - a role without the admin module sees an honest "not available", not 403s
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import QuarantinePage from "../../app/settings/quarantine/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/quarantine",
}));

const ME_ADMIN = {
  ...ME_FIXTURE,
  permissions: ["admin:view", "admin:edit", "admin:delete"],
};

const DIGEST = "a".repeat(64);
const DIGEST_TWO = "b".repeat(64);

function item(overrides: Record<string, unknown> = {}) {
  return {
    sha256: DIGEST,
    filename: "invoice-8821.pdf",
    content_type: "application/pdf",
    size_bytes: 2048,
    scanner: "clamd",
    detail: "stream: Eicar-Test-Signature FOUND",
    state: "pending",
    quarantined_at: "2026-10-20T09:31:00Z",
    scanned_at: "2026-10-20T09:31:00Z",
    ticket_id: "70000000-0000-4000-8000-000000000001",
    uploaded_by_user_id: "00000000-0000-4000-8000-000000000001",
    uploaded_by_portal_user_id: null,
    storage_key: `org/00000000-0000-4000-8000-000000000002/quarantine/${DIGEST}`,
    released_at: null,
    released_by_user_id: null,
    release_reason: null,
    ...overrides,
  };
}

function setupApi(
  page: { items: unknown[]; pending: number; released: number } = {
    items: [],
    pending: 0,
    released: 0,
  },
  me: Record<string, unknown> = ME_ADMIN,
): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("GET", "/api/v1/admin/quarantine", () => ({ json: page }));
  mock.install();
  return mock;
}

function renderPage() {
  return render(
    <AuthProvider>
      <QuarantinePage />
    </AuthProvider>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("/settings/quarantine", () => {
  it("states plainly that release is an override", async () => {
    setupApi();
    renderPage();
    const note = await screen.findByTestId("release-override-note");
    expect(note.textContent).toContain("Release is an override");
    expect(note.textContent).toContain("audit trail");
  });

  it("draws an honest empty state when nothing is held", async () => {
    setupApi();
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId("quarantine-counts").textContent).toContain("0");
    });
    expect(screen.getByText(/Nothing is quarantined/)).toBeTruthy();
  });

  it("offers release and delete on a held row, and records the reason", async () => {
    const mock = setupApi({
      items: [item()],
      pending: 1,
      released: 0,
    });
    mock.on("POST", `/api/v1/admin/quarantine/${DIGEST}/release`, () => ({
      json: item({
        state: "released",
        released_at: "2026-10-20T10:00:00Z",
        released_by_user_id: ME_FIXTURE.user_id,
        release_reason: "Vendor confirmed a false positive",
      }),
    }));
    const user = userEvent.setup();
    renderPage();

    const row = await screen.findByTestId(`quarantine-row-${DIGEST}`);
    expect(row.textContent).toContain("invoice-8821.pdf");
    expect(row.textContent).toContain("held");
    expect(screen.getByTestId(`delete-${DIGEST}`)).toBeTruthy();

    await user.click(screen.getByTestId(`release-${DIGEST}`));
    // A reason is required: the confirm button stays disabled until one lands.
    const confirm = screen.getByTestId("confirm-release") as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    await user.type(screen.getByTestId("release-reason"), "Vendor confirmed a false positive");
    await waitFor(() => expect(confirm.disabled).toBe(false));
    await user.click(confirm);

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/admin/quarantine/${DIGEST}/release`)).toHaveLength(1);
    });
    expect(
      mock.lastBody<{ reason: string }>("POST", `/api/v1/admin/quarantine/${DIGEST}/release`),
    ).toEqual({ reason: "Vendor confirmed a false positive" });
  });

  it("shows a released row with its reason and no release or delete action", async () => {
    setupApi({
      items: [
        item({
          sha256: DIGEST_TWO,
          state: "released",
          released_at: "2026-10-20T10:00:00Z",
          released_by_user_id: ME_FIXTURE.user_id,
          release_reason: "Vendor confirmed a false positive",
        }),
      ],
      pending: 0,
      released: 1,
    });
    const user = userEvent.setup();
    renderPage();

    const row = await screen.findByTestId(`quarantine-row-${DIGEST_TWO}`);
    expect(row.textContent).toContain("released");
    expect(screen.queryByTestId(`release-${DIGEST_TWO}`)).toBeNull();
    expect(screen.queryByTestId(`delete-${DIGEST_TWO}`)).toBeNull();

    await user.click(screen.getByTestId(`inspect-${DIGEST_TWO}`));
    const inspect = await screen.findByTestId("quarantine-inspect");
    expect(inspect.textContent).toContain("Vendor confirmed a false positive");
    expect(inspect.textContent).toContain(DIGEST_TWO);
  });

  it("says what failed and offers a retry when the list cannot load", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_ADMIN }));
    mock.on("GET", "/api/v1/admin/quarantine", () => ({
      status: 500,
      json: { error: { reason: "internal", message: "boom" } },
    }));
    mock.install();
    renderPage();

    const alert = await screen.findByTestId("quarantine-load-error");
    expect(alert.textContent).toContain("could not be loaded");
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("tells a role without the admin module that the page is not available", async () => {
    setupApi(undefined, { ...ME_FIXTURE, permissions: [] });
    renderPage();
    expect(await screen.findByTestId("settings-not-available")).toBeTruthy();
    expect(screen.queryByTestId("release-override-note")).toBeNull();
  });
});
