"use client";
/**
 * The settings form family — the five states the pack draws once (Wave 41
 * lane 12, `p41-ui-settings`).
 *
 * Purpose: the shapes every /settings surface reuses, built to
 * `brand/marula-mockups/marula-surface-pack/web/settings` states 02–09 and
 * `DESIGN-NOTES-settings.md` §3, so no page draws its own:
 *
 *   - `SettingsDrawer` 480px from the right over a dimmed page (04, 05, 07, 10)
 *   - `FormField` — label, control, hint; validation on SUBMIT, blush not red
 *   - `SaveFailedBanner` — what happened, and that the work is still here
 *   - `StagedSaveBar` — "N unsaved changes. Nothing has changed for anyone yet"
 *   - `BulkActionBar` — selection replacing the column header in place
 *   - `ToggleRow` / `ToggleSwitch` — bold title, one line, switch at the right
 *   - `DangerZone` — a small label and a card apart from the rest
 *   - `ConfirmRemap` — names the consequence, asks where things go, and its
 *     button says what it will do
 *   - `ImportReview` — the file chip, what we found row by row, and a primary
 *     that states the count
 *
 * Two rules the whole family obeys, from the pack's form table: toggles in a
 * table STAGE behind a save bar, toggles on their own page APPLY AT ONCE and
 * say Saved; and a switch that cannot be changed here is disabled with a line
 * saying who manages it.
 *
 * Tokens only (`docs/theme.md`): no hex in this file. Motion is opacity and
 * transform at 120ms, and `prefers-reduced-motion` collapses it.
 */

import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Button } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";

/* --- geometry, straight from the pack ------------------------------------ */

/** The app bar's height (chrome lane, 56px). An overlay anchored below it
 * leaves the bar usable and keeps the layer out of the page's own scroller. */
export const APP_BAR = 56;

export const DRAWER_WIDTH = 480;
export const CONFIRM_WIDTH = 520;
export const SETTINGS_COLUMN = 1152;
export const SECTION_RAIL = 232;
export const SHELL_GAP = 32;

/* --- the switch ----------------------------------------------------------- */

export interface ToggleSwitchProps {
  on: boolean;
  /** Changes the value. Omit with `disabled` for a switch managed elsewhere. */
  onChange?: (next: boolean) => void;
  /** Screen-reader name; a visible title should be passed so it matches. */
  label: string;
  /** A switch this page cannot change: disabled, and the row says why. */
  disabled?: boolean;
  testId?: string;
}

/**
 * The 44x26 switch the pack draws: ink track with the gold thumb on, a muted
 * track with the ink thumb off, and a disabled state that keeps its contrast
 * instead of dropping opacity (docs/theme.md §6 — opacity drags the label
 * under the floor).
 */
export function ToggleSwitch({
  on,
  onChange,
  label,
  disabled = false,
  testId,
}: ToggleSwitchProps): React.JSX.Element {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled || onChange === undefined}
      data-testid={testId}
      onClick={() => onChange?.(!on)}
      style={{
        width: 44,
        height: 26,
        borderRadius: 999,
        border: "1px solid transparent",
        background: disabled
          ? "var(--m-disabled-bg)"
          : on
            ? "var(--m-primary)"
            : "var(--m-muted)",
        borderColor: disabled
          ? "var(--m-border)"
          : on
            ? "transparent"
            : "var(--m-input)",
        position: "relative",
        padding: 0,
        flexShrink: 0,
        cursor: disabled ? "not-allowed" : "pointer",
        transition: "background-color 120ms ease, border-color 120ms ease",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          position: "absolute",
          top: 2,
          right: on ? 3 : "auto",
          left: on ? "auto" : 3,
          width: 20,
          height: 20,
          borderRadius: 999,
          background: disabled
            ? "var(--m-disabled-fg)"
            : on
              ? "var(--m-on-primary)"
              : "var(--m-input)",
          transition: "left 120ms ease, right 120ms ease",
        }}
      />
    </button>
  );
}

/* --- toggle rows ---------------------------------------------------------- */

export interface ToggleRowProps {
  title: string;
  /** One line saying what the control does. */
  description: ReactNode;
  children?: ReactNode;
  /** A pill beside the title — the drawn "Saved" after an immediate apply. */
  badge?: ReactNode;
  /** The first row of a card carries no divider above it. */
  first?: boolean;
  testId?: string;
}

/**
 * One row of a settings card: a bold title, one line of description, and the
 * control at the right. Rows are 56px at the default density step.
 */
export function ToggleRow({
  title,
  description,
  children,
  badge,
  first = false,
  testId,
}: ToggleRowProps): React.JSX.Element {
  return (
    <div
      data-testid={testId}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        minHeight: 56,
        padding: "12px 20px",
        borderTop: first ? "none" : "1px solid var(--m-border)",
      }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <b style={{ color: "var(--m-text)", fontWeight: 600 }}>{title}</b>
          {badge}
        </div>
        <span
          style={{
            display: "block",
            color: "var(--m-text-secondary)",
            fontSize: 13,
            lineHeight: 1.45,
          }}
        >
          {description}
        </span>
      </div>
      {children}
    </div>
  );
}

/** A card that holds toggle rows, with an optional heading above the rows. */
export function ToggleCard({
  title,
  description,
  children,
  testId,
}: {
  title?: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <section
      data-testid={testId}
      style={{
        border: "1px solid var(--m-border)",
        borderRadius: 16,
        background: "var(--m-surface)",
        overflow: "hidden",
      }}
    >
      {title === undefined ? null : (
        <div style={{ padding: "16px 20px 0" }}>
          <b style={{ color: "var(--m-text)", fontSize: 15, fontWeight: 600 }}>{title}</b>
          {description === undefined ? null : (
            <p
              style={{
                margin: "2px 0 0",
                color: "var(--m-text-secondary)",
                fontSize: 13,
              }}
            >
              {description}
            </p>
          )}
        </div>
      )}
      {children}
    </section>
  );
}

/* --- notices -------------------------------------------------------------- */

/** A muted note strip: context the admin needs, not a warning. */
export function Note({
  children,
  icon,
  testId,
}: {
  children: ReactNode;
  icon?: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <div
      data-testid={testId}
      style={{
        display: "flex",
        gap: 10,
        padding: "12px 14px",
        borderRadius: 12,
        background: "var(--m-surface-2)",
        fontSize: 13,
        lineHeight: 1.5,
        color: "var(--m-text)",
      }}
    >
      {icon}
      <span style={{ minWidth: 0 }}>{children}</span>
    </div>
  );
}

/**
 * The failed-save banner. Blush, never red: nothing is wrong with the input,
 * the request did not land. It says what happened and that the work is still
 * in the drawer, and it carries the retry.
 */
export function SaveFailedBanner({
  children,
  onRetry,
  testId,
}: {
  children: ReactNode;
  onRetry?: () => void;
  testId?: string;
}): React.JSX.Element {
  return (
    <div
      role="alert"
      data-testid={testId ?? "settings-save-failed"}
      style={{
        display: "flex",
        gap: 10,
        padding: "12px 14px",
        borderRadius: 12,
        background: "var(--m-status-due-bg)",
        color: "var(--m-status-due-fg)",
        fontSize: 13,
        lineHeight: 1.5,
      }}
    >
      <AlertIcon />
      <span style={{ flex: 1, minWidth: 0 }}>{children}</span>
      {onRetry === undefined ? null : (
        <Button
          variant="secondary"
          onClick={onRetry}
          style={{ height: 32, background: "var(--m-surface)", color: "var(--m-text)" }}
        >
          Try again
        </Button>
      )}
    </div>
  );
}

/** The blush notice the import drawer uses for rows it will skip. */
export function SkippedNotice({ children }: { children: ReactNode }): React.JSX.Element {
  return (
    <div
      style={{
        display: "flex",
        gap: 10,
        padding: "12px 14px",
        borderRadius: 12,
        background: "var(--m-status-due-bg)",
        color: "var(--m-status-due-fg)",
        fontSize: 13,
        lineHeight: 1.5,
      }}
    >
      <AlertIcon />
      <span>{children}</span>
    </div>
  );
}

export function AlertIcon(): React.JSX.Element {
  return (
    <span style={{ display: "inline-flex", flexShrink: 0, marginTop: 1 }}>
      <Icon name="alert" size={16} />
    </span>
  );
}

export function CheckIcon({ color = "var(--m-status-success-fg)" }: { color?: string }): React.JSX.Element {
  return (
    <span style={{ display: "inline-flex", flexShrink: 0, color }}>
      <Icon name="check" size={16} />
    </span>
  );
}

/* --- staged changes ------------------------------------------------------- */

/**
 * The save bar for a table of toggles: it counts the staged changes, says
 * that nothing has changed for anyone yet, and offers Discard and Save. It is
 * only rendered while something is staged, because a save bar that is always
 * there is a bar nobody reads.
 */
export function StagedSaveBar({
  count,
  onDiscard,
  onSave,
  saving = false,
  noun = "change",
}: {
  count: number;
  onDiscard: () => void;
  onSave: () => void;
  saving?: boolean;
  noun?: string;
}): React.JSX.Element | null {
  if (count <= 0) return null;
  return (
    <div
      role="region"
      aria-label="Unsaved changes"
      data-testid="settings-staged-bar"
      style={{
        position: "sticky",
        bottom: 0,
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "12px 16px",
        borderRadius: 14,
        background: "var(--m-surface)",
        border: "1px solid var(--m-border)",
        boxShadow: "var(--m-shadow)",
        marginTop: 16,
      }}
    >
      <b style={{ color: "var(--m-text)", fontWeight: 700 }}>
        {count} unsaved {noun}
        {count === 1 ? "" : "s"}
      </b>
      <span style={{ color: "var(--m-text-secondary)", fontSize: 13 }}>
        Nothing has changed for anyone yet.
      </span>
      <span style={{ flex: 1 }} />
      <Button variant="secondary" onClick={onDiscard} disabled={saving}>
        Discard
      </Button>
      <Button variant="primary" onClick={onSave} disabled={saving}>
        {saving ? "Saving…" : "Save changes"}
      </Button>
    </div>
  );
}

/**
 * Selection replacing the column header in place. Destructive is an OUTLINED
 * red, never a filled one — the filled red is reserved for the confirm that
 * follows it.
 */
export function BulkActionBar({
  count,
  children,
  onClear,
  testId,
}: {
  count: number;
  children: ReactNode;
  onClear: () => void;
  testId?: string;
}): React.JSX.Element {
  return (
    <div
      data-testid={testId ?? "settings-bulk-bar"}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        minHeight: 40,
        padding: "0 12px",
        background: "var(--m-accent)",
        color: "var(--m-accent-text)",
        borderBottom: "1px solid var(--m-border)",
        position: "sticky",
        top: 0,
        zIndex: 5,
      }}
    >
      <b style={{ fontWeight: 700, marginRight: 6 }}>{count} selected</b>
      {children}
      <button
        type="button"
        onClick={onClear}
        style={{
          marginLeft: "auto",
          background: "transparent",
          border: "none",
          color: "inherit",
          fontWeight: 600,
          textDecoration: "underline",
          fontSize: 13,
          cursor: "pointer",
        }}
      >
        Clear selection
      </button>
    </div>
  );
}

/** The outlined red action inside a bulk bar. */
export function BulkDestructiveAction({
  onClick,
  children,
  testId,
}: {
  onClick: () => void;
  children: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <Button
      // The OUTLINED red: the filled red belongs to the confirm that follows.
      variant="destructive"
      onClick={onClick}
      data-testid={testId}
      style={{
        height: 32,
        fontSize: 13,
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
      }}
    >
      <TrashIcon />
      {children}
    </Button>
  );
}

export function TrashIcon(): React.JSX.Element {
  return <Icon name="trash" size={15} />;
}

/* --- fields --------------------------------------------------------------- */

export interface FormFieldProps {
  label: string;
  htmlFor?: string;
  /** The hint under an untouched, valid field. */
  hint?: ReactNode;
  /** A submit-time error. Blush, with an icon and words — never red, never
   * on keystroke, and the primary button is never disabled to hint at it. */
  error?: string | null;
  children: ReactNode;
  testId?: string;
}

export function FormField({
  label,
  htmlFor,
  hint,
  error,
  children,
  testId,
}: FormFieldProps): React.JSX.Element {
  return (
    <div data-testid={testId}>
      <label
        htmlFor={htmlFor}
        style={{ display: "block", fontSize: 14, fontWeight: 600, color: "var(--m-text)" }}
      >
        {label}
      </label>
      <div style={{ marginTop: 6 }}>{children}</div>
      {error === null || error === undefined ? (
        hint === undefined ? null : (
          <p style={{ margin: "6px 0 0", fontSize: 13, color: "var(--m-text-secondary)" }}>
            {hint}
          </p>
        )
      ) : (
        <p
          role="alert"
          data-testid={`${testId ?? "field"}-error`}
          style={{
            margin: "6px 0 0",
            fontSize: 13,
            fontWeight: 600,
            color: "var(--m-status-due-fg)",
            display: "flex",
            alignItems: "center",
            gap: 6,
          }}
        >
          <AlertIcon />
          {error}
        </p>
      )}
    </div>
  );
}

/** The text input the drawer form family uses, in its valid and error forms. */
export function FieldInput({
  id,
  value,
  onChange,
  invalid = false,
  placeholder,
  testId,
}: {
  id?: string;
  value: string;
  onChange: (next: string) => void;
  invalid?: boolean;
  placeholder?: string;
  testId?: string;
}): React.JSX.Element {
  return (
    <input
      id={id}
      type="text"
      value={value}
      placeholder={placeholder}
      aria-invalid={invalid || undefined}
      data-testid={testId}
      onChange={(event) => onChange(event.target.value)}
      style={{
        width: "100%",
        height: 44,
        borderRadius: 10,
        // Validation is BLUSH (rule 2: overdue, at risk and validation errors
        // are blush, never red), and 2px so the state is visible without
        // colour.
        border: `2px solid ${invalid ? "var(--m-status-due-fg)" : "var(--m-input)"}`,
        background: "var(--m-surface)",
        color: "var(--m-text)",
        padding: invalid ? "0 11px" : "0 12px",
        font: "inherit",
      }}
    />
  );
}

/** The three-way Kind choice the status drawer draws. */
export function SegmentedChoice<T extends string>({
  value,
  options,
  onChange,
  label,
  testId,
  labelFor,
}: {
  value: T;
  options: readonly T[];
  onChange: (next: T) => void;
  label: string;
  testId?: string;
  /** What each option says. A stored value is not a word a person reads, so
   * a caller whose values are slugs (`in_progress`) passes the drawn words
   * here rather than shipping the slug to the screen. */
  labelFor?: (option: T) => string;
}): React.JSX.Element {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      data-testid={testId}
      style={{ display: "inline-flex", gap: 4, padding: 3, borderRadius: 12, background: "var(--m-surface-2)", alignSelf: "flex-start" }}
    >
      {options.map((option) => {
        const active = option === value;
        return (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option)}
            style={{
              height: 36,
              padding: "0 16px",
              border: "none",
              borderRadius: 9,
              background: active ? "var(--m-surface)" : "transparent",
              color: active ? "var(--m-text)" : "var(--m-text-secondary)",
              fontWeight: active ? 600 : 500,
              cursor: "pointer",
            }}
          >
            {option}
          </button>
        );
      })}
    </div>
  );
}

/* --- the drawer ----------------------------------------------------------- */

/**
 * The 480px drawer: a header with the page's title and one line about what the
 * form governs, a scrolling body, and a footer with Cancel and the primary.
 * Esc closes it, and the page behind it is dimmed.
 */
export function SettingsDrawer({
  title,
  description,
  children,
  footer,
  onClose,
  labelledBy,
  testId,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  labelledBy?: string;
  testId?: string;
}): React.JSX.Element {
  const headingId = useId();
  const panel = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div style={{ position: "fixed", top: APP_BAR, right: 0, bottom: 0, left: 0, zIndex: 61 }}>
      <button
        type="button"
        aria-label="Close"
        tabIndex={-1}
        onClick={onClose}
        style={{
          position: "absolute",
          inset: 0,
          background: "var(--m-scrim)",
          border: "none",
          cursor: "default",
        }}
      />
      <aside
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy ?? headingId}
        data-testid={testId ?? "settings-drawer"}
        style={{
          position: "absolute",
          right: 0,
          top: 0,
          bottom: 0,
          width: DRAWER_WIDTH,
          maxHeight: "100%",
          maxWidth: "100%",
          background: "var(--m-surface)",
          borderLeft: "1px solid var(--m-border)",
          display: "flex",
          flexDirection: "column",
          color: "var(--m-text)",
          boxShadow: "var(--m-shadow-overlay)",
        }}
      >
        <div
          style={{
            padding: "20px 24px",
            borderBottom: "1px solid var(--m-border)",
            display: "flex",
            alignItems: "flex-start",
            gap: 12,
          }}
        >
          <div>
            <h2 id={headingId} style={{ margin: 0, fontSize: 20, fontWeight: 600 }}>
              {title}
            </h2>
            {description === undefined ? null : (
              <p style={{ margin: "2px 0 0", color: "var(--m-text-secondary)", fontSize: 13 }}>
                {description}
              </p>
            )}
          </div>
          <span style={{ flex: 1 }} />
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            style={{
              width: 36,
              height: 36,
              borderRadius: 10,
              border: "1px solid var(--m-border)",
              background: "transparent",
              color: "var(--m-text)",
              cursor: "pointer",
            }}
          >
            <Icon name="x" size={14} />
          </button>
        </div>
        <div
          style={{
            padding: "20px 24px",
            display: "flex",
            flexDirection: "column",
            gap: 18,
            flex: 1,
            overflow: "auto",
          }}
        >
          {children}
        </div>
        <div
          style={{
            padding: "16px 24px",
            borderTop: "1px solid var(--m-border)",
            display: "flex",
            gap: 10,
            background: "var(--m-canvas)",
          }}
        >
          {footer}
        </div>
      </aside>
    </div>
  );
}

/** The drawer's footer: Cancel at the left, the primary at the right. */
export function DrawerFooter({
  onCancel,
  children,
}: {
  onCancel: () => void;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <>
      <Button variant="secondary" onClick={onCancel}>
        Cancel
      </Button>
      <span style={{ flex: 1 }} />
      {children}
    </>
  );
}

/* --- the destructive confirm --------------------------------------------- */

/**
 * The confirm the pack draws: the title names the record, the consequence is
 * stated, a remap is asked for when the product has somewhere to put things,
 * and the confirm button says exactly what it will do. It is the only filled
 * red on the screen.
 */
export function ConfirmRemap({
  open,
  title,
  irreversible = "This can't be undone.",
  children,
  footerNote,
  confirmLabel,
  options,
  onConfirm,
  onCancel,
  busy = false,
  testId,
}: {
  open: boolean;
  title: string;
  irreversible?: string;
  children?: ReactNode;
  /** The line that survives the action, e.g. "History keeps the old name." */
  footerNote?: string;
  confirmLabel: string;
  /** Remap targets. Omit when the product has nowhere to move things. */
  options?: { label: string; value: string }[];
  onConfirm: (remapTo: string | null) => void;
  onCancel: () => void;
  busy?: boolean;
  testId?: string;
}): React.JSX.Element | null {
  const [remap, setRemap] = useState<string>(options?.[0]?.value ?? "");
  const headingId = useId();

  useEffect(() => {
    if (open) setRemap(options?.[0]?.value ?? "");
  }, [open, options]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onCancel]);

  if (!open) return null;
  return (
    <div
      style={{
        position: "fixed",
        top: APP_BAR,
        right: 0,
        bottom: 0,
        left: 0,
        zIndex: 70,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 24,
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          background: "var(--m-scrim)",
        }}
        onClick={busy ? undefined : onCancel}
      />
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={headingId}
        data-testid={testId ?? "settings-confirm"}
        style={{
          position: "relative",
          width: CONFIRM_WIDTH,
          maxWidth: "calc(100% - 32px)",
          maxHeight: "calc(100% - 48px)",
          overflow: "auto",
          background: "var(--m-surface)",
          borderRadius: 20,
          color: "var(--m-text)",
          boxShadow: "var(--m-shadow-overlay)",
        }}
      >
        <div style={{ padding: "20px 26px", borderBottom: "1px solid var(--m-border)" }}>
          <h2 id={headingId} style={{ margin: 0, fontSize: 20, fontWeight: 600 }}>
            {title}
          </h2>
          <p style={{ margin: "2px 0 0", color: "var(--m-text-secondary)", fontSize: 13 }}>
            {irreversible}
          </p>
        </div>
        <div
          style={{
            padding: "18px 26px",
            display: "flex",
            flexDirection: "column",
            gap: 14,
          }}
        >
          {children}
          {options === undefined ? null : (
            <label style={{ display: "flex", flexDirection: "column", gap: 6, fontSize: 14, fontWeight: 600 }}>
              <select
                value={remap}
                onChange={(event) => setRemap(event.target.value)}
                style={{
                  height: 44,
                  borderRadius: 10,
                  border: "1px solid var(--m-input)",
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                  padding: "0 12px",
                  font: "inherit",
                  fontWeight: 400,
                }}
              >
                {options.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>
        <div
          style={{
            padding: "16px 26px",
            borderTop: "1px solid var(--m-border)",
            display: "flex",
            alignItems: "center",
            gap: 10,
            background: "var(--m-canvas)",
          }}
        >
          {footerNote === undefined ? null : (
            <span style={{ color: "var(--m-text-secondary)", fontSize: 13, marginRight: "auto" }}>
              {footerNote}
            </span>
          )}
          <Button variant="secondary" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant="destructive-solid"
            onClick={() => onConfirm(options === undefined ? null : remap)}
            disabled={busy}
          >
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}

/* --- danger zone ---------------------------------------------------------- */

/** A small label and a card set apart from everything above it. */
export function DangerZone({
  label = "Danger zone",
  children,
}: {
  label?: string;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <div style={{ marginTop: 22 }}>
      <div
        style={{
          fontSize: 12,
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          fontWeight: 600,
          color: "var(--m-text-secondary)",
          marginBottom: 8,
        }}
      >
        {label}
      </div>
      <section
        style={{
          border: "1px solid var(--m-border)",
          borderRadius: 16,
          background: "var(--m-surface)",
          overflow: "hidden",
        }}
      >
        {children}
      </section>
    </div>
  );
}

/** The outlined red action inside a danger zone. */
export function DangerAction({
  onClick,
  children,
  testId,
}: {
  onClick: () => void;
  children: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <Button
      variant="secondary"
      onClick={onClick}
      data-testid={testId}
      style={{
        borderColor: "var(--m-status-danger-fg)",
        color: "var(--m-status-danger-fg)",
        display: "inline-flex",
        alignItems: "center",
        gap: 8,
        flexShrink: 0,
      }}
    >
      <TrashIcon />
      {children}
    </Button>
  );
}

/* --- import review -------------------------------------------------------- */

export interface ImportRow {
  /** Stable key for the row. */
  key: string;
  name: string;
  kind: string;
  /** False when the row will be skipped, with the reason. */
  ok: boolean;
  /** Why the row is skipped, in the words the drawer shows. */
  reason?: string;
}

/**
 * The import review: the file chip, then what we found, row by row. A valid
 * row carries a check; a skipped row is blush and says why; the primary states
 * the count it is about to act on.
 */
export function ImportReview({
  fileName,
  rowCount,
  rows,
  onChooseAnother,
  skipNote,
}: {
  fileName: string;
  rowCount: number;
  rows: ImportRow[];
  onChooseAnother: () => void;
  skipNote: ReactNode;
}): React.JSX.Element {
  return (
    <>
      <Note
        icon={<CheckIcon />}
        testId="settings-import-file"
      >
        <b style={{ fontWeight: 700 }}>{fileName}</b> · {rowCount} rows read.{" "}
        <button
          type="button"
          onClick={onChooseAnother}
          style={{
            background: "none",
            border: "none",
            padding: 0,
            color: "var(--m-text)",
            fontWeight: 600,
            textDecoration: "underline",
            fontSize: 13,
            cursor: "pointer",
          }}
        >
          Choose another
        </button>
      </Note>
      <div>
        <span style={{ fontSize: 14, fontWeight: 600 }}>What we found</span>
        <div
          style={{
            marginTop: 6,
            border: "1px solid var(--m-border)",
            borderRadius: 12,
            overflow: "hidden",
          }}
        >
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "36px 1fr 90px",
              alignItems: "center",
              minHeight: 36,
              background: "var(--m-canvas)",
              borderBottom: "1px solid var(--m-border)",
            }}
          >
            <span />
            <span
              style={{
                fontSize: 12,
                letterSpacing: "0.06em",
                textTransform: "uppercase",
                fontWeight: 600,
                color: "var(--m-text-secondary)",
              }}
            >
              Name
            </span>
            <span
              style={{
                fontSize: 12,
                letterSpacing: "0.06em",
                textTransform: "uppercase",
                fontWeight: 600,
                color: "var(--m-text-secondary)",
              }}
            >
              Kind
            </span>
          </div>
          {rows.map((row) => (
            <div
              key={row.key}
              data-testid={`settings-import-row-${row.key}`}
              style={{
                display: "grid",
                gridTemplateColumns: "36px 1fr 90px",
                alignItems: "center",
                minHeight: 44,
                padding: "0 0",
                borderTop: row.ok ? "1px solid var(--m-border)" : "none",
                background: row.ok ? "var(--m-surface)" : "var(--m-status-due-bg)",
                color: row.ok ? "var(--m-text)" : "var(--m-status-due-fg)",
              }}
            >
              <span style={{ paddingLeft: 10 }}>
                {row.ok ? (
                  <CheckIcon />
                ) : (
                  <span style={{ color: "var(--m-status-due-fg)" }}>
                    <AlertIcon />
                  </span>
                )}
              </span>
              <span style={{ fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {row.name}
              </span>
              <span>{row.kind}</span>
            </div>
          ))}
        </div>
      </div>
      <SkippedNotice>{skipNote}</SkippedNotice>
    </>
  );
}

/** The drop zone the import drawer opens with. */
export function ImportDropZone({
  onChoose,
  onDownloadTemplate,
}: {
  onChoose: () => void;
  onDownloadTemplate: () => void;
}): React.JSX.Element {
  return (
    <div
      style={{
        border: "2px dashed var(--m-input)",
        borderRadius: 14,
        padding: 22,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 8,
        textAlign: "center",
        color: "var(--m-text-secondary)",
      }}
    >
      <b style={{ color: "var(--m-text)" }}>Drop a CSV here</b>
      <span>or</span>
      <Button variant="secondary" onClick={onChoose}>
        Choose a file
      </Button>
      <button
        type="button"
        onClick={onDownloadTemplate}
        style={{
          background: "none",
          border: "none",
          padding: 0,
          color: "var(--m-text)",
          fontWeight: 600,
          textDecoration: "underline",
          fontSize: 13,
          cursor: "pointer",
        }}
      >
        Download the template
      </button>
    </div>
  );
}

/** The import drawer's primary, which states the count it will act on. */
export function ImportPrimary({
  validCount,
  skipCount,
  onConfirm,
  testId,
}: {
  validCount: number;
  skipCount: number;
  onConfirm: () => void;
  testId?: string;
}): React.JSX.Element {
  return (
    <Button variant="primary" onClick={onConfirm} data-testid={testId}>
      {skipCount === 0
        ? `Import ${validCount}`
        : `Import ${validCount}, skip ${skipCount}`}
    </Button>
  );
}

/* --- the confidence bar, shared by the AI pages --------------------------- */

/** The confidence bar with its threshold tick: the same tick in the list and
 * in the drawer, so moving one is visibly moving the other. */
export function ThresholdBar({
  value,
  threshold,
  width = 120,
}: {
  value: number;
  threshold: number;
  width?: number;
}): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      style={{
        position: "relative",
        height: 8,
        borderRadius: 999,
        background: "var(--m-surface-2)",
        width,
        flexShrink: 0,
        display: "inline-block",
      }}
    >
      <span
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          bottom: 0,
          width: `${Math.max(0, Math.min(1, value)) * 100}%`,
          borderRadius: 999,
          background: value >= threshold ? "var(--m-primary)" : "var(--m-input)",
        }}
      />
      <span
        style={{
          position: "absolute",
          top: -3,
          bottom: -3,
          left: `${Math.max(0, Math.min(1, threshold)) * 100}%`,
          width: 2,
          background: "var(--m-text)",
          borderRadius: 1,
        }}
      />
    </span>
  );
}

/** The four figures the run log leads with. */
export function Kpi({ value, label }: { value: string; label: string }): React.JSX.Element {
  return (
    <div
      style={{
        border: "1px solid var(--m-border)",
        borderRadius: 14,
        background: "var(--m-surface)",
        padding: "14px 16px",
      }}
    >
      <div
        style={{
          fontSize: 24,
          fontWeight: 600,
          lineHeight: 1.2,
          color: "var(--m-text)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </div>
      <div style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>{label}</div>
    </div>
  );
}

/** The promote card at the end of the run log: evidence in one sentence, and
 * the statement that a person still approves every write. */
export function PromoteCard({
  title,
  children,
  actions,
  promoted = false,
}: {
  title: string;
  children: ReactNode;
  actions: ReactNode;
  promoted?: boolean;
}): React.JSX.Element {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "16px 18px",
        borderRadius: 16,
        border: `2px solid ${promoted ? "var(--m-border)" : "var(--m-status-success-fg)"}`,
        background: promoted ? "var(--m-surface)" : "var(--m-status-success-bg)",
        marginTop: 14,
      }}
    >
      <span style={{ flex: 1, minWidth: 0 }}>
        <b
          style={{
            display: "block",
            fontSize: 15,
            fontWeight: 600,
            color: promoted ? "var(--m-text)" : "var(--m-status-success-fg)",
          }}
        >
          {title}
        </b>
        <span
          style={{
            display: "block",
            fontSize: 13,
            lineHeight: 1.5,
            color: promoted ? "var(--m-text-secondary)" : "var(--m-status-success-fg)",
          }}
        >
          {children}
        </span>
      </span>
      {actions}
    </div>
  );
}
