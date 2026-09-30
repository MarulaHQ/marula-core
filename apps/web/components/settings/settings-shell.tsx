// Settings shell primitives — the shared IA vocabulary for /settings/*.
//
// Purpose: one save pattern, one destructive-confirm pattern, and one
// honest-empty-state pattern used by EVERY settings page so the area reads as
// a single product (Wave 17 lane 7 settings IA pass). The page header itself
// is no longer defined here: `SettingsHeader` is a thin alias for the
// surface pack's `SurfaceHeader` at its `compact` size (Wave 41 lane
// `p41-ui-chrome`), which is what the 65 staff pages outside /settings were
// each re-deriving on their own. Keeping the name and the props means all 33
// call sites are untouched while the frame has exactly one owner.
//
// Tokens only (docs/theme.md) — components never declare hexes.
//
// Non-purpose: no data fetching and no settings logic here — pages own
// their payloads; these primitives only render and expose callbacks.
"use client";

import { Button } from "@marula-core/ui";
import { Dialog, DialogActions, DialogContent } from "@marula-core/ui";
import { SurfaceHeader } from "@marula-core/ui";
import { friendlyMutationText } from "../../lib/error-copy";
import type { ReactNode } from "react";

/**
 * Standard settings page header: title + one-line description, plus a
 * right-aligned meta/actions slot.
 *
 * The `compact` title is the pack's settings/utility shape; a settings page
 * that wants the large data-surface title passes `<SurfaceHeader>` directly.
 */
export function SettingsHeader({
  title,
  description,
  children,
}: {
  title: string;
  /** Inline copy or rich node (links) — rendered under the title. */
  description?: ReactNode;
  /** Optional right-aligned meta/actions (badges, primary buttons). */
  children?: ReactNode;
}) {
  return (
    <SurfaceHeader
      size="compact"
      title={title}
      description={description}
      actions={children}
      // The alias keeps the settings area's historical 16px under the header;
      // `SurfaceHeader` itself is spacing-neutral so a new surface can drop it
      // into a grid or a flex column without fighting for the gap.
      className="mb-4"
    />
  );
}

export interface SaveBarProps {
  /** Whether the draft differs from the persisted record (Save is armed). */
  dirty: boolean;
  saving?: boolean;
  /** Deterministic inline confirmation string after a successful save. */
  saved?: boolean;
  error?: string | null;
  onSave?: () => void;
  /** Optional revert-to-last-saved affordance; hidden when absent. */
  onRevert?: () => void;
  saveLabel?: string;
  /** Optional deterministic hook point for tests/drivers on the save button. */
  saveTestId?: string;
}

/**
 * The ONE save pattern for settings forms: consistent copy, disabled-while-
 * saving, deterministic confirmation, typed error surfaced verbatim via
 * friendlyMutationText. Renders nothing when the page has no save action
 * (read-only surfaces omit it entirely instead of showing a dead button).
 */
export function SaveBar({
  dirty,
  saving = false,
  saved = false,
  error = null,
  onSave,
  onRevert,
  saveLabel = "Save changes",
  saveTestId,
}: SaveBarProps): React.JSX.Element | null {
  if (onSave === undefined) return null;
  return (
    <div
      className="flex flex-wrap items-center gap-3"
      data-testid="settings-save-bar"
    >
      <Button variant="primary" disabled={!dirty || saving} onClick={onSave} data-testid={saveTestId}>
        {saving ? "Saving…" : saveLabel}
      </Button>
      {onRevert !== undefined && dirty && !saving ? (
        <Button variant="secondary" onClick={onRevert}>
          Discard changes
        </Button>
      ) : null}
      {saved && !error && (
        <span
          className="text-[12px]"
          role="status"
          style={{ color: "var(--m-status-success-fg)" }}
          data-testid="settings-save-confirm"
        >
          Saved.
        </span>
      )}
      {error && (
        <span
          className="text-[12px]"
          role="alert"
          style={{ color: "var(--m-status-danger-fg)" }}
          data-testid="settings-save-error"
        >
          {error}
        </span>
      )}
    </div>
  );
}

export interface ConfirmDestructiveProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** The consequence sentence; tone matches the danger without melodrama. */
  body: string;
  /** Exact confirm button copy, e.g. "Delete tag" — action-word first. */
  confirmLabel: string;
  destructiveLabel?: string;
  busy?: boolean;
  onConfirm: () => void;
}

/**
 * The ONE destructive-action treatment across /settings: a dialog that
 * names the exact record, states irreversible consequences, uses the
 * destructive button styling (never mimicking a primary Save), and an
 * explicit cancel. Replaces raw window.confirm().
 */
export function ConfirmDestructive({
  open,
  onOpenChange,
  title,
  body,
  confirmLabel,
  destructiveLabel,
  busy = false,
  onConfirm,
}: ConfirmDestructiveProps): React.JSX.Element {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={title} description={body}>
        <DialogActions data-testid="destructive-actions">
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={busy}>
            {destructiveLabel ?? "Cancel"}
          </Button>
          <Button variant="destructive" onClick={onConfirm} disabled={busy}>
            {confirmLabel}
          </Button>
        </DialogActions>
      </DialogContent>
    </Dialog>
  );
}

/** Typed empty state: honest "backend hasn't landed" surfaces. */
export function NotAvailable({
  what,
  hint,
}: {
  what: string;
  hint?: string;
}): React.JSX.Element {
  return (
    <p
      role="status"
      data-testid="settings-not-available"
      className="text-[13px]"
      style={{ color: "var(--m-text-muted)" }}
    >
      {what} is not available yet{hint ? ` — ${hint}` : "."}
    </p>
  );
}

/** Resolve a server-side rejection into the friendly inline copy. */
export function mutationErrorText(error: unknown): string {
  return friendlyMutationText(error);
}
