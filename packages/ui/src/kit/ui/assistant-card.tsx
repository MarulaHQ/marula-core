import * as React from "react"
import { cn } from "../../lib/cn"

/**
 * THE inverted surface. There is exactly one per screen, and it only exists when an AI grant does.
 * Ink on light themes, cream on dark. A recommendation, what it is based on, one primary action, two quiet ones.
 */
export function AssistantCard({ label = "Recommended next step", title, children, primary, secondary, className }: { label?: string; title: React.ReactNode; children?: React.ReactNode; primary?: React.ReactNode; secondary?: React.ReactNode[]; className?: string }) {
  return (
    <div data-slot="assistant-card" className={cn("flex flex-col gap-2.5 rounded-card bg-mc-assist p-4 text-mc-assist-fg", className)}>
      <div className="text-[12px] font-semibold uppercase tracking-[.08em] opacity-80">{label}</div>
      <div className="text-[15px] font-semibold leading-[1.4]">{title}</div>
      {children ? <p className="m-0 text-[13px] leading-[1.5] opacity-90">{children}</p> : null}
      {primary}
      {secondary?.length ? <div className="flex gap-2">{secondary}</div> : null}
    </div>
  )
}
/** The one primary action inside the assistant card. */
export function AssistantAction({ className, type = "button", ...props }: React.ComponentProps<"button">) {
  return <button type={type} className={cn("flex h-10 cursor-pointer items-center justify-center gap-2 rounded-[10px] border-0 bg-mc-assist-btn font-semibold text-mc-assist-btn-fg", className)} {...props} />
}
/** The two quiet actions underneath: "View evidence", "Not this". */
export function AssistantQuiet({ className, type = "button", ...props }: React.ComponentProps<"button">) {
  return <button type={type} className={cn("flex h-9 flex-1 cursor-pointer items-center justify-center rounded-[10px] border border-mc-assist-line bg-transparent text-[13px] font-semibold text-inherit", className)} {...props} />
}
