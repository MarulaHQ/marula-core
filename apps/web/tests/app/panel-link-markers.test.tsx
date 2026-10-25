// Wave 56 lane `p56-panel-gates` (F53): the link resolver reports a target
// the caller's role may not read as `target_denied`, and the panels must SAY
// so. Before this, every panel fell back to `target_label ?? "target"`, so a
// restricted row read as an anonymous "target" — a marker the resolver set
// and nobody rendered.
//
// Purpose: each of the three link panels shows the restricted marker when
// `target_denied` is set and the resolved label when it is not. The copy is
// one string across the surfaces, and a denied row never leaks its label.
//
// Non-purpose: no geometry/visual assertions — those belong to the screenshot
// review; no backend contract testing (Rust integration covers the gates).
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import DocumentDetailPage from "../../app/documents/[id]/page";
import VaultPage from "../../app/passwords/page";
import AssetDetailPage from "../../app/assets/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

/** One mutable subject id: the three pages read `useParams().id`, and each
 * test renders exactly one of them. */
const nav = vi.hoisted(() => ({ id: "" }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: nav.id }),
  usePathname: () => "/",
}));

const ORG = ME_FIXTURE.org_id;
const SUBJECT_ID = "00000000-0000-4000-8000-0000000000d1";
const CLIENT_ID = "00000000-0000-4000-8000-0000000000c1";
const PASSWORD_ID = "00000000-0000-4000-8000-0000000000p1";
const RESTRICTED_MARKER = "restricted target — not visible to your role";

const VISIBLE_LABEL = "Acme Systems";
const HIDDEN_LABEL = "Acme domain admin";

function shell(mock: MockApi): void {
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
}

function linkRow(
  id: string,
  fromKind: string,
  toKind: string,
  targetId: string,
  label: string | null,
  denied: boolean,
) {
  return {
    id,
    org_id: ORG,
    from_kind: fromKind,
    from_id: SUBJECT_ID,
    to_kind: toKind,
    to_id: targetId,
    created_by: null,
    created_at: "2026-10-01T09:00:00Z",
    target_client_id: null,
    target_label: label,
    target_deleted: false,
    target_denied: denied,
  };
}

/** Two rows: a client the caller may read, and a vault entry the caller may
 * not — the resolver keeps the row and marks it. */
function links(fromKind: string) {
  return [
    linkRow("00000000-0000-4000-8000-0000000000e1", fromKind, "client", CLIENT_ID, VISIBLE_LABEL, false),
    linkRow("00000000-0000-4000-8000-0000000000e2", fromKind, "password", PASSWORD_ID, null, true),
  ];
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  cleanup();
});

describe("/documents/[id] — restricted link targets", () => {
  function setup(): MockApi {
    nav.id = SUBJECT_ID;
    const mock = new MockApi();
    shell(mock);
    mock.on("GET", `/api/v1/documents/${SUBJECT_ID}`, () => ({
      json: {
        id: SUBJECT_ID,
        org_id: ORG,
        title: "Incident hub",
        body: "Runbook body",
        client_id: null,
        layout_id: null,
        fields: {},
        current_version: 1,
        created_at: "2026-09-01T09:00:00Z",
        updated_at: "2026-09-02T09:00:00Z",
        change_summary: "",
        content_updated_at: "2026-09-02T09:00:00Z",
      },
    }));
    mock.on("GET", "/api/v1/documents/layouts", () => ({ json: [] }));
    mock.on("GET", "/api/v1/clients", () => ({ json: [] }));
    mock.on("GET", `/api/v1/documents/${SUBJECT_ID}/versions`, () => ({ json: [] }));
    mock.on("GET", `/api/v1/documents/${SUBJECT_ID}/links`, () => ({ json: links("document") }));
    mock.on("GET", `/api/v1/documents/${SUBJECT_ID}/attachments`, () => ({
      json: { attachments: [] },
    }));
    mock.install();
    return mock;
  }

  it("renders the marker for a denied target and the label for a readable one", async () => {
    setup();
    render(
      <AuthProvider>
        <DocumentDetailPage />
      </AuthProvider>,
    );

    await waitFor(() => {
      expect(screen.getByText(VISIBLE_LABEL)).toBeTruthy();
    });
    expect(screen.getAllByText(RESTRICTED_MARKER)).toHaveLength(1);
    expect(screen.queryByText(HIDDEN_LABEL)).toBeNull();
    expect(screen.queryByText("target")).toBeNull();
  });
});

describe("/passwords — restricted link targets", () => {
  const FOLDER_ID = "00000000-0000-4000-8000-0000000000f1";
  const RECORD_ID = "00000000-0000-4000-8000-0000000000r1";

  const FOLDER = {
    id: FOLDER_ID,
    org_id: ORG,
    client_id: null,
    parent_id: null,
    slug: "servers",
    label: "Servers",
    kind: "passwords",
    is_default_seed: false,
    restricted: false,
    forked_from: null,
    created_by: null,
    updated_by: null,
    created_at: "2026-09-01T09:00:00Z",
    updated_at: "2026-09-01T09:00:00Z",
  };

  const RECORD = {
    id: RECORD_ID,
    org_id: ORG,
    client_id: null,
    folder_id: FOLDER_ID,
    kind: "password",
    title: "Domain admin",
    expires_at: null,
    rotate_at: null,
    last_rotated_at: null,
    object_key: null,
    file_sha256: null,
    file_bytes: null,
    fields: [],
    created_by: null,
    updated_by: null,
    created_at: "2026-09-01T09:00:00Z",
    updated_at: "2026-09-01T09:00:00Z",
  };

  function setup(): MockApi {
    nav.id = "";
    const mock = new MockApi();
    shell(mock);
    mock.on("GET", "/api/v1/passwords", () => ({ json: [] }));
    mock.on("GET", "/api/v1/clients", () => ({ json: [] }));
    mock.on("GET", "/api/v1/secrets/folders", () => ({ json: [FOLDER] }));
    mock.on("GET", "/api/v1/secrets/records", () => ({ json: [RECORD] }));
    mock.on("GET", `/api/v1/secrets/records/${RECORD_ID}`, () => ({ json: RECORD }));
    mock.on("GET", `/api/v1/secrets/records/${RECORD_ID}/links`, () => ({
      json: links("secret"),
    }));
    mock.install();
    return mock;
  }

  it("renders the marker for a denied target and the label for a readable one", async () => {
    setup();
    render(
      <AuthProvider>
        <VaultPage />
      </AuthProvider>,
    );

    // Category → record → the record dialog's Links card.
    await waitFor(() => {
      expect(screen.getByText("Servers")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("Servers"));
    await waitFor(() => {
      expect(screen.getByText("Domain admin")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("Domain admin"));

    await waitFor(() => {
      expect(screen.getByText(VISIBLE_LABEL)).toBeTruthy();
    });
    expect(screen.getAllByText(RESTRICTED_MARKER)).toHaveLength(1);
    expect(screen.queryByText(HIDDEN_LABEL)).toBeNull();
  });
});

describe("/assets/[id] — restricted link targets", () => {
  const ASSET = {
    id: SUBJECT_ID,
    org_id: ORG,
    client_id: null,
    scope: "internal_org",
    site_id: null,
    tag: "EXCH-01",
    name: "Exchange server",
    kind: "device",
    status: "active",
    layout_id: null,
    fields: {},
    ownership: "owned",
    rental_vendor: null,
    rental_start: null,
    rental_end: null,
    rental_cost_minor: null,
    rental_currency: null,
    rental_cycle: null,
    purchase_date: null,
    purchase_cost_minor: null,
    purchase_currency: null,
    purchase_fx_rate_used: null,
    purchase_fx_rate_date: null,
    purchase_fx_source: null,
    base_purchase_cost_minor: null,
    depreciation_method: "none",
    warranty: null,
    useful_life_months: null,
    salvage_value_minor: null,
    depreciation_rate_bps: null,
    identity_keys: {},
    observed_fields: {},
    curated_fields: {},
    field_sources: {},
    merged_into_id: null,
    created_at: "2026-09-01T09:00:00Z",
    updated_at: "2026-09-02T09:00:00Z",
  };

  function setup(): MockApi {
    nav.id = SUBJECT_ID;
    const mock = new MockApi();
    shell(mock);
    mock.on("GET", `/api/v1/assets/${SUBJECT_ID}`, () => ({ json: ASSET }));
    mock.on("GET", "/api/v1/documents/layouts", () => ({ json: [] }));
    mock.on("GET", `/api/v1/assets/${SUBJECT_ID}/links`, () => ({ json: links("asset") }));
    mock.on("GET", `/api/v1/assets/${SUBJECT_ID}/vault`, () => ({ json: [] }));
    mock.on("GET", "/api/v1/assets/integration-links", () => ({ json: [] }));
    mock.install();
    return mock;
  }

  it("renders the marker for a denied target and the label for a readable one", async () => {
    setup();
    render(
      <AuthProvider>
        <AssetDetailPage />
      </AuthProvider>,
    );

    await waitFor(() => {
      expect(screen.getByText(VISIBLE_LABEL)).toBeTruthy();
    });
    expect(screen.getAllByText(RESTRICTED_MARKER)).toHaveLength(1);
    expect(screen.queryByText(HIDDEN_LABEL)).toBeNull();
    // The remove button never reads as a resolved label for a denied row.
    expect(screen.getByLabelText("Remove restricted link")).toBeTruthy();
  });
});
