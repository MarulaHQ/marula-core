// Event bus — a tiny synchronous pub/sub for parsed instant events.
//
// Purpose: decouple the stream (producer) from the store, the offline
// queue (reconnect triggers) and any UI listeners (consumers). One place
// to subscribe without threading callbacks through component trees.
//
// Non-purpose: no buffering or replay — events are live deltas; a consumer
// that needs missed state rehydrates through the store.

import type { InstantEvent } from "./events";

export type EventListener = (event: InstantEvent) => void;

export class EventBus {
  private readonly listeners = new Set<EventListener>();

  subscribe(fn: EventListener): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  publish(event: InstantEvent): void {
    for (const fn of [...this.listeners]) fn(event);
  }

  get size(): number {
    return this.listeners.size;
  }
}
