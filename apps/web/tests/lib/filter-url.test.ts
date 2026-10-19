import { describe, expect, it } from "vitest";
import { filterUrl, parseFilterUrl } from "../../lib/filter-url";

describe("filter-url", () => {
  it("round-trips the view, the match mode and the clauses", () => {
    const url = filterUrl("https://marula.test/tickets", {
      view: "all",
      match: "any",
      filters: [
        { field: "client_id", op: "contains", value: "Acme Manufacturing" },
        { field: "category", op: "is_set" },
      ],
      sort: { field: "sla", dir: "asc" },
    });
    expect(url).toContain("view=all");
    expect(url).toContain("match=any");
    expect(url).toContain("sort=sla%3Aasc");
    expect(parseFilterUrl(new URL(url).search)).toEqual({
      view: "all",
      match: "any",
      filters: [
        { field: "client_id", op: "contains", value: "Acme Manufacturing" },
        { field: "category", op: "is_set", value: undefined },
      ],
    });
  });

  it("omits match when it is the pack's default", () => {
    const url = filterUrl("https://marula.test/tickets", { match: "all", filters: [] });
    expect(url).not.toContain("match=");
    expect(parseFilterUrl(new URL(url).search)).toEqual({});
  });

  it("drops a malformed clause rather than inventing one", () => {
    expect(parseFilterUrl("?f=client_id%7E~")).toEqual({});
    expect(parseFilterUrl("?f=client_id~contains~Acme")).toEqual({
      filters: [{ field: "client_id", op: "contains", value: "Acme" }],
    });
  });
});
