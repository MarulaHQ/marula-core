"use client";
// LiveRegion — the ONE global announcer for async state (p29-accessibility-
// deep, WCAG 4.1.3). AI streaming ticks, save-bar confirmations, decision
// suggestions, and undo toasts announce through `announce()`; screen readers
// hear one settled sentence instead of every intermediate render.
//
// Mount ONCE in AppShell. Call sites import { announce } — never roll a
// local aria-live div (competing regions read over each other).

import { useEffect, useRef, useState } from "react";

type Politeness = "polite" | "assertive";

interface PendingMessage {
  text: string;
  politeness: Politeness;
}

let queue: PendingMessage[] = [];
let flushListeners: ((messages: PendingMessage[]) => void)[] = [];

/** Announce a settled message. Pass "assertive" only for errors the user must act on. */
export function announce(text: string, politeness: Politeness = "polite"): void {
  if (!text || typeof window === "undefined") return;
  queue = [...queue, { text, politeness }];
  for (const listener of flushListeners) listener(queue);
}

export function LiveRegion(): React.JSX.Element {
  const [polite, setPolite] = useState("");
  const [assertive, setAssertive] = useState("");
  const timer = useRef<number | null>(null);

  useEffect(() => {
    const listener = (messages: PendingMessage[]): void => {
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        const politeText = messages
          .filter((m) => m.politeness === "polite")
          .map((m) => m.text)
          .join(". ");
        const assertiveText = messages
          .filter((m) => m.politeness === "assertive")
          .map((m) => m.text)
          .join(". ");
        if (politeText) setPolite(politeText);
        if (assertiveText) setAssertive(assertiveText);
        queue = [];
      }, 150);
    };
    flushListeners.push(listener);
    return () => {
      flushListeners = flushListeners.filter((l) => l !== listener);
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, []);

  return (
    <div className="sr-only" data-testid="a11y-live-region">
      <div role="status" aria-live="polite">
        {polite}
      </div>
      <div role="alert" aria-live="assertive">
        {assertive}
      </div>
    </div>
  );
}
