"use client";
/**
 * ConfirmDialog — the destructive confirm (§9).
 *
 * Purpose: `Dialog` is a container; a destructive act needs one more thing —
 * a statement of what will be destroyed, a confirm that is the only action
 * carrying the destructive fill, and no dismissal route that pretends the act
 * happened. The dialog stays open while the request is in flight and renders
 * the server's refusal inside itself, because a confirm that closes on a
 * refused delete has told the operator the row is gone when it is not.
 *
 * The cancel control is the initial focus, so a hurried Return keeps the data.
 * Escape and the close control both cancel without calling `onConfirm`; Radix
 * returns focus to the trigger on close.
 */
import { useEffect, useState, type ReactNode } from "react";
import { Button } from "./button";
import { Dialog, DialogActions, DialogContent } from "./dialog";

export interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** What will happen, in the operator's terms. Required: no unnamed deletes. */
  description: string;
  confirmLabel: string;
  cancelLabel?: string;
  /** Runs the act. A rejection is rendered in place; the dialog stays open. */
  onConfirm: () => Promise<void>;
  testId?: string;
}

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  cancelLabel = "Cancel",
  onConfirm,
  testId,
}: ConfirmDialogProps): ReactNode {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // A reopened dialog must not carry the previous attempt's refusal.
  useEffect(() => {
    if (open) setError(null);
  }, [open]);

  const run = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (busy ? undefined : onOpenChange(next))}>
      <DialogContent title={title} description={description}>
        <div style={{ display: "grid", gap: 12 }} data-testid={testId}>
          {error !== null ? (
            <p
              role="alert"
              style={{
                margin: 0,
                padding: "8px 10px",
                borderRadius: "var(--m-radius-control)",
                background: "var(--m-status-danger-bg)",
                color: "var(--m-status-danger-fg)",
                fontSize: 13,
              }}
              data-testid="confirm-dialog-error"
            >
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button
              size="sm"
              variant="secondary"
              autoFocus
              onClick={() => onOpenChange(false)}
              disabled={busy}
              data-testid="confirm-dialog-cancel"
            >
              {cancelLabel}
            </Button>
            <Button
              size="sm"
              variant="destructive-solid"
              onClick={() => void run()}
              disabled={busy}
              data-testid="confirm-dialog-confirm"
            >
              {busy ? "Working…" : confirmLabel}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
