import * as React from "react"
import { cn } from "../../lib/cn"
import { Icon } from "../icons/Icon"

type Size = "default" | "lg"
const control = {
  default: "h-[44px] rounded-field px-3 data-[invalid=true]:px-[11px]",
  lg: "h-12 rounded-ctl px-[14px] data-[invalid=true]:px-[13px]",
} as const
const controlBase = "w-full border border-input bg-card text-foreground placeholder:text-muted-foreground data-[invalid=true]:border-2 data-[invalid=true]:border-mc-p2"

export function Input({ className, size = "default", invalid, ...props }: Omit<React.ComponentProps<"input">, "size"> & { size?: Size; invalid?: boolean }) {
  return <input data-slot="input" data-invalid={invalid || undefined} aria-invalid={invalid || undefined} className={cn(controlBase, control[size], className)} {...props} />
}
export function Select({ className, size = "default", invalid, ...props }: Omit<React.ComponentProps<"select">, "size"> & { size?: Size; invalid?: boolean }) {
  return <select data-slot="select" data-invalid={invalid || undefined} aria-invalid={invalid || undefined} className={cn(controlBase, control[size], className)} {...props} />
}
export function Textarea({ className, invalid, ...props }: React.ComponentProps<"textarea"> & { invalid?: boolean }) {
  return <textarea data-slot="textarea" data-invalid={invalid || undefined} aria-invalid={invalid || undefined} className={cn(controlBase, "h-[76px] resize-none rounded-field px-3 py-2.5 data-[invalid=true]:px-[11px]", className)} {...props} />
}

/**
 * Label, control, hint and error in one place. The error is BLUSH, never red, says what to do, and is announced.
 *   <Field label="Work email" error="Enter the address your admin invited."><Input /></Field>
 */
export function Field({ label, hint, error, errorIcon, children, className }: { label: string; hint?: React.ReactNode; error?: React.ReactNode; errorIcon?: React.ReactNode; children: React.ReactElement<Record<string, unknown>>; className?: string }) {
  const id = React.useId()
  const hintId = hint ? `${id}-hint` : undefined
  const errId = error ? `${id}-err` : undefined
  const control = React.cloneElement(children, { id, invalid: !!error || undefined, "aria-describedby": [errId, hintId].filter(Boolean).join(" ") || undefined })
  return (
    <div data-slot="field" className={cn("flex flex-col gap-1.5", className)}>
      <label htmlFor={id} className="text-[14px] font-semibold">{label}</label>
      {control}
      {error ? (
        <div id={errId} role="alert" className="flex items-center gap-1.5 text-[13px] font-semibold text-mc-attn-fg">
          {errorIcon ?? <Icon name="alert" size={14} />}
          <span>{error}</span>
        </div>
      ) : hint ? (
        <div id={hintId} className="text-[13px] text-muted-foreground">{hint}</div>
      ) : null}
    </div>
  )
}
