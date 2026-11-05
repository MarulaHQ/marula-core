"use client";
/**
 * AiGateNotice — one typed "not available yet" state for an AI refusal.
 *
 * The admission path refuses before any model runs and says why with a stable
 * reason code (`ai_features_disabled`, `ai_surface_not_granted`, a cap, a
 * missing credential, a model that is not allowlisted). That is a different
 * thing from a draft the registry rejected, so it gets its own state: the
 * reason in a sentence, the one remedy beside it, and no Generate-shaped dead
 * end.
 *
 * The copy for every reason lives in ONE map (`lib/error-copy`, `aiGateCopy`),
 * because /reports and /dashboards both reach this component and must not
 * describe the same refusal differently.
 *
 * Non-purpose: no retry button. Retrying an org-wide switch, a grant or a cap
 * changes nothing the caller can reach; the remedy names the person who can.
 */
import { Callout, Icon } from "@marula-core/ui/kit";
import { aiGateCopy } from "../../lib/error-copy";

export interface AiGateNoticeProps {
  /** The typed reason from the refusal envelope (`outcome.refusal`). */
  reason: string;
  /** The server's own sentence, used when it is not an AI admission reason. */
  message?: string | null;
  /** Which entry is blocked, so a test and a reader know where it came from. */
  surface?: "reports" | "dashboards";
}

export function AiGateNotice({ reason, message, surface }: AiGateNoticeProps) {
  const gate = aiGateCopy(reason);
  const title = gate?.title ?? message ?? "The draft is not available right now.";
  const remedy =
    gate?.remedy ??
    "Check the value, then try again. Nothing was saved.";

  return (
    <div
      className="grid gap-1"
      data-testid="ai-gate-notice"
      data-reason={reason}
      data-surface={surface}
      role="alert"
    >
      <Callout icon={<Icon name="alert" size={14} />}>
        <span className="block text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
          {title}
        </span>
        <span className="block text-[13px] leading-[1.5]" style={{ color: "var(--m-text-secondary)" }}>
          {remedy}
        </span>
      </Callout>
    </div>
  );
}
