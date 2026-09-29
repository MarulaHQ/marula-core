/**
 * Class-name composition helper shared by all components.
 * Purpose: merge conditional class lists and de-duplicate Tailwind-style
 * conflicts. Non-purpose: no styling logic of its own.
 *
 * The implementation is the `cn` package (shadcn's, v0.2.6) — NOT a
 * hand-rolled `twMerge(clsx(...))`. It is the same import the app already
 * uses in `apps/web/lib/utils.ts`, so the workspace has ONE class-name
 * helper instead of two.
 *
 * Why it matters here (wave41 D9): the hand-rolled version resolved
 * conflicts through `tailwind-merge@^2.6.0`, the **Tailwind v3** class
 * table, while the app runs `tailwindcss@^4.1.0`. Every v4 class name
 * missing from that table failed to de-duplicate and the FIRST class won
 * instead of the last. `cn` ships its own v4-aware table, is verified
 * against tailwind-merge by 356k differential tests, and has zero
 * dependencies — which is why `clsx` and `tailwind-merge` are dropped
 * from this package rather than bumped.
 */
export { cn } from "cn";
