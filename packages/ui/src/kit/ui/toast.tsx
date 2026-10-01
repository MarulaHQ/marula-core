import * as React from "react"
import { cn } from "../../lib/cn"

/**
 * The look of a toast: the inverted surface, a sentence, optionally an action. Two lines at most.
 * With Sonner:  <Toaster toastOptions={{ classNames: { toast: TOAST_CLASS } }} />
 */
export const TOAST_CLASS = "inline-flex items-center gap-3.5 rounded-ctl bg-mc-assist px-4 py-3 font-medium text-mc-assist-fg shadow-mc"
export function ToastSurface({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="toast" role="status" className={cn(TOAST_CLASS, className)} {...props} />
}
