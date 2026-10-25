// The isolation policy as a browser receives it (p56-isolation-headers; audit
// record `apps/web/response-headers-missing-coop-frame-isolation`).
//
// WHY A REAL RESPONSE AND NOT THE CONFIG. `apps/web/tests/app/security-headers.test.ts`
// asserts the map the config produces; that suite cannot prove Next.js serves
// it, and the finding was about a live window, not a JavaScript object. This
// spec runs against the standalone production build the suite already boots
// (e2e/playwright.config.ts `webServer`), so what it reads is what a browser
// gets.
//
// It runs in both base theme projects like every other flow; nothing here
// depends on theme, session or seeded data.
import { expect, test } from "@playwright/test";
import { WEB_URL } from "../helpers/api";

/** The policy, restated here on purpose: a spec that imported the config would
 *  agree with a wrong value. These are the values this lane was sent to ship. */
const EXPECTED = {
  "cross-origin-opener-policy": "same-origin",
  "cross-origin-resource-policy": "same-origin",
  "x-frame-options": "DENY",
  "content-security-policy": "frame-ancestors 'none'",
} as const;

function lookup(headers: Record<string, string>, name: string): string | undefined {
  // Playwright lower-cases header names; do the same so the spec says what it
  // means rather than depending on the server's casing.
  return headers[name];
}

test.describe("response isolation policy", () => {
  test("the document carries COOP, CORP, XFO and frame-ancestors", async ({ request }) => {
    const response = await request.get(`${WEB_URL}/login`);
    expect(response.status()).toBe(200);
    const headers = response.headers();
    for (const [name, value] of Object.entries(EXPECTED)) {
      expect(lookup(headers, name), `${name} on the /login document`).toBe(value);
    }
  });

  test("redirects carry it too, so the shell cannot be reached around the policy", async ({
    request,
  }) => {
    // `/` is a server redirect to /today (app/page.tsx). A frame or an opener
    // can point at it, and the redirect response is the one that answers.
    const response = await request.get(`${WEB_URL}/`, { maxRedirects: 0 });
    expect([307, 308]).toContain(response.status());
    for (const [name, value] of Object.entries(EXPECTED)) {
      expect(lookup(response.headers(), name), `${name} on the / redirect`).toBe(value);
    }
  });

  test("COEP stays off", async ({ request }) => {
    const headers = (await request.get(`${WEB_URL}/login`)).headers();
    // Off is a decision, not an oversight: the KB renders registered oEmbed
    // embeds and allowlisted remote images that send no CORP (see the lane
    // REVIEW). `unsafe-none` is the default and is not sent either — sending it
    // would imply a choice was made at the header instead of in the config.
    expect(headers["cross-origin-embedder-policy"]).toBeUndefined();
  });

  test("XFO and frame-ancestors agree on the same response", async ({ request }) => {
    const headers = (await request.get(`${WEB_URL}/login`)).headers();
    const xfo = headers["x-frame-options"];
    const csp = headers["content-security-policy"] ?? "";
    const directive = csp
      .split(";")
      .map((part) => part.trim().split(/\s+/))
      .find(([name]) => name?.toLowerCase() === "frame-ancestors");
    expect(directive, "frame-ancestors must be present").toBeTruthy();
    // The pair must say the same thing. `DENY` beside a permissive
    // `frame-ancestors` would refuse on old engines and permit on current ones.
    expect(xfo).toBe("DENY");
    expect(directive!.slice(1)).toEqual(["'none'"]);
  });

  test("another origin cannot frame the app", async ({ page }) => {
    // A `data:` document has an opaque origin, so it is cross-origin to the
    // app: exactly the relationship the finding is about.
    await page.goto("data:text/html,<body>not marula</body>");
    await page.evaluate((url) => {
      const frame = document.createElement("iframe");
      frame.src = `${url}/login`;
      document.body.appendChild(frame);
    }, WEB_URL);

    // Wait for the frame's own navigation to resolve (Chromium parks a refused
    // frame on `chrome-error://chromewebdata/`), then assert no frame ever
    // points at the app. Without the wait this could pass vacuously.
    await expect.poll(() => page.frames().length, { timeout: 15_000 }).toBeGreaterThan(1);
    const loaded = page.frames().map((frame) => frame.url());
    expect(loaded.some((url) => url.startsWith(WEB_URL))).toBe(false);
  });
});
