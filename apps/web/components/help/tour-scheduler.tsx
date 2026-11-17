// TourScheduler — first-visit-per-surface auto-start (Wave 26 lane 2).
//
// Purpose: decides WHEN a tour may start, never what it says (data lives
// in lib/tours/tours.ts). Contract:
//  - only for authed staff-shell surfaces (headerless paths own their chrome);
//  - the account's tours.disabled switch and the org's tours.org_disabled
//    kill switch both suppress (help-panel guides always stay available);
//  - a tour runs once per account (tours.completed, server-persisted with
//    an offline localStorage fallback);
//  - capped per session (SESSION_TOUR_CAP) so surface-hopping never nags;
//  - every step target must exist on screen or the tour silently skips;
//  - explicit replay ("Take the tour" in the help panel, via the tour
//    request bus) bypasses the switches and the session cap — both
//    govern AUTO-starts only, never a deliberate ask;
//  - "Skip tour" (overlay, onSkipTour) reuses the Done write path: mark
//    THIS tour's id complete and close, leaving the global switch alone.

"use client";

import { usePathname } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "../../lib/api";
import { isAdminViewer, routeKeyForPath } from "../../lib/help/manifest";
import {
  bumpSessionTourCount,
  sessionTourCount,
  tourForRouteKey,
  tourTargetsReady,
  type Tour,
} from "../../lib/tours/tours";
import { subscribeTourRequests } from "../../lib/tours/tour-bus";
import { useToursState } from "../../lib/help/help-settings";
import { TourOverlay } from "./tour-overlay";

const START_DELAY_MS = 900;

export function TourScheduler() {
  const pathname = usePathname();
  const { me, status } = useAuth();
  const tours = useToursState(status === "authed");
  const [active, setActive] = useState<Tour | null>(null);
  const [stepIndex, setStepIndex] = useState(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const isAdmin = isAdminViewer(me?.permissions ?? []);

  useEffect(() => {
    if (timer.current !== null) clearTimeout(timer.current);
    if (!tours.loaded || tours.disabled || tours.orgDisabled || active) return;
    const routeKey = routeKeyForPath(pathname);
    const tour = tourForRouteKey(routeKey, tours.completed, isAdmin);
    if (!tour) return;
    if (sessionTourCount() >= 1) return;
    timer.current = setTimeout(() => {
      if (!tourTargetsReady(tour)) return;
      bumpSessionTourCount();
      setActive(tour);
      setStepIndex(0);
    }, START_DELAY_MS);
    return () => {
      if (timer.current !== null) clearTimeout(timer.current);
    };
  }, [pathname, tours.loaded, tours.disabled, tours.orgDisabled, tours.completed, isAdmin, active]);

  // Replay ("Take the tour" in the help panel): explicit intent, so this
  // path skips the disabled/org switches and the session cap (both
  // govern auto-starts only) and passes an EMPTY completed list, so a
  // tour already seen still replays. No bump: replay never consumes the
  // session cap.
  useEffect(
    () =>
      subscribeTourRequests((routeKey) => {
        if (active) return;
        const tour = tourForRouteKey(routeKey, [], isAdmin);
        if (!tour || !tourTargetsReady(tour)) return;
        setActive(tour);
        setStepIndex(0);
      }),
    [active, isAdmin],
  );

  const finish = useCallback(() => {
    if (active) void tours.markCompleted(active.id);
    setActive(null);
    setStepIndex(0);
  }, [active, tours]);

  const disableTours = useCallback(() => {
    void tours.setDisabled(true);
    finish();
  }, [tours, finish]);

  if (!active) return null;

  return (
    <TourOverlay
      tour={active}
      stepIndex={stepIndex}
      onNext={() => setStepIndex((i) => Math.min(i + 1, active.steps.length - 1))}
      onBack={() => setStepIndex((i) => Math.max(i - 1, 0))}
      onDone={finish}
      // "Skip tour" is this-tour-only: same write path as Done (mark the
      // id complete, close) — the global switch is onDisableTours, not this.
      onSkipTour={finish}
      onDisableTours={disableTours}
    />
  );
}
