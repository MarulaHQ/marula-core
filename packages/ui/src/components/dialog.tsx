/**
 * Dialog — Radix dialog with scrim + bordered overlay surface. Shadows are
 * reserved for overlays like this one (§5).
 */
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { forwardRef, type ComponentProps, type ReactNode } from "react";
import { cn } from "../lib/cn";

export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;

export type DialogContentProps = ComponentProps<typeof DialogPrimitive.Content> & {
  /** Accessible title; rendered as the dialog heading. */
  title: string;
  /** Optional secondary description wired via aria-describedby. */
  description?: string;
};

export const DialogContent = forwardRef<HTMLDivElement, DialogContentProps>(
  ({ className, title, description, children, ...props }, ref) => (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="m-dialog-overlay" />
      <DialogPrimitive.Content
        ref={ref}
        aria-describedby={description ? "m-dialog-description" : undefined}
        className={cn("m-dialog-content", className)}
        {...props}
      >
        <DialogPrimitive.Title className="m-dialog-title">{title}</DialogPrimitive.Title>
        {description ? (
          <DialogPrimitive.Description id="m-dialog-description" className="m-dialog-description">
            {description}
          </DialogPrimitive.Description>
        ) : null}
        <DialogPrimitive.Close className="m-dialog-close" aria-label="Close">
          <svg aria-hidden="true" width="12" height="12" viewBox="0 0 16 16" fill="none">
            <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
        </DialogPrimitive.Close>
        {children}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  ),
);

DialogContent.displayName = "DialogContent";

/** Right-aligned action row for dialog footers. */
export function DialogActions({ children }: { children: ReactNode }): ReactNode {
  return (
    <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 16 }}>
      {children}
    </div>
  );
}
