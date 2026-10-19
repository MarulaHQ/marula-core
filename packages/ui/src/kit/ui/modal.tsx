import * as React from "react"
import { Dialog as DialogPrimitive } from "radix-ui"
import { cn } from "../../lib/cn"
import { IconButton } from "./button"
import { Icon } from "../icons/Icon"

/**
 * The surface of a dialog, without any behaviour. Use it on its own for a static render, or through <DialogContent>.
 * Structure: header (title, close), body, footer (a hint on the left, the buttons on the right, on the page background).
 */
export function ModalSurface({ className, width = 760, style, ...props }: React.ComponentProps<"div"> & { width?: number }) {
  return <div data-slot="modal" style={{ width, maxWidth: "calc(100% - 32px)", ...style }} className={cn("max-h-[calc(100%-48px)] overflow-auto rounded-modal bg-card text-foreground shadow-mc", className)} {...props} />
}
export function ModalHeader({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex items-center gap-3 border-b border-border px-[26px] py-5", className)} {...props} />
}
export function ModalTitle({ className, ...props }: React.ComponentProps<"h2">) {
  return <h2 className={cn("m-0 text-[20px] font-semibold", className)} {...props} />
}
export function ModalBody({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex flex-col gap-3.5 px-[26px] py-[18px]", className)} {...props} />
}
export function ModalFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex items-center gap-2.5 border-t border-border bg-background px-[26px] py-4", className)} {...props} />
}
/** Left side of the footer: the sentence that says what happens next. */
export function ModalHint({ className, ...props }: React.ComponentProps<"span">) {
  return <span className={cn("mr-auto text-[13px] text-muted-foreground", className)} {...props} />
}

/** The compact confirm used for destructive and irreversible actions. Say what will happen, and what will not. */
export function ConfirmSurface({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="confirm" className={cn("flex flex-col gap-3 rounded-modal border border-border bg-card p-5 shadow-mc", className)} {...props} />
}

/* Behaviour: Radix Dialog, with our surface and scrim. */
export const Dialog = DialogPrimitive.Root
export const DialogTrigger = DialogPrimitive.Trigger
export const DialogClose = DialogPrimitive.Close
export const DialogTitle = DialogPrimitive.Title
export const DialogDescription = DialogPrimitive.Description

export function DialogContent({ className, children, width, ...props }: React.ComponentProps<typeof DialogPrimitive.Content> & { width?: number }) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-50 flex items-center justify-center bg-mc-scrim" />
      <div className="pointer-events-none fixed inset-0 z-50 flex items-center justify-center">
        <DialogPrimitive.Content className={cn("pointer-events-auto outline-none", className)} {...props}>
          <ModalSurface width={width}>{children}</ModalSurface>
        </DialogPrimitive.Content>
      </div>
    </DialogPrimitive.Portal>
  )
}
export function DialogCloseButton({ label = "Close" }: { label?: string }) {
  return (
    <DialogPrimitive.Close asChild>
      <IconButton aria-label={label}><Icon name="x" size={18} /></IconButton>
    </DialogPrimitive.Close>
  )
}
