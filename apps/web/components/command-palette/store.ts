// Command palette and chat panel store.
//
// Purpose: ONE state container shared by the palette overlay (K01, K02) and
// the slide-up chat panel (K03-K05), so "Open as a panel" moves the same
// conversation at the same message state instead of starting a second chat.
//
// Non-purpose: no network, no React. The turn itself lives in chat-turn.ts;
// the surfaces only dispatch here. Messages are client-owned — the runner
// persists no transcript — which is why the session id is minted here.

"use client";

export type PaletteMode = "search" | "ask";

/** Panel states. `entering` is K04, the mid-expansion frame: the lower edge
 *  is clipped behind the timer bar while it translates in. */
export type PanelPhase = "closed" | "collapsed" | "entering" | "open";

export type ChatSource = {
  /** What was used, in the user's words. */
  label: string;
  /** Where it came from, when the frame carries it. */
  detail?: string | null;
};

export type ChatProposal = {
  proposalId?: string;
  ticketId?: string;
  title: string;
  detail: string;
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  text: string;
  sources: ChatSource[];
  proposals: ChatProposal[];
  /** The turn is still streaming. */
  pending: boolean;
  /** A typed refusal or transport failure, shown in the danger pair. */
  error: string | null;
};

export type PaletteState = {
  open: boolean;
  mode: PaletteMode;
  panel: PanelPhase;
  query: string;
  messages: ChatMessage[];
  /** A turn is in flight; a second send would race the first. */
  busy: boolean;
};

const EMPTY: PaletteState = {
  open: false,
  mode: "search",
  panel: "closed",
  query: "",
  messages: [],
  busy: false,
};

let state: PaletteState = EMPTY;
const listeners = new Set<() => void>();
let sessionId: string | null = null;
let messageSeq = 0;

function emit(): void {
  for (const listener of listeners) listener();
}

function set(patch: Partial<PaletteState>): void {
  state = { ...state, ...patch };
  emit();
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getSnapshot(): PaletteState {
  return state;
}

/** Server render has no overlay open; a stable object keeps hydration quiet. */
export function getServerSnapshot(): PaletteState {
  return EMPTY;
}

/** A bare UUID the ChatRunner accepts (it is client-borne, one per session). */
export function chatSessionId(): string {
  if (sessionId !== null) return sessionId;
  const minted = globalThis.crypto?.randomUUID?.();
  sessionId = minted ?? fallbackUuid();
  return sessionId;
}

/** Only for environments without crypto.randomUUID (older jsdom). */
function fallbackUuid(): string {
  const hex = (count: number) =>
    Array.from({ length: count }, () => Math.floor(Math.random() * 16).toString(16)).join("");
  return `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`;
}

export const actions = {
  open(mode: PaletteMode = "search"): void {
    set({ open: true, mode });
  },
  close(): void {
    set({ open: false });
  },
  setMode(mode: PaletteMode): void {
    set({ mode });
  },
  setQuery(query: string): void {
    set({ query });
  },
  /** `entering` is set by the panel and settles to `open` after the motion. */
  setPanel(panel: PanelPhase): void {
    set({ panel });
  },
  pushUserMessage(text: string): ChatMessage {
    const message: ChatMessage = {
      id: nextMessageId("you"),
      role: "user",
      text,
      sources: [],
      proposals: [],
      pending: false,
      error: null,
    };
    set({ messages: [...state.messages, message] });
    return message;
  },
  startAssistant(): ChatMessage {
    const message: ChatMessage = {
      id: nextMessageId("agent"),
      role: "assistant",
      text: "",
      sources: [],
      proposals: [],
      pending: true,
      error: null,
    };
    set({ messages: [...state.messages, message], busy: true });
    return message;
  },
  appendDelta(messageId: string, delta: string): void {
    patchMessage(messageId, (message) => ({ ...message, text: message.text + delta }));
  },
  addSource(messageId: string, source: ChatSource): void {
    patchMessage(messageId, (message) => ({ ...message, sources: [...message.sources, source] }));
  },
  addProposal(messageId: string, proposal: ChatProposal): void {
    patchMessage(messageId, (message) => ({ ...message, proposals: [...message.proposals, proposal] }));
  },
  finishAssistant(messageId: string, text: string | null): void {
    patchMessage(messageId, (message) => ({
      ...message,
      // `turn_completed.text` wins when the runner sends one; deltas otherwise.
      text: text !== null && text !== "" ? text : message.text,
      pending: false,
    }));
    set({ busy: false });
  },
  failAssistant(messageId: string, error: string): void {
    patchMessage(messageId, (message) => ({
      ...message,
      pending: false,
      error,
      text: message.text,
    }));
    set({ busy: false });
  },
  /** Test seam: clears everything without touching the minted session id. */
  resetForTests(): void {
    state = EMPTY;
    emit();
  },
};

function patchMessage(messageId: string, update: (message: ChatMessage) => ChatMessage): void {
  set({
    messages: state.messages.map((message) => (message.id === messageId ? update(message) : message)),
  });
}

function nextMessageId(prefix: string): string {
  messageSeq += 1;
  return `${prefix}-${messageSeq}`;
}

/** The last assistant line, for the collapsed tab's one-line preview (K05). */
export function lastAssistantPreview(messages: ChatMessage[]): string {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message !== undefined && message.role === "assistant" && message.text !== "") {
      return message.text.replace(/\s+/g, " ").trim();
    }
  }
  return "";
}
