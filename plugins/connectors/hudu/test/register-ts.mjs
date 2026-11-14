// Node's type stripping runs TypeScript as-is, so the `.js` specifiers the
// sources use (correct for `tsc` under `moduleResolution: bundler`, and for
// the published package) do not resolve against the `.ts` files on disk.
// This registers a resolve hook that falls back to the sibling `.ts` file.
// Tests only: nothing here ships, and no dependency is added.
import { register } from "node:module";

register("./resolve-ts.mjs", import.meta.url);
