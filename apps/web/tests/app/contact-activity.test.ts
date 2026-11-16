import { describe, expect, it } from "vitest";

import {
  contactActivitySummary,
  contactKindLabel,
  type ContactActivity,
} from "../../lib/contact-activity";

describe("contact activity", () => {
  it("names every source the server derives", () => {
    expect(contactKindLabel("email_in")).toBe("Emailed us");
    expect(contactKindLabel("email_out")).toBe("We emailed");
    expect(contactKindLabel("portal_reply")).toBe("Portal reply");
    expect(contactKindLabel("call_in")).toBe("Called us");
    expect(contactKindLabel("call_out")).toBe("We called");
  });

  it("falls back to a neutral word for an unknown source", () => {
    expect(contactKindLabel("carrier_pigeon")).toBe("Contact");
    expect(contactKindLabel(null)).toBe("Contact");
  });

  it("reads Never only when nothing was recorded", () => {
    const empty: ContactActivity = {
      contact_id: "c1",
      last_at: null,
      last_kind: null,
      recent: [],
    };
    expect(contactActivitySummary(empty)).toBe("Never");
    expect(contactActivitySummary(undefined)).toBe("Never");
  });

  it("puts the source before the recency", () => {
    const row: ContactActivity = {
      contact_id: "c1",
      last_at: new Date(Date.now() - 2 * 86_400_000).toISOString(),
      last_kind: "portal_reply",
      recent: [],
    };
    expect(contactActivitySummary(row)).toMatch(/^Portal reply · 2d ago$/);
  });
});
