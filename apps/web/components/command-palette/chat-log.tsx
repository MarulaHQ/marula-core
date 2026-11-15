// The conversation body shared by the palette's Ask mode and the chat panel.
//
// One renderer for both surfaces, so "Open as a panel" moves the same message
// state and nothing about an answer changes on the way. Sources sit under the
// answer they were used for; a proposal is dashed, says it waits for approval,
// and links to the agent platform's queue — it is never approved here.

"use client";

import { Button } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import Link from "next/link";

import type { ChatMessage } from "./store";

export function ChatLog({ messages, emptyHint }: { messages: ChatMessage[]; emptyHint: string }) {
  if (messages.length === 0) {
    return <p className="text-[13px] text-muted-foreground">{emptyHint}</p>;
  }
  return (
    <>
      {messages.map((message) => (
        <MessageBubble key={message.id} message={message} />
      ))}
    </>
  );
}

function MessageBubble({ message }: { message: ChatMessage }) {
  if (message.role === "user") {
    return (
      <div className="flex max-w-[88%] flex-col gap-1.5 self-end">
        <div className="whitespace-pre-wrap rounded-[14px] bg-mc-act px-3.5 py-2.5 text-sm leading-normal text-mc-act-fg">
          {message.text}
        </div>
      </div>
    );
  }
  return (
    <div className="flex max-w-[88%] flex-col gap-1.5">
      <div className="whitespace-pre-wrap rounded-[14px] bg-muted px-3.5 py-2.5 text-sm leading-normal">
        {message.text === "" && message.pending ? (
          <span className="text-muted-foreground">Thinking…</span>
        ) : (
          message.text
        )}
        {message.pending && message.text !== "" ? (
          <span
            aria-hidden="true"
            className="ml-0.5 inline-block h-3.5 w-1 animate-pulse bg-muted-foreground align-middle motion-reduce:animate-none"
          />
        ) : null}
      </div>
      {message.error !== null ? (
        <p
          role="alert"
          className="rounded-ctl px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
        >
          {message.error}
        </p>
      ) : null}
      {message.sources.length > 0 ? (
        <div className="flex flex-wrap gap-x-2.5 gap-y-1 text-xs text-muted-foreground">
          {message.sources.map((source, index) => (
            <span key={`${source.label}-${index}`} className="inline-flex items-center gap-1">
              <Icon name="info" size={12} />
              {source.label}
              {source.detail !== null && source.detail !== undefined ? ` · ${source.detail}` : ""}
            </span>
          ))}
        </div>
      ) : null}
      {message.proposals.map((proposal, index) => (
        <div
          key={`${proposal.proposalId ?? "proposal"}-${index}`}
          data-testid="palette-proposal"
          className="flex items-center gap-2.5 rounded-ctl border border-dashed border-input bg-mc-entry-agent px-3 py-2.5 text-[13px]"
        >
          <Icon name="mail" size={16} className="shrink-0" />
          <span className="min-w-0 flex-1">
            <span className="block font-semibold">{proposal.title}</span>
            <span className="block text-muted-foreground">{proposal.detail}</span>
          </span>
          <Button asChild size="sm">
            <Link href="/settings/agent-platform" title="Review proposals in the agent platform">
              Review
            </Link>
          </Button>
        </div>
      ))}
    </div>
  );
}
