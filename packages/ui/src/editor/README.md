# packages/ui/src/editor/ — shared editor primitives (ADR 0007, Wave 17 lane 1)

> **Status:** scaffolded, empty — lane `p17-kb-tiptap-model` populates; README-only
> until then. Component homes row: "KB rich-content editor (Tiptap 3 shell +
> read-only renderer)" (TASKLIST.md ⇒ `apps/web/lib/editor/` + this home).

**Purpose:** the reusable, theme-owned UI of the editor that `apps/web/lib/editor/`
(wiring) composes: toolbar, slash menu, floating marks menu, embed/link cards,
mention/asset pickers, empty states.

**Non-purpose:** no Tiptap bypass — primitives are presentation + interaction only;
all editor state flows through Tiptap commands (`@tiptap/core`) owned by the shell;
no business policy (Rust-side validation lives in `crates/core-kb`).

**Binding constraints (same as every Wave 16 surface):** Workbench/Tangerine tokens
only (no off-palette hex — CI hex-scan applies), 2px-radius vocabulary from the
theme, shadow/radius consistency, every element keyboard-reachable + focus-visible,
light+dark parity, transitions `≤120ms` transform/opacity with
`prefers-reduced-motion` honored, WCAG 2.2 AA labels on icon-only controls.
