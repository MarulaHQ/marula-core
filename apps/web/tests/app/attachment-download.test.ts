// Presigned attachment downloads (Wave 52 lane P7), web side.
//
// Purpose: pin the client contract of the two download helpers — the ticket
// request, the save from the store with NO bearer token, and every fallback
// to the stream. The failure this suite exists for is a download that either
// leaks the API token to the object store or dies when the store's host is
// not reachable from the browser.
//
// Non-purpose: no rendering (the buttons that call these helpers are covered
// by their page suites) and no backend contract (that lives in Rust).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearTokens,
  downloadAttachment,
  downloadDocumentAttachment,
  saveTokens,
} from "../../lib/api";
import { setFetchRouter } from "../helpers/fetch-router";

interface Call {
  url: string;
  headers: Headers;
}

const calls: Call[] = [];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function streamResponse(filename: string, bytes = "PNG"): Response {
  return new Response(bytes, {
    status: 200,
    headers: {
      "content-type": "image/png",
      "content-disposition": `attachment; filename="${filename}"`,
    },
  });
}

function ticket(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    mode: "presigned",
    url: "https://objects.test/org/o/attachments/abc?sig=1",
    expires_at: "2026-10-20T12:05:00Z",
    filename: "screenshot.png",
    content_type: "image/png",
    size_bytes: 3,
    sha256: "abc",
    ...overrides,
  };
}

beforeEach(() => {
  calls.length = 0;
  // A signed-in session, so the assertions can tell the two requests apart:
  // the API call carries the bearer token and the store call must not.
  saveTokens({
    access_token: "test-access-token",
    refresh_token: "test-refresh-token",
    session_id: "test-session",
    expires_in: 3600,
  });
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  Object.assign(URL, {
    createObjectURL: () => "blob:download",
    revokeObjectURL: () => {},
  });
});

afterEach(() => {
  setFetchRouter(null);
  clearTokens();
  vi.restoreAllMocks();
});

describe("attachment downloads", () => {
  it("saves from the store and sends the token to nobody but the API", async () => {
    setFetchRouter(async (input, init) => {
      const url = typeof input === "string" ? input : input.toString();
      calls.push({ url, headers: new Headers(init?.headers) });
      if (url.includes("presign=true")) return jsonResponse(ticket());
      return streamResponse("screenshot.png");
    });

    await downloadAttachment("att-1", "screenshot.png");

    expect(calls.map((c) => c.url)).toEqual([
      expect.stringContaining("/api/v1/attachments/att-1/download?presign=true"),
      "https://objects.test/org/o/attachments/abc?sig=1",
    ]);
    // The signed URL carries its own signature; a bearer token on it would
    // hand the API credential to the object store.
    expect(calls[0]?.headers.get("authorization")).toBe("Bearer test-access-token");
    expect(calls[1]?.headers.get("authorization")).toBeNull();
  });

  it("streams through the API when the deployment cannot sign a URL", async () => {
    setFetchRouter(async (input, init) => {
      const url = typeof input === "string" ? input : input.toString();
      calls.push({ url, headers: new Headers(init?.headers) });
      if (url.includes("presign=true")) return jsonResponse(ticket({ mode: "stream", url: null }));
      return streamResponse("runbook.pdf");
    });

    await downloadDocumentAttachment("doc-1", "att-2", "fallback.pdf");

    expect(calls).toHaveLength(2);
    expect(calls[1]?.url).toContain(
      "/api/v1/documents/doc-1/attachments/att-2/download",
    );
    expect(calls[1]?.url).not.toContain("presign");
  });

  it("falls back to the stream when the store's host is unreadable", async () => {
    setFetchRouter(async (input, init) => {
      const url = typeof input === "string" ? input : input.toString();
      calls.push({ url, headers: new Headers(init?.headers) });
      if (url.includes("presign=true")) return jsonResponse(ticket());
      // A bucket with no CORS rule for this origin rejects like this.
      if (url.startsWith("https://objects.test/")) throw new TypeError("Failed to fetch");
      return streamResponse("screenshot.png");
    });

    await downloadAttachment("att-1", "screenshot.png");

    expect(calls).toHaveLength(3);
    expect(calls[2]?.url).toContain("/api/v1/attachments/att-1/download");
    expect(calls[2]?.url).not.toContain("presign");
    expect(calls[2]?.headers.get("authorization")).toBe("Bearer test-access-token");
  });

  it("propagates a refusal instead of quietly streaming it", async () => {
    setFetchRouter(async (input) => {
      const url = typeof input === "string" ? input : input.toString();
      calls.push({ url, headers: new Headers() });
      return jsonResponse(
        { error: { code: 409, reason: "conflict", message: "these bytes are held by the scan gate" } },
        409,
      );
    });

    await expect(downloadAttachment("att-held", "x.png")).rejects.toThrow(
      "Download failed (409)",
    );
    expect(calls).toHaveLength(1);
  });
});
