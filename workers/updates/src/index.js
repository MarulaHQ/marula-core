/**
 * The desktop update channel (ADR 0028, §5).
 *
 * The endpoint is compiled into every client, so this path shape is a
 * compatibility promise: `desktop/<target>/<arch>/<current_version>`. The
 * client says what it is and what it has; this answers whether there is
 * something newer, and nothing else.
 *
 * Two answers exist, and the absence of a third matters:
 *
 *   204  nothing for you — unknown platform, no release, staged, or current
 *   200  here is a version, a URL and the minisign signature over that exact
 *        file, which the client verifies against the key in its own bundle
 *
 * A 404 would make a client that has never been published for its platform
 * look like a broken server; 204 says "stay as you are", which is true.
 *
 * Tauri refuses a version that is not strictly higher, so a replayed older
 * release is ignored by the client even if this Worker is compromised. That is
 * the property worth more than TLS here.
 */

const parse = (value) => String(value || "0").split(".").map((n) => parseInt(n, 10) || 0);

/** Strictly newer, three parts, numeric. `0.10.0` is newer than `0.9.0`. */
function newer(candidate, current) {
  const a = parse(candidate);
  const b = parse(current);
  for (let i = 0; i < 3; i += 1) {
    if ((a[i] || 0) > (b[i] || 0)) return true;
    if ((a[i] || 0) < (b[i] || 0)) return false;
  }
  return false;
}

const stay = () => new Response(null, { status: 204, headers: { "cache-control": "no-store" } });

export default {
  async fetch(request, env) {
    const match = new URL(request.url).pathname.match(/^\/desktop\/([^/]+)\/([^/]+)\/([^/]+)$/);
    if (!match) return new Response("not found", { status: 404 });

    const [, target, arch, current] = match;
    // The pointer is written by `scripts/release/updates.mjs`, one per platform
    // and channel. A single KV read: this path is hit on every launch.
    const latest = await env.UPDATES.get(`latest:${target}-${arch}`, "json");
    if (!latest) return stay();

    // A staged release is announced but not yet served. Nothing identifies the
    // client, so the schedule is time rather than a cohort: the doctrine is no
    // telemetry, and a hash of an IP would be one.
    if (latest.not_before && Date.now() < Date.parse(latest.not_before)) return stay();
    if (!newer(latest.version, current)) return stay();

    return Response.json(
      {
        version: latest.version,
        pub_date: latest.pub_date,
        notes: latest.notes,
        url: latest.url,
        signature: latest.signature,
      },
      { headers: { "cache-control": "no-store" } },
    );
  },
};
