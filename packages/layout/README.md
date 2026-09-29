# @marula-core/layout

> **Purpose:** Multi-pane layout engine: CSS grid + container queries, persisted panel state per user/device class, multi-monitor support.
> **Non-purpose:** No domain knowledge — it arranges panels, it knows nothing about tickets.

The engine renders a declarative pane tree (nested rows/columns with draggable dividers and tabbed panes), persists that tree through a pluggable store keyed per user id, and enforces minimum-size constraints both in JS math and as CSS `minmax()` backstops. It is a first-class internal package per BUILD-PROMPT §2: TypeScript + React, framework-agnostic (no `next/*` imports anywhere), consumed by `apps/web`.

## Non-goals

- No floating docks, pop-out windows, or saved workspace *presets* management.
- No drag-and-drop of panes or tabs between containers (programmatic moves are provided; pointer DnD comes later).
- No cascading resize redistribution: a divider drag affects exactly its two neighboring children.
- No content rendering of any kind — pane bodies are host-supplied render props.
- No theme: every color is a `--marula-*` variable owned by `packages/ui`.

## Install / wiring

In-repo workspace package; apps consume sources directly (no build step):

```ts
import { PanelLayout, usePanelLayout } from "@marula-core/layout";
import "@marula-core/layout/styles.css";
```

## API sketch

### Tree model (`types.ts`)

```ts
type LayoutNode = SplitNode | PaneNode;

interface SplitNode {
  kind: "split";
  id: string;
  direction: "row" | "column";   // row = side-by-side, column = stacked
  children: readonly LayoutNode[];
  sizes: readonly number[];      // one positive fraction per child, sums to ~1
}

interface PaneNode {
  kind: "pane";
  id: string;
  tabs: readonly PaneTab[];      // { id, title }
  activeTabId: string | null;
  minWidthPx?: number;           // min constraint along a horizontal axis
  minHeightPx?: number;          // min constraint along a vertical axis
}
```

Factories (`createPane`, `createSplit`) and pure immutable operations (`insertPane`, `removePane`, `openTab`, `closeTab`, `moveTab`, `activateTab`, `setSplitSizes`, `unwrapSingleChildSplits`) live in `tree.ts`. All return new roots; nothing mutates.

### Constraint math (`constraints.ts`)

Pure, fully tested functions:

- `fractionsToPx` / `pxToFractions` — fraction/pixel conversion.
- `dividerRangePx` — feasible absolute range for one divider given both neighbors' bounds. Unsatisfiable constraints collapse to a deterministic midpoint.
- `applyPairResizePx` — place the boundary at a target offset; preserves the pair's pixel total so other siblings never move.
- `renormalizeFractions` — water-filling redistribution onto available pixels honoring per-child min/max; falls back to proportional-to-minimum when space cannot fit all minimums (container clips).
- `subtreeAxisMinPx(node, axis)` — recursive minimum extent; splits sum their children, panes use their declared value or `DEFAULT_MIN_PANE_PX` (120px).

### Controller hook

```tsx
const layout = usePanelLayout({
  userId: currentUser.id,
  fallback: defaultWorkspaceTree,   // used before first save or on corrupt data
  store: /* optional; defaults to localStorage adapter in browsers */,
  saveDebounceMs: 400,              // default
  onPersistIssue: reportToTelemetry, // optional
});

<PanelLayout
  controller={layout}
  renderPaneContent={({ pane, tab }) => <TicketPane tab={tab} />}
  renderEmptyPane={(pane) => <EmptyState />}
  collapseBelowPx={720}
/>
```

`PanelLayoutController` exposes `status`, `root`, `loadWarnings`, plus mutations:
`setSplitSizes`, `activateTab`, `openTab`, `closeTab`, `moveTab`, `splitPane`,
`removePane`, `reset`. Switching `userId` loads that user's layout.

### Interaction behavior

- **Pointer:** press a divider to capture; the grid re-renders live on every animation frame while dragging (no ghost-line preview). Double-click equalizes the pair.
- **Keyboard:** focus a divider (`role="separator"`), then Arrow keys move it (12px steps; Shift = 48px); Home/End jump to the min/max feasible boundary.
- **Reduced motion:** all transitions are ≤120ms and wrapped in `@media (prefers-reduced-motion: no-preference)`; dragging itself is direct manipulation, never animated.
- **Responsive collapse:** below `collapseBelowPx` (measured via ResizeObserver), a top-level `row` split renders stacked as a `column`; nested splits keep their authored direction. Every pane is also an inline-size container (`marula-pane`) so host content can adapt with its own `@container` rules, and the tab strip compacts itself under 320px pane width.

## Persistence contract

```ts
interface PanelStateStore {
  load(userId: string): Promise<string | null>;   // opaque payload or null
  save(userId: string, payload: string): Promise<void>;
  clear(userId: string): Promise<void>;
}
```

- Payloads are **opaque serialized envelopes** produced by `encodeLayoutState`; adapters never interpret them. Shipped adapters: `LocalStoragePanelStateStore` (keys: `<namespace>:<userId>`, namespace default `marula.layout`) and `MemoryPanelStateStore` (tests/SSR). A server adapter implements the same three methods later — no engine changes required.
- Envelope: `{ version: 1, savedAt: <ISO-8601 UTC>, root: <tree> }`.
- **Decode is defensive by design.** `decodeLayoutState` validates untrusted input and repairs recoverable defects, reporting each repair as a warning: unknown versions still parse best-effort; missing/invalid sizes reset to equal fractions; single-child splits collapse recursively up to the root; duplicate ids regenerate; dangling `activeTabId`s clear; malformed tabs drop individually. Structurally hopeless input throws `LayoutStateDecodeError`, and the hook restores the fallback tree.
- Saves are debounced (default 400ms) after each mutation; write failures surface via `onPersistIssue` and never corrupt React state.

## CSS contract

Import `@marula-core/layout/styles.css` once. The engine consumes **only** these custom properties and contains zero hardcoded colors:

| Variable | Used for |
|---|---|
| `--marula-color-canvas` | layout background behind panes |
| `--marula-color-surface` | pane bodies and the tab strip |
| `--marula-color-border` | divider center line, tab strip bottom border, close-button hover |
| `--marula-color-accent` | divider hover/drag/focus indicators, active-tab underline, focus outlines |
| `--marula-color-text-primary` | active tab label |
| `--marula-color-text-secondary` | inactive tab labels, empty-pane placeholder |
| `--marula-font-family` | tab strip and placeholder text |
| `--marula-radius-sm` | 2px corners on tabs/close buttons |

Structural (non-color) variables carry engine defaults overridable per subtree:
`--marula-layout-divider-size` (5px hit area). Until `packages/ui` defines the color tokens, unpainted variables degrade gracefully (transparent/borderless chrome) rather than inventing off-palette values.

Class names: `marula-layout`, `marula-split[data-direction][data-depth]`, `marula-pane`, `marula-tabs`, `marula-tab[__title|__close]`, `marula-pane__body`, `marula-pane__empty`, `marula-divider[data-direction][data-dragging]`.

## Testing

`pnpm test` runs Vitest suites covering serialize/restore round-trips and hostile-payload repair, all resize-constraint math (ranges, clamping, water-filling, unsatisfiable cases), tree operation invariants, and both persistence adapters. DOM interaction flows are owned by the repository-level Playwright E2E suite per BUILD-PROMPT §8; this package keeps its component layer thin and delegates every decision to the tested pure modules above.
