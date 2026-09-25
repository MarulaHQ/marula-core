// A ticking clock for render-safe relative times.
//
// Purpose: SLA pills and due banners compare against "now". Reading
// Date.now() during render is impure (React Compiler flags it, and the value
// never updates on its own). This hook owns the clock in state and re-ticks
// on an interval, so a pill recomputes when time passes.
//
// Non-purpose: no timers for the server-backed timer session (that lives in
// timer-bar.tsx against the server's segments).

import { useEffect, useState } from "react";

export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(interval);
  }, [intervalMs]);
  return now;
}
