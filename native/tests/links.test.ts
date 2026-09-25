// Deep-link vocabulary: the frozen printed-QR shapes, the dev scan
// injector link, push routes, and non-links.

import { describe, expect, it } from "vitest";

import { notificationRoute, parseMeridianLink, routeForPushPayload } from "@/core/links";

const ID = "0192a1b2-c3d4-7000-8000-000000000001";

describe("parseMeridianLink", () => {
  it("parses every frozen printed-QR shape", () => {
    expect(parseMeridianLink(`meridian://stock-item/${ID}`)).toEqual({
      kind: "entity",
      entity: "stock-item",
      id: ID,
    });
    expect(parseMeridianLink(`meridian://stock-location/${ID}`)).toEqual({
      kind: "entity",
      entity: "stock-location",
      id: ID,
    });
    expect(parseMeridianLink(`meridian://stock-serial/${ID}`)).toEqual({
      kind: "entity",
      entity: "stock-serial",
      id: ID,
    });
    expect(parseMeridianLink(`meridian://device-asset/${ID}`)).toEqual({
      kind: "entity",
      entity: "device-asset",
      id: ID,
    });
  });

  it("tolerates surrounding whitespace, a trailing slash, and an empty authority", () => {
    expect(parseMeridianLink(`  meridian:///stock-item/${ID}/  `)).toEqual({
      kind: "entity",
      entity: "stock-item",
      id: ID,
    });
  });

  it("parses the dev scan injector link", () => {
    expect(parseMeridianLink("meridian://scan-result?text=RSV-1234")).toEqual({
      kind: "scan-result",
      text: "RSV-1234",
    });
    expect(parseMeridianLink("meridian://scan-result?text=")).toEqual({
      kind: "scan-result",
      text: "",
    });
  });

  it("parses ticket and notifications destinations", () => {
    expect(parseMeridianLink(`meridian://ticket/${ID}`)).toEqual({ kind: "ticket", id: ID });
    expect(parseMeridianLink("meridian://notifications")).toEqual({ kind: "notifications" });
  });

  it("refuses near-misses instead of guessing", () => {
    expect(parseMeridianLink("meridian://stock-item/not-a-uuid")).toEqual({
      kind: "unknown",
      url: "meridian://stock-item/not-a-uuid",
    });
    expect(parseMeridianLink("https://example.com/stock-item/x")).toBeNull();
    expect(parseMeridianLink("")).toBeNull();
  });
});

describe("push routes", () => {
  it("routes a ticket notification to the ticket pane", () => {
    expect(routeForPushPayload({ ticket_id: ID })).toBe(`/ticket/${ID}`);
    expect(notificationRoute({ ticket_id: ID })).toBe(`/ticket/${ID}`);
  });

  it("falls back to the notification center", () => {
    expect(routeForPushPayload({ notification_id: ID })).toBe("/notifications");
    expect(notificationRoute({})).toBe("/notifications");
    expect(routeForPushPayload(undefined)).toBe("/notifications");
  });
});
