// Shared purchasing vocabulary (Wave 32 lane 4): one tone and one word per
// purchase-order status and per line receipt state, so the browser and the
// order detail never disagree about what "partially received" looks like.
"use client";

import {
  Badge,
} from "@marula-core/ui";
import type { ReactNode } from "react";

import type { Tone } from "../../lib/stock-format";

export function orderStatusTone(status: string): Tone {
  switch (status) {
    case "ordered":
    case "partially_received":
      return "warning";
    case "received":
    case "closed":
      return "success";
    case "cancelled":
      return "danger";
    default:
      return "neutral";
  }
}

export function orderStatusLabel(status: string): string {
  return status.replace(/_/g, " ");
}

export function receiptStateTone(state: string): Tone {
  switch (state) {
    case "complete":
      return "success";
    case "partial":
    case "over":
      return "warning";
    default:
      return "neutral";
  }
}

export function OrderStatusTag({ status }: { status: string }): ReactNode {
  return <Badge tone={orderStatusTone(status)}>{orderStatusLabel(status)}</Badge>;
}

export function ReceiptStateTag({ state }: { state: string }): ReactNode {
  return <Badge tone={receiptStateTone(state)}>{state}</Badge>;
}
