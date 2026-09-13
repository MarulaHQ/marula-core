// Test setup for @meridian/web: DOM matchers plus the same jsdom shims
// Radix primitives rely on as packages/ui (ResizeObserver, scrollIntoView,
// pointer capture, matchMedia).
import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { routedFetch } from "./helpers/fetch-router";

// Auto-unmount between tests. Manual end-of-test cleanup() calls never run
// when an assertion throws mid-test, so one failing test leaked its DOM into
// the next render (duplicate buttons/labels — the workflow-page flake in
// full suite runs). Double cleanup is a no-op.
afterEach(() => cleanup());

// Install the routed fetch BEFORE any app module imports @meridian/sdk —
// openapi-fetch captures the default fetch at client construction.
vi.stubGlobal("fetch", routedFetch);

class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
}

if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.releasePointerCapture = () => {};
}

// Never-connecting EventSource shim: the instant layer's SSE live leg
// (lib/instant/stream.ts). Page tests drive frames through the runtime's
// `stream.deliver` test seam instead of a live wire; the shim exists so
// `new EventSource(...)` never throws a ReferenceError under jsdom and
// subscribers settle into the quiet "connecting" state.
class FakeEventSource {
  static instances: FakeEventSource[] = [];
  readyState = 0;
  url: string;
  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }
  close(): void {}
  addEventListener(): void {}
  removeEventListener(): void {}
}

if (typeof globalThis.EventSource === "undefined") {
  globalThis.EventSource = FakeEventSource as unknown as typeof EventSource;
}

if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
