// Shared glyphs for the ticket workspace pods, drawn through the kit's one
// icon door so both themes track tokens automatically.
// Non-purpose: no bitmap or emoji icons.

import type { ReactNode } from "react";
import { Icon } from "@marula-core/ui/kit";

export function PaperclipIcon(): ReactNode {
  return <Icon name="paperclip" size={12} />;
}

export function CalendarIcon(): ReactNode {
  return <Icon name="calendar" size={12} />;
}
