"use client";
/**
 * Dialog — reimplemented (wave41 lane 20) over the handoff kit's modal
 * primitives, so every dialog gets the kit's surface, scrim, header, body and
 * footer instead of the old `.m-dialog-*` stylesheet.
 *
 * The call-site API is kept: `DialogContent` still takes `title` and an
 * optional `description`, and `DialogActions` is still the footer row.
 */
import type { ComponentProps, ReactNode } from "react";
import {
  Dialog as KitDialog,
  DialogClose as KitDialogClose,
  DialogCloseButton,
  DialogContent as KitDialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger as KitDialogTrigger,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../kit";

export const Dialog = KitDialog;
export const DialogTrigger = KitDialogTrigger;
export const DialogClose = KitDialogClose;

export type DialogContentProps = ComponentProps<typeof KitDialogContent> & {
  /** Accessible title; rendered as the dialog heading. */
  title: string;
  /** Optional secondary description wired via aria-describedby. */
  description?: string;
};

export function DialogContent({ className, title, description, children, ...props }: DialogContentProps) {
  return (
    <KitDialogContent className={className} {...props}>
      <ModalHeader>
        <DialogTitle className="m-0 text-[20px] font-semibold">{title}</DialogTitle>
        <div className="ml-auto">
          <DialogCloseButton />
        </div>
      </ModalHeader>
      <ModalBody>
        {description ? <DialogDescription className="m-0 text-muted-foreground">{description}</DialogDescription> : null}
        {children}
      </ModalBody>
    </KitDialogContent>
  );
}

/** Right-aligned action row for dialog footers. */
export function DialogActions({ children }: { children: ReactNode }): ReactNode {
  return <ModalFooter>{children}</ModalFooter>;
}
