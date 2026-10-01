import * as React from "react"
import { Slot } from "radix-ui"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "../../lib/cn"

/**
 * Buttons.
 *  - outline   the default. Toolbars, secondary actions.
 *  - primary   the one action that matters on the screen. Ink fill, gold text (gold fill in dark).
 *  - secondary the stronger outline used on auth and portal screens.
 *  - danger    a FILLED red button. Only ever the confirm button of a destructive dialog. Red is never decoration.
 *  - strip     a white button that sits on a blush or green strip.
 * aria-expanded="true" turns any button gold-tint, which is how an open filter or menu button looks.
 */
export const buttonVariants = cva(
  "inline-flex shrink-0 cursor-pointer select-none items-center gap-2 whitespace-nowrap border text-[14px] font-semibold disabled:cursor-not-allowed aria-expanded:border-transparent aria-expanded:bg-mc-act aria-expanded:text-mc-act-fg",
  {
    variants: {
      variant: {
        outline: "border-border bg-transparent text-foreground hover:bg-muted",
        secondary: "border-input bg-transparent text-foreground hover:bg-muted",
        primary: "border-transparent bg-primary text-primary-foreground disabled:bg-muted disabled:text-muted-foreground",
        danger: "border-transparent bg-destructive text-destructive-foreground",
        strip: "border-transparent bg-card text-foreground hover:bg-muted",
      },
      size: {
        /** height follows density: 44 / 36 / 32 */
        default: "h-ctl rounded-ctl px-[14px]",
        sm: "h-[30px] rounded-ctl px-[14px] text-[13px]",
        /** the 48px full-width button on auth and portal screens */
        lg: "h-12 w-full justify-center rounded-ctl px-[14px] text-[16px]",
      },
    },
    defaultVariants: { variant: "outline", size: "default" },
  }
)

export type ButtonProps = React.ComponentProps<"button"> & VariantProps<typeof buttonVariants> & { asChild?: boolean }

export function Button({ className, variant, size, asChild = false, type, ...props }: ButtonProps) {
  const Comp = asChild ? Slot.Root : "button"
  return <Comp data-slot="button" type={asChild ? undefined : (type ?? "button")} className={cn(buttonVariants({ variant, size }), className)} {...props} />
}

/** Underlined text button. For "Forgot password?", "Show all 12", "Review all". */
export function LinkButton({ className, type = "button", ...props }: React.ComponentProps<"button">) {
  return <button data-slot="link-button" type={type} className={cn("cursor-pointer border-0 bg-transparent p-0 font-semibold text-foreground underline", className)} {...props} />
}

/** 40px square icon button (close, more, theme). Always give it an aria-label. */
export function IconButton({ className, type = "button", ...props }: React.ComponentProps<"button">) {
  return (
    <button
      data-slot="icon-button"
      type={type}
      className={cn("inline-flex size-10 shrink-0 cursor-pointer items-center justify-center rounded-[10px] border border-border bg-transparent text-foreground hover:bg-muted", className)}
      {...props}
    />
  )
}
