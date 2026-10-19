// A minimal Server-Sent Events transport for React Native.
//
// React Native ships no global `EventSource`, and this app carries no SSE
// dependency, so without a transport the live-sync client would open nothing
// on a device and sit in backoff forever. This module supplies the
// `WireEventSource` contract over `XMLHttpRequest`, which React Native does
// implement with incremental bodies: `readyState` 3 exposes `responseText` as
// it arrives, which is what an SSE reader needs. On the web build the global
// `EventSource` exists, so it is used directly and this class is never built.
//
// Scope: the framing the stream sends — `data:` and `id:` fields with
// blank-line dispatch, multi-line data joined by newlines, `:` comment lines
// ignored. It is not a general-purpose EventSource: the only dispatched event
// is the default `message`, and reconnect policy lives in the caller
// (realtime.ts). The one exception is the difference between a transient drop
// and a refusal, which the wire layer is the only place that can tell apart:
// a stream that ends reopens here, while a non-2xx response closes with
// `readyState` 2 so the client reconnects with a fresh token.

import type { WireEventSource } from "./realtime";

const CONNECTING = 0;
const OPEN = 1;
const CLOSED = 2;

/** How long to wait before reopening after a transient drop. The server may
 * close an idle stream, and this keeps that reconnect inside the source. */
const TRANSIENT_RETRY_MS = 1_000;

type Handler = (event?: unknown) => void;

/** A frame as the client's listeners see it: `data` is the frame JSON, and
 * `lastEventId` is the SSE `id:` field the client resends on reconnect. */
interface SseMessageEvent {
  data: string;
  lastEventId: string;
}

class XhrEventSource implements WireEventSource {
  private xhr: XMLHttpRequest | null = null;
  private readonly listeners = new Map<string, Set<Handler>>();
  private buffer = "";
  private consumed = 0;
  private lastEventId = "";
  private state = CLOSED;
  private closedByUser = false;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly url: string) {
    this.open();
  }

  get readyState(): number {
    return this.state;
  }

  addEventListener(type: "open" | "error" | "message", fn: Handler): void {
    let set = this.listeners.get(type);
    if (set === undefined) {
      set = new Set<Handler>();
      this.listeners.set(type, set);
    }
    set.add(fn);
  }

  removeEventListener(type: "open" | "error" | "message", fn: Handler): void {
    this.listeners.get(type)?.delete(fn);
  }

  close(): void {
    this.closedByUser = true;
    this.cancelRetry();
    this.teardown();
    this.state = CLOSED;
  }

  private open(): void {
    const xhr = new XMLHttpRequest();
    this.xhr = xhr;
    this.state = CONNECTING;
    this.buffer = "";
    this.consumed = 0;
    xhr.open("GET", this.url, true);
    xhr.setRequestHeader("Accept", "text/event-stream");
    xhr.setRequestHeader("Cache-Control", "no-cache");
    if (this.lastEventId !== "") {
      xhr.setRequestHeader("Last-Event-ID", this.lastEventId);
    }
    xhr.onreadystatechange = () => this.onReadyStateChange(xhr);
    xhr.onerror = () => this.onTransientFailure(xhr);
    xhr.ontimeout = () => this.onTransientFailure(xhr);
    xhr.send();
  }

  private onReadyStateChange(xhr: XMLHttpRequest): void {
    if (this.xhr !== xhr) return;
    if (xhr.readyState === 2) {
      // Headers are in. A non-2xx is a refusal the client must reconnect on
      // with a fresh token, so it is reported as CLOSED.
      if (xhr.status >= 200 && xhr.status < 300) {
        this.state = OPEN;
        this.emit("open");
      } else {
        this.state = CLOSED;
        this.teardown();
        this.emit("error");
      }
      return;
    }
    if (xhr.readyState === 3) {
      if (this.state === OPEN) this.consume(xhr.responseText);
      return;
    }
    if (xhr.readyState === 4) {
      // The stream ended. Unless the user closed it, reopen in place so the
      // client sees a status blip rather than a refusal.
      this.onTransientFailure(xhr);
    }
  }

  private onTransientFailure(xhr: XMLHttpRequest): void {
    if (this.xhr !== xhr || this.closedByUser || this.state === CLOSED) return;
    this.teardown();
    this.state = CONNECTING;
    this.emit("error");
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (!this.closedByUser) this.open();
    }, TRANSIENT_RETRY_MS);
  }

  private consume(text: string): void {
    if (text.length <= this.consumed) return;
    this.buffer += text.slice(this.consumed);
    this.consumed = text.length;
    const boundary = /\r?\n\r?\n/;
    let match = boundary.exec(this.buffer);
    while (match !== null) {
      this.dispatch(this.buffer.slice(0, match.index));
      this.buffer = this.buffer.slice(match.index + match[0].length);
      match = boundary.exec(this.buffer);
    }
  }

  private dispatch(block: string): void {
    let data = "";
    for (const line of block.split(/\r?\n/)) {
      if (line === "" || line.startsWith(":")) continue;
      const colon = line.indexOf(":");
      const field = colon === -1 ? line : line.slice(0, colon);
      let value = colon === -1 ? "" : line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      if (field === "data") {
        data = data === "" ? value : `${data}\n${value}`;
      } else if (field === "id") {
        // A bare `id:` resets the cursor; an `id:` carrying a null byte is
        // ignored, per the EventSource parsing rules.
        if (!value.includes("\u0000")) this.lastEventId = value;
      }
    }
    if (data === "") return;
    const message: SseMessageEvent = { data, lastEventId: this.lastEventId };
    this.emit("message", message);
  }

  private teardown(): void {
    const xhr = this.xhr;
    if (xhr === null) return;
    this.xhr = null;
    xhr.onreadystatechange = null;
    xhr.onerror = null;
    xhr.ontimeout = null;
    try {
      xhr.abort();
    } catch {
      // The request already finished; nothing to abort.
    }
  }

  private cancelRetry(): void {
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }

  private emit(type: "open" | "error" | "message", event?: unknown): void {
    const set = this.listeners.get(type);
    if (set === undefined) return;
    for (const fn of [...set]) fn(event);
  }
}

/** Opens the live stream. The web build has a global `EventSource`; a device
 * build does not, and gets the `XMLHttpRequest` implementation above. */
export function createSseSource(url: string): WireEventSource {
  const GlobalEventSource = (
    globalThis as { EventSource?: new (target: string) => unknown }
  ).EventSource;
  if (typeof GlobalEventSource === "function") {
    return new GlobalEventSource(url) as unknown as WireEventSource;
  }
  return new XhrEventSource(url);
}
