"use client";
/**
 * ComposerFrame — Wave 41 lane 2b, from DESIGN-NOTES §4.
 *
 * Purpose: carry the composer's mode, and make the mode VISIBLE on the whole
 * composer rather than on a label somewhere inside it. The spec is explicit:
 * a reply to the client uses the plain surface, an internal note takes the
 * internal-note tint — the same tint internal timeline entries use, so one
 * glance at a screenshot tells you whether a person is writing to the client
 * or to their own team.
 *
 * The two modes and their exact copy:
 *
 * - `reply` — plain surface. "Visible to <client>". The primary button says
 *   Send reply, and a "Set to Waiting after send" checkbox is offered,
 *   because a sent reply usually means you are waiting on the client.
 * - `internal` — the note tint, no border. "Only your team can see this". The
 *   button says Add note and the checkbox is NOT offered: an internal note
 *   moves nothing, and offering the control would imply it might.
 *
 * Non-purpose: this does not send anything, and it does not know the
 * client's name. Both are props, because a surface that guesses either one
 * gets a "Visible to " with a blank after it, which is worse than no label.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export type ComposerMode = "reply" | "internal";

export interface ComposerFrameProps {
  mode: ComposerMode;
  /** The person the reply is visible to, for the reply label. */
  clientName?: string;
  /** The client's organisation, when the surface has one. */
  clientOrg?: string;
  /** Hide the "set to Waiting" affordance (a mode where it makes no sense). */
  offerWaitingMove?: boolean;
  waitingMove?: boolean;
  onWaitingMoveChange?: (next: boolean) => void;
  /** The composer's own children: the editor, the tools, the send control. */
  children?: ReactNode;
  className?: string;
}

export function ComposerFrame({
  mode,
  clientName,
  clientOrg,
  offerWaitingMove = true,
  waitingMove = false,
  onWaitingMoveChange,
  children,
  className,
}: ComposerFrameProps): React.JSX.Element {
  const internal = mode === "internal";
  return (
    <div
      className={cn(
        "m-composer",
        internal ? "m-composer-internal" : "m-composer-reply",
        className,
      )}
      data-composer-mode={mode}
    >
      <div className="m-composer-bar">
        <span className="m-composer-visibility">
          {internal ? "Only your team can see this" : `Visible to ${clientName ?? "the client"}`}
        </span>
        {clientOrg && !internal ? (
          <span className="m-composer-org">{clientOrg}</span>
        ) : null}
        {offerWaitingMove && !internal ? (
          <label className="m-composer-waiting">
            <input
              type="checkbox"
              checked={waitingMove}
              onChange={(e) => onWaitingMoveChange?.(e.target.checked)}
            />
            <span>Set to Waiting after send</span>
          </label>
        ) : null}
      </div>
      {children}
    </div>
  );
}

/**
 * The send control's label is part of the mode, not part of the verb: "Send
 * reply" tells the reader where it is going before they press it, and
 * "Add note" tells them it is going nowhere. Returning the string keeps the
 * choice in one place instead of at each call site.
 */
export function composerActionLabel(mode: ComposerMode): string {
  return mode === "internal" ? "Add note" : "Send reply";
}
