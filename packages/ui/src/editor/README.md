# packages/ui/src/editor/ — shared editor primitives (ADR 0007, Wave 16 lane 2)

> **Status:** populated (Wave 16 lane 2, 2026-09-16) — toolbar, block menu,
> and the saved-prose renderer live in `src/editor/` (exported from the
> package barrel). CSS rides `components.css` `.m-editor-*` classes.

**Purpose:** the reusable, theme-owned UI of the editor that `apps/web/lib/editor/`
(wiring) composes: toolbar, slash menu, floating marks menu, embed/link cards,
mention/asset pickers, empty states.

**Non-purpose:** no Tiptap bypass — primitives are presentation + interaction only;
all editor state flows through Tiptap commands (`@tiptap/core`) owned by the shell;
no business policy (Rust-side validation lives in `crates/core-kb`).

**Binding constraints (same as every Wave 17 surface):** Workbench/Tangerine tokens
only (no off-palette hex — CI hex-scan applies), 2px-radius vocabulary from the
theme, shadow/radius consistency, every element keyboard-reachable + focus-visible,
light+dark parity, transitions `≤120ms` transform/opacity with
`prefers-reduced-motion` honored, WCAG 2.2 AA labels on icon-only controls.
