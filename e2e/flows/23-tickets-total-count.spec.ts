// Flow 23 — the tickets list's pagination contract headers: x-total-count
// (the footer's N-of-M denominator) and its CORS exposure.
//
// The surface reads the count cross-origin (the app and the API answer from
// different origins in the demo), so the assertion is not only "the header
// exists" — an unexposed header exists and is still useless to the page. Both
// halves are pinned here. Per the isolation-policy spec's own note, this pins
// live responses rather than a Rust handler's intent.
import { expect, test } from "../helpers/harness";
import { API_URL, apiLogin, DEMO_OWNER_EMAIL, DEMO_PASSWORD } from "../helpers/api";

test.describe("tickets list x-total-count", () => {
  test("the header answers the filter's denominator and CORS exposes it", async ({ request }) => {
    const tokens = await apiLogin(API_URL, DEMO_OWNER_EMAIL, DEMO_PASSWORD);
    const response = await request.get(`${API_URL}/api/v1/tickets?limit=1&offset=0`, {
      headers: { authorization: `Bearer ${tokens.access_token}` },
    });
    expect(response.status()).toBe(200);

    // One loaded page holds 1 row; the header carries the whole matching set.
    const total = response.headers()["x-total-count"];
    expect(total, "x-total-count on the tickets list").toBeTruthy();
    expect(Number(total)).toBeGreaterThanOrEqual(1);

    // A bounded filter narrows the denominator; the two headers must agree
    // with each other in direction, not value.
    const filtered = await request.get(`${API_URL}/api/v1/tickets?limit=1&offset=0&priority=p1`, {
      headers: { authorization: `Bearer ${tokens.access_token}` },
    });
    expect(filtered.status()).toBe(200);
    const filteredTotal = Number(filtered.headers()["x-total-count"]);
    expect(filteredTotal).toBeLessThanOrEqual(Number(total));

    // The exposure the browser needs: a preflight-free GET's response only
    // carries this header to a cross-origin page when Access-Control-Expose
    // -Headers names it.
    const exposed = response.headers()["access-control-expose-headers"] ?? "";
    expect(exposed.toLowerCase()).toContain("x-total-count");
  });
});
