// Documentation completeness scoring — pure metadata scoring (lane 2).
import { describe, expect, it } from "vitest";
import { clientCompleteness, scoreTone } from "../../lib/docs-completeness";
import type { components } from "@meridian/sdk";

type Document = components["schemas"]["Document"];
type Site = components["schemas"]["Site"];
type Contact = components["schemas"]["Contact"];
type DeviceAsset = components["schemas"]["DeviceAsset"];
type PortalUser = components["schemas"]["PortalUser"];
type DocumentLayout = components["schemas"]["DocumentLayout"];

function doc(overrides: Partial<Document> = {}): Document {
  return {
    id: "11111111-1111-4111-8111-111111111111",
    org_id: "o",
    title: "Network manifest",
    client_id: "c",
    layout_id: "22222222-2222-4222-8222-222222222222",
    current_version: 1,
    fields: {},
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    created_by: null,
    updated_by: null,
    ...overrides,
  };
}

function layout(id: string): DocumentLayout {
  return {
    id,
    org_id: "o",
    name: "Network",
    slug: "network-" + id.slice(0, 6),
    icon: "network",
    description: "",
    is_system: true,
    fields: [],
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    semantic_class: null,
  };
}

function site(): Site {
  return {
    id: "33333333-3333-4333-8333-333333333333",
    org_id: "o",
    client_id: "c",
    name: "HQ",
    address: null,
    city: null,
    country: null,
    region: null,
    postal_code: null,
    latitude: null,
    longitude: null,
    created_at: new Date().toISOString(),
  };
}

function contact(primary: boolean, email: string | null): Contact {
  return {
    id: "44444444-4444-4444-8444-44444444444" + (primary ? "1" : "2"),
    org_id: "o",
    client_id: "c",
    name: primary ? "Primary" : "Other",
    email: email ?? undefined,
    phone: null,
    title: null,
    is_primary: primary,
    created_at: new Date().toISOString(),
  };
}

function asset(): DeviceAsset {
  return {
    id: "55555555-5555-4555-8555-555555555551",
    org_id: "o",
    client_id: "c",
    tag: "LAP-001",
    name: "Laptop",
    status: "active",
    site_id: null,
    layout_id: null,
    fields: {},
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    kind: "device",
  };
}

function portalUser(): PortalUser {
  return {
    id: "66666666-6666-4666-8666-666666666661",
    org_id: "o",
    client_id: "c",
    email: "p@x.test",
    display_name: "Portal",
    role: "contact",
    status: "invited",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  };
}

describe("clientCompleteness", () => {
  it("scores an empty client 0 with every gap named", () => {
    const r = clientCompleteness({ sites: [], contacts: [], assets: [], documents: [], layouts: [], portalUsers: [] });
    expect(r.score).toBe(0);
    const keys = r.gaps.map((g) => g.key);
    expect(keys).toContain("contacts");
    expect(keys).toContain("sites");
    expect(keys).toContain("documents");
    expect(keys).toContain("assets");
    expect(keys).toContain("portal");
    expect(r.basis).toContain("0 site(s)");
  });

  it("scores a fully documented, fresh client at 100", () => {
    const l = layout("22222222-2222-4222-8222-222222222222");
    const fresh = new Date();
    const r = clientCompleteness({
      sites: [site()],
      contacts: [contact(true, "p@x.test")],
      assets: [asset()],
      documents: [doc({ updated_at: fresh.toISOString() })],
      layouts: [l],
      portalUsers: [portalUser()],
    });
    expect(r.score).toBe(100);
    expect(r.gaps).toEqual([]);
  });

  it("scores an untyped document at 10 and flags the typed gap", () => {
    const r = clientCompleteness({
      sites: [site()],
      contacts: [contact(true, "p@x.test")],
      assets: [asset()],
      documents: [doc({ layout_id: null })],
      layouts: [layout("22222222-2222-4222-8222-222222222222")],
      portalUsers: [],
    });
    // 25 contacts + 20 sites + 10 untyped doc + 10 fresh + 15 assets = 80
    expect(r.score).toBe(80);
    expect(r.gaps.map((g) => g.key)).toContain("typed-document");
  });

  it("flags stale documents beyond 180 days and withholds freshness points", () => {
    const stale = new Date(Date.now() - 200 * 24 * 3600 * 1000);
    const r = clientCompleteness({
      sites: [site()],
      contacts: [contact(true, "p@x.test")],
      assets: [],
      documents: [doc({ updated_at: stale.toISOString() })],
      layouts: [],
      portalUsers: [portalUser()],
    });
    // 25 + 20 + 10 + 0 + 5 = 60
    expect(r.score).toBe(60);
    expect(r.gaps.map((g) => g.key)).toContain("freshness");
  });

  it("withholds contact points when no contact carries an email", () => {
    const r = clientCompleteness({
      sites: [site()],
      contacts: [contact(true, null)],
      assets: [],
      documents: [],
      layouts: [],
      portalUsers: [],
    });
    // 20 sites only: 20/90 pro-rated → 22; contact + typed gaps present,
    // primary is fine
    expect(r.score).toBe(22);
    const keys = r.gaps.map((g) => g.key);
    expect(keys).toContain("contact-email");
    expect(keys).not.toContain("primary-contact");
  });

  it("pro-rates over known legs only when a metadata leg is unavailable (null)", () => {
    const r = clientCompleteness({
      sites: [site()],
      contacts: [contact(true, "p@x.test")],
      assets: null,
      documents: [doc()],
      layouts: [layout("22222222-2222-4222-8222-222222222222")],
      portalUsers: null,
    });
    // Known max: 25 + 20 + 25 + 10 = 80; earned 80 → 100. No asset/portal gaps.
    expect(r.score).toBe(100);
    const keys = r.gaps.map((g) => g.key);
    expect(keys).not.toContain("assets");
    expect(keys).not.toContain("portal");
    expect(r.basis).toContain("n/a asset(s)");
  });
});

describe("scoreTone", () => {
  it("maps scores to AA-safe semantic tones", () => {
    expect(scoreTone(100)).toBe("success");
    expect(scoreTone(75)).toBe("success");
    expect(scoreTone(74)).toBe("warning");
    expect(scoreTone(40)).toBe("warning");
    expect(scoreTone(39)).toBe("danger");
    expect(scoreTone(0)).toBe("danger");
  });
});
