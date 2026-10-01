// Workbench v2 — the right context-aware Agent panel (Wave 15 lane 7).
//
// Purpose: the mockup's agent rail wired to the REAL agent/knowledge
// surfaces: knowledge sources from the live KB (title-keyword query),
// evidence cards from the unified search over this ticket's words, and a
// recommended next step from pending agent proposals filed against this
// ticket (rationale + skill). Every button performs a real action or
// renders a typed "not available yet" state — never fake data, never a
// dead button. The conversational chat surface has no live agent endpoint
// yet, so submits answer with the typed unavailable note while the real
// local capabilities (summarise from the ticket's own data, find related
// tickets through /search) work end-to-end.
// Non-purpose: no proposal approve/decline UX here (the agent platform
// pages own the review workflow).

"use client";

import type { components } from "@marula-core/sdk";
import { Icon } from "@marula-core/ui/kit";
import Link from "next/link";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { runChatTurn } from "../../lib/agent-chat";
import { announce } from "../a11y/live-region";
import { ModelPicker, useLazySurfacePick } from "../ai/model-picker";
import {
  AssistantCard,
  AssistantCardDismissed,
  AssistantCardQuestion,
  Button,
} from "@marula-core/ui";

type KbArticle = components["schemas"]["KbArticle"];
type SearchResult = components["schemas"]["SearchResult"];
type AgentProposalView = components["schemas"]["AgentProposalView"];
type TicketDetail = components["schemas"]["TicketWithFields"];
type TicketComment = components["schemas"]["TicketComment"];

interface ChatLine {
  id: string;
  role: "you" | "agent";
  text: string;
}

/** Title words worth searching by (shared with the Related tab). */
export function keywordsOf(title: string): string {
  const stop = new Set(["the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with", "is", "not", "every", "keeps"]);
  const words = title
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((w) => w.length > 2 && !stop.has(w));
  return words.slice(0, 4).join(" ");
}

export interface WbAgentPanelProps {
  ticket: TicketDetail;
  comments: TicketComment[];
  namesByUser: Map<string, string>;
  open: boolean;
  /**
   * Bumped by the workbench's "Take It" action; the panel refreshes its
   * live data load and reacts with ONE chat line pointing at the pending
   * proposal rationale or the top KB hit — the typed gap line otherwise.
   * Never fake analysis.
   */
  takeItNonce?: number;
  onClose(): void;
  /** Inserts a drafted customer update into the floating composer. */
  onDraftUpdate(body: string): void;
}

export function WbAgentPanel({ ticket, comments, namesByUser, open, onClose, takeItNonce, onDraftUpdate }: WbAgentPanelProps): ReactNode {
  const [articles, setArticles] = useState<KbArticle[] | null>(null);
  const [hits, setHits] = useState<SearchResult[] | null>(null);
  const [proposals, setProposals] = useState<AgentProposalView[] | null>(null);
  // The spec's assistant states, keyed by proposal id: "ran" once a person
  // approved it, "dismissed" once they said not this one. Keyed rather than a
  // single pair of booleans because the panel outlives any one proposal — a
  // new one must not inherit the last one's dismissal.
  const [proposalStates, setProposalStates] = useState<Record<string, "ran" | "dismissed">>({});
  const [deciding, setDeciding] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [chat, setChat] = useState<ChatLine[]>([]);
  const [chatDraft, setChatDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const chatInputRef = useRef<HTMLTextAreaElement | null>(null);
  const chatLogRef = useRef<HTMLDivElement | null>(null);

  // Keep the latest reply in view (chat log scrolls with the panel).
  const bumpLog = () => {
    requestAnimationFrame(() => chatLogRef.current?.scrollIntoView({ block: "nearest" }));
  };

  const load = useCallback(async () => {
    const q = keywordsOf(ticket.title);
    const [kb, search, props] = await Promise.all([
      q !== "" ? api.api.GET("/api/v1/kb/articles", { params: { query: { status: "published", q } } }) : Promise.resolve({ data: [] as KbArticle[] | undefined }),
      q !== "" ? api.api.GET("/api/v1/search", { params: { query: { q, limit: 10 } } }) : Promise.resolve({ data: undefined }),
      api.api.GET("/api/v1/agent-proposals", { params: { query: { limit: 200 } } }),
    ]);
    const freshArticles = (kb.data ?? []).slice(0, 3);
    const related = (search.data?.results ?? []).filter(
      (r) => r.entity_kind === "ticket" && r.entity_id !== ticket.id,
    );
    const freshHits = related.slice(0, 2);
    const freshProposals = (props.data?.proposals ?? []).filter((p) => p.entity === "ticket" && p.entity_id === ticket.id);
    setArticles(freshArticles);
    setHits(freshHits);
    setProposals(freshProposals);
    return { articles: freshArticles, hits: freshHits, proposals: freshProposals };
  }, [ticket.id, ticket.title]);

  useEffect(() => {
    if (!open) return;
    void load().catch(() => {
      setArticles([]);
      setHits([]);
      setProposals([]);
    });
  }, [open, load]);

  // "Take It" reaction: reload the real data, then one chat line with a
  // real-data pointer — pending proposal rationale, top KB hit, or the
  // typed gap. Never fake analysis.
  const lastTakeIt = useRef(0);
  useEffect(() => {
    if (takeItNonce === undefined || takeItNonce === 0) return;
    if (lastTakeIt.current === takeItNonce) return;
    lastTakeIt.current = takeItNonce;
    // The panel may be closed (narrow drawer) — react only when visible.
    if (!open) return;
    void (async () => {
      let loaded: { articles: KbArticle[]; hits: SearchResult[]; proposals: AgentProposalView[] };
      try {
        loaded = await load();
      } catch {
        respond("Not available yet — no agent triage proposal or matching KB article exists for this ticket. The panel lights up when either lands.");
        return;
      }
      const pendingFresh = loaded.proposals.find((p) => p.status === "proposed" && p.rationale != null) ?? null;
      const pointer =
        pendingFresh !== null
          ? `Agent triage worth a look (skill ${pendingFresh.skill_slug ?? "triage"}):\n${pendingFresh.rationale}`
          : loaded.articles[0] !== undefined
            ? `No agent triage is filed yet, but the top knowledge source points here:\n“${loaded.articles[0].title}”`
            : "Not available yet — no agent triage proposal or matching KB article exists for this ticket. The panel lights up when either lands.";
      respond(`Taken — you own ${ticket.number} and the timer is running.\n${pointer}`);
    })();
  }, [takeItNonce, open, load, ticket.number]);

  const pending = proposals?.find((p) => p.status === "proposed" && p.rationale != null) ?? null;
  const decided = proposals?.filter((p) => p.status !== "proposed").slice(0, 2) ?? [];

  const summarise = useCallback((): string => {
    const lines: string[] = [];
    lines.push(`${ticket.number} — ${ticket.title}`);
    lines.push(`Priority ${ticket.priority.toUpperCase()} · status category ${ticket.category}`);
    if (ticket.body != null && ticket.body !== "") lines.push(`Request: ${ticket.body}`);
    const last = comments[0];
    if (last !== undefined) {
      lines.push(
        `Latest note (${namesByUser.get(last.author_id ?? "") ?? "unknown"}): ${last.body.slice(0, 220)}${last.body.length > 220 ? "…" : ""}`,
      );
    }
    lines.push(`${comments.length} note(s) on the ticket.`);
    return lines.join("\n");
  }, [ticket, comments, namesByUser]);

  const pushYou = (text: string) => {
    setChat((prev) => [
      ...prev,
      { id: `c-${Date.now()}-you`, role: "you", text },
    ]);
  };

  const respond = (text: string) => {
    setChat((prev) => [...prev, { id: `c-${Date.now()}-a`, role: "agent", text }]);
    // Async answer lands out of the focus stream; announce the settled
    // first sentence (4.1.3) instead of every streaming tick.
    announce(text.split("\n")[0] ?? "", "polite");
    bumpLog();
  };

  // The general_chat sticky model pick (this lane): loads once per panel
  // open, renders its picker in the header, and rides every turn.
  const pick = useLazySurfacePick("general_chat");

  async function submitLiveTurn(text: string, history: ChatLine[]): Promise<boolean> {
    const result = await runChatTurn(
      // The session id is client-borne and must be a bare UUID — one
      // continuous agent session per ticket (the transcript rides
      // `chat`, the runner persists none of it).
      ticket.id,
      {
        surface_key: "general_chat",
        provider: pick.provider ?? undefined,
        model: pick.model ?? undefined,
        session_opener: history.length === 0,
        messages: [
          {
            role: "system",
            content:
              `You assist a service desk technician on ticket ${ticket.number} — ${ticket.title} (category ${ticket.category}).`,
          },
          ...history.map((m) => ({
            role: m.role === "you" ? "user" : "assistant",
            content: m.text,
          })),
          { role: "user", content: text },
        ],
      },
    );
    if (result.refusal !== undefined) {
      respond(result.refusalMessage ?? "Not available yet.");
      return false;
    }
    if (result.text !== "") respond(result.text);
    for (const frame of result.frames) {
      if (frame.event === "tool_invocation") {
        respond(`Ran: ${String(frame.data.capability ?? "tool")} (${String(frame.data.outcome ?? "done")}).`);
      }
    }
    return result.frames.some((f) => f.event === "turn_completed");
  }

  async function submitChat() {
    const text = chatDraft.trim();
    if (text === "" || busy) return;
    setBusy(true);
    setChatDraft("");
    pushYou(text);
    try {
      announce("Assistant is thinking");
      const live = await submitLiveTurn(text, chat.slice(0, -1));
      if (live) return;
      const lower = text.toLowerCase();
      if (lower.includes("summar") || lower.includes("summary")) {
        respond(summarise());
      } else if (lower.includes("likely") || lower.includes("cause")) {
        // Likely-cause answer is derived from REAL agent + KB surfaces;
        // when neither files anything, answer with the typed gap.
        if (pending?.rationale != null) {
          respond(`Most likely cause (agent triage, skill ${pending.skill_slug ?? "triage"}):\n${pending.rationale}`);
        } else if (articles !== null && articles[0] !== undefined) {
          respond(`No agent triage is filed yet, but the top knowledge source points here:\n“${articles[0].title}”`);
        } else {
          respond("Not available yet — no agent triage proposal or matching KB article exists for this ticket. The panel lights up when either lands.");
        }
      } else if (lower.includes("fix") || lower.includes("resolve") || lower.includes("how do i")) {
        if (articles !== null && articles[0] !== undefined) {
          respond(`Start from the published runbook:\n“${articles[0].title}” — open it via View evidence or the knowledge sources above.`);
        } else {
          respond("Not available yet — no matching runbook in the KB or filed agent proposal for this ticket.");
        }
      } else if (lower.includes("related") || lower.includes("similar")) {
        if (hits !== null && hits.length > 0) {
          respond(`Related tickets from search:\n${hits.map((h) => `• ${h.title} (${h.snippet.slice(0, 80)}…)`).join("\n")}`);
        } else if (hits !== null) {
          respond("No related tickets came back from search for this ticket's words.");
        } else {
          respond("Search evidence has not loaded yet — reopen the panel or try again.");
        }
      } else {
        respond(
          "Agent chat is not available yet — no conversational agent endpoint is wired. Summarise and Find related tickets work today; drafting rides “Draft customer update”.",
        );
      }
    } finally {
      setBusy(false);
    }
  }

  function draftUpdate() {
    const latest = comments[0];
    const body = [
      "Hi,",
      "",
      `Update on ${ticket.number} (${ticket.title}):`,
      pending?.rationale != null
        ? `Next step under review: ${pending.rationale}`
        : latest !== undefined
          ? `Latest work recorded: ${latest.body.slice(0, 200)}`
          : "We are investigating and will post a detailed update shortly.",
      "",
      "We'll keep you posted.",
    ].join("\n");
    onDraftUpdate(body);
    respond("✓ Draft ready — inserted into the ticket composer.");
  }

    async function decide(decision: "approve" | "decline"): Promise<void> {
      if (pending === null || deciding) return;
      setDeciding(true);
      setActionError(null);
      const previous = proposalStates;
      // Optimistic, because a decision that waits for a round trip to feel
      // acknowledged is a decision people avoid making. A declined proposal
      // writes nothing server-side, so there is nothing to reconcile.
      setProposalStates((prev) => ({ ...prev, [pending.id]: decision === "approve" ? "ran" : "dismissed" }));
      const { error } =
        decision === "approve"
          ? await api.api.POST("/api/v1/agent-proposals/{id}/approve", {
              params: { path: { id: pending.id } },
              body: { note: null },
            })
          : await api.api.POST("/api/v1/agent-proposals/{id}/decline", {
              params: { path: { id: pending.id } },
              body: { note: null },
            });
      setDeciding(false);
      if (error !== undefined) {
        setProposalStates(previous);
        setActionError(decision === "approve" ? "The run was rejected." : "The dismissal was rejected.");
        return;
      }
      if (decision === "approve") {
        // The spec's System entry. The proposal's own write is the record; this
        // line is the sentence in the timeline that says a person ran it.
        respond(`Ran the agent's recommendation: ${pending.rationale ?? "the proposed next step"}.`);
      }
    }

    // Undo of a dismissal: a decline cannot be un-declined, so Undo brings the
    // recommendation back to be approved rather than pretending it was never
    // touched. The audit row stays — it is the server's, not the panel's.
    function undoDismissal(): void {
      if (pending === null) return;
      setProposalStates((prev) => {
        const next = { ...prev };
        delete next[pending.id];
        return next;
      });
      setActionError(null);
    }

    // The recommended next step. Three states, all of them from the spec: the
    // card with its primary action, the card after the action RAN, and the
    // dashed row it collapses to when the recommendation is not this one.
    const pendingState = pending === null ? undefined : proposalStates[pending.id];
    const recommendBlock =
      pending?.rationale != null && pendingState === "dismissed" ? (
        <div className="mt-3">
          <AssistantCardDismissed
            data-testid="wb-agent-dismissed"
            summary={`Dismissed: ${pending.rationale}`}
            onUndo={undoDismissal}
          />
        </div>
      ) : (
        <div className="mt-3">
          <AssistantCard
            eyebrow="Recommended next step"
            data-testid="wb-agent-recommend"
            heading={pending?.rationale ?? "No agent proposals filed for this ticket yet."}
            action={
              pendingState === "ran" ? (
                // "Script ran" is a finished state, not a disabled button: it
                // reports what happened and stays visible so the person can
                // see their own work in the timeline.
                <span className="m-assistant-ran" data-testid="wb-agent-ran">
                  <Icon name="check" size={12} />
                  Script ran
                </span>
              ) : (
                <Button
                  variant="assistant"
                  size="sm"
                  disabled={deciding || pending === null}
                  data-testid="wb-agent-run"
                  onClick={() => void decide("approve")}
                >
                  {deciding ? "Working…" : "Run"}
                </Button>
              )
            }
            secondaryActions={
              <Button
                // Outlined, not filled: the vision review caught Run and Not
                // this rendering as two identical gold pills, which leaves the
                // primary no more likely than the dismissal. Gold means "act
                // here", so a card where everything is gold says nothing.
                variant="assistant-outline"
                size="sm"
                disabled={deciding || pending === null || pendingState === "ran"}
                data-testid="wb-agent-not-this"
                onClick={() => void decide("decline")}
              >
                Not this
              </Button>
            }
            footer={
              <div className="flex flex-wrap items-center gap-2">
                {articles !== null && articles[0] !== undefined ? (
                  <Link
                    href={`/kb/${articles[0].id}`}
                    className="m-assistant-link"
                    data-testid="wb-agent-evidence"
                  >
                    View evidence ↗
                  </Link>
                ) : null}
                <button
                  type="button"
                  className="m-assistant-link"
                  data-testid="wb-agent-draft"
                  onClick={draftUpdate}
                >
                  Draft customer update
                </button>
                <button
                  type="button"
                  className="m-assistant-link"
                  onClick={() => chatInputRef.current?.focus()}
                >
                  Ask Agent
                </button>
              </div>
            }
          >
            <AssistantCardQuestion
              question="Do you want me to do anything client-facing?"
              answer={
                pendingState === "ran"
                  ? "No — drafts wait for your approval, and nothing is sent without it."
                  : undefined
              }
            />
          </AssistantCard>
          {actionError !== null ? (
            <p className="m-assistant-error" role="alert" data-testid="wb-agent-action-error">
              {actionError}
            </p>
          ) : null}
        </div>
      );

  return (
    <aside
      aria-label="Agent panel"
      data-testid="wb-agent"
      data-open={open}
      className="m-wb-agent flex min-h-0 shrink-0 flex-col overflow-hidden border-l"
    >
      <div className="flex items-center gap-2 border-b px-4" style={{ borderColor: "var(--m-border)", height: 49 }}>
        <span aria-hidden style={{ display: "inline-flex", color: "var(--m-accent)" }}>
          <Icon name="spark" size={15} />
        </span>
        <span className="text-[14px] font-bold" style={{ color: "var(--m-text)" }}>Agent</span>
        <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>context-aware</span>
        <button
          type="button"
          data-testid="wb-agent-close"
          aria-label="Hide agent panel"
          className="ml-auto cursor-pointer border-none bg-transparent text-[13px]"
          style={{ color: "var(--m-text-muted)" }}
          onClick={onClose}
        >
          ×
        </button>
      </div>

      <div className="m-wb-agent-scroll min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        <p className="border-b pb-3 text-[11px] leading-relaxed" style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}>
          Based on this ticket, customer history and documentation…
        </p>

        <div className="mt-3 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
          Relevant knowledge sources
        </div>
        {articles === null ? (
          <p className="py-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>Searching the knowledge base…</p>
        ) : articles.length > 0 ? (
          articles.map((a) => (
            <Link key={a.id} href={`/kb/${a.id}`} className="m-wb-linkrow flex items-start gap-2 py-1.5">
              <span className="m-wb-source-icon grid h-6 w-6 shrink-0 place-items-center rounded-md border" style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text-muted)" }}>
                <svg aria-hidden width="11" height="11" viewBox="0 0 18 18" fill="none">
                  <path d="M3 4.5c2-1 4-1 6 0v9c-2-1-4-1-6 0v-9Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
                  <path d="M15 4.5c-2-1-4-1-6 0v9c2-1 4-1 6 0v-9Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
                </svg>
              </span>
              <span className="min-w-0">
                <strong className="block truncate text-[11px]" style={{ color: "var(--m-text)" }}>{a.title}</strong>
                <span className="block text-[9.5px]" style={{ color: "var(--m-text-muted)" }}>kb · {a.slug}</span>
              </span>
            </Link>
          ))
        ) : (
          <p className="py-1 text-[11px]" data-testid="wb-agent-no-kb" style={{ color: "var(--m-text-muted)" }}>
            No published KB articles match this ticket&apos;s words yet.
          </p>
        )}

        {/* Evidence: related tickets through the unified search */}
        {hits !== null && hits.length > 0 && (
          <div
            className="mt-2 overflow-hidden rounded-lg border"
            style={{ borderColor: "var(--m-status-success-fg)", background: "var(--m-status-success-bg)" }}
          >
            <div className="flex items-center gap-2 p-2.5 text-[11px] font-bold" style={{ color: "var(--m-status-success-fg)" }}>
              <span aria-hidden>✓</span>
              <span>Related tickets found</span>
            </div>
            <div className="px-2.5 pb-2.5">
              {hits.map((h) => (
                <p key={h.entity_id} className="py-0.5 text-[10.5px]" style={{ color: "var(--m-text-secondary)" }}>
                  • {h.title}
                </p>
              ))}
            </div>
          </div>
        )}
        {hits !== null && hits.length === 0 && (
          <p className="mt-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            No similar tickets in search — this looks like a first occurrence.
          </p>
        )}
        {hits === null && <p className="mt-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>Loading search evidence…</p>}

        {/* Agent decisions on this ticket (real ledger) */}
        {decided.length > 0 && (
          <>
            <div className="mt-3 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
              Agent activity on this ticket
            </div>
            {decided.map((p) => (
              <div key={p.id} className="mt-1.5 rounded-lg border p-2.5 text-[10.5px]" style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text-secondary)" }}>
                <strong className="text-[11px]" style={{ color: "var(--m-text)" }}>{p.skill_slug ?? p.agent_kind}</strong>{" "}
                · {p.status}
                {p.rationale != null && <div className="mt-0.5">{p.rationale}</div>}
              </div>
            ))}
          </>
        )}


        {recommendBlock}

        {/* Chat */}
        <div className="mt-4 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
          <div className="mb-1.5 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
            Chat with agent
          </div>
          {chat.length > 0 && (
            <div className="mb-2 grid gap-1.5" data-testid="wb-agent-chat-log" ref={chatLogRef}>
              {chat.map((line) => (
                <div
                  key={line.id}
                  className="whitespace-pre-wrap rounded-lg border p-2 text-[10.5px]"
                  style={{
                    borderColor: line.role === "you" ? "var(--m-border)" : "color-mix(in srgb, var(--m-chart-1) 35%, var(--m-border))",
                    background:
                      line.role === "you"
                        ? "var(--m-surface)"
                        : "color-mix(in srgb, var(--m-chart-1) 12%, var(--m-surface))",
                    color: "var(--m-text-secondary)",
                  }}
                >
                  {line.text}
                </div>
              ))}
            </div>
          )}
          <div className="rounded-lg border p-2" style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}>
            <textarea
              ref={chatInputRef}
              data-testid="wb-agent-chat-input"
              value={chatDraft}
              onChange={(e) => setChatDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void submitChat();
                }
              }}
              placeholder="Ask a question about this ticket…"
              aria-label="Ask the agent"
              className="min-h-[33px] w-full resize-none border-0 bg-transparent text-[11px] outline-none"
              style={{ color: "var(--m-text)" }}
            />
            <div className="flex items-center justify-between gap-2">
              <ModelPicker pickState={pick} compact />
              <button
                type="button"
                aria-label="Send to agent"
                className="h-6 w-6 cursor-pointer rounded-full border text-[10px]"
                style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)", color: "var(--m-text-secondary)" }}
                onClick={() => void submitChat()}
              >
                ➤
              </button>
            </div>
          </div>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {([
              ["What's the likely cause?", "What's the likely cause?"],
              ["How do I fix this?", "How do I fix this?"],
              ["Find related tickets", "Find related tickets"],
              ["Summarise this ticket", "Summarise this ticket"],
            ] as const).map(([label, fill]) => (
              <button
                key={label}
                type="button"
                className="m-wb-chip cursor-pointer text-[9.5px]"
                onClick={() => {
                  setChatDraft(fill);
                  void submitChat();
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </aside>
  );
}
