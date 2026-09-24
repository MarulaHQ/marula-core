// Rewrite every symlink in the Next standalone output to the resolved real
// path, remapping the PARTIAL next instance that `next build` drops into
// apps/web/node_modules/.pnpm (dev-server require-hook mechanism) onto the
// FULL root store. Same trick scripts/build-desktop.sh uses for the .app
// bundle; without it the container runtime dies on `Cannot find module
// './cpu-profile'` from next/dist/server/lib/start-server.js.
const fs = require("fs");
const path = require("path");
const root = process.argv[2] || "/repo/apps/web/.next/standalone";
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isSymbolicLink()) {
      // Resolve by LINK TEXT (works for dangling links too — next's partial
      // instance makes some targets vanish), then re-anchor the store prefix
      // onto the full root store, keeping the exact version tail.
      const text = fs.readlinkSync(p);
      const abs = path.resolve(path.dirname(p), text).replace(
        "/repo/apps/web/node_modules/.pnpm",
        "/repo/node_modules/.pnpm",
      );
      fs.unlinkSync(p);
      fs.symlinkSync(abs, p);
    } else if (e.isDirectory()) {
      walk(p);
    }
  }
})(root);
console.log("standalone symlinks remapped to the full root store");
