"use client";
/**
 * Toaster — Wave 41 lane 2b: the ink toast with a gold "Undo".
 *
 * Purpose: transient confirmation and, more importantly, transient
 * RECOVERY. Anything destructive this product does in one gesture gets an
 * undo, and an undo rendered in the body's ink on a cream toast is not an
 * undo anybody reaches for — so the toast is ink in every theme and the undo
 * is the one gold thing in it.
 *
 * Non-purpose: this is not the notification system. A notification is a
 * record a user can come back to; a toast is a moment that is over. The dock
 * owns the first, this owns the second, and a toast that carries information
 * nobody can retrieve afterwards is a bug in the caller.
 */
import { Toaster as SonnerToaster, toast } from "sonner";
import { TOAST_CLASS } from "../kit";

export { toast };

/** The Marula toast surface. Mounted once, near the root of the app shell.
 *  The look comes from the kit's `TOAST_CLASS`, not a local stylesheet. */
export function Toaster(): React.JSX.Element {
  return (
    <SonnerToaster
      position="bottom-center"
      closeButton
      toastOptions={{ classNames: { toast: TOAST_CLASS } }}
    />
  );
}
