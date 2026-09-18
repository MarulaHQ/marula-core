// First-run setup banner for the Today surface.
//
// Purpose: if the org hasn't completed (or dismissed) the Tier-1 wizard,
// offer a quiet entry point. Dismissal IS completion — the same
// onboarding.wizard_completed registry setting persists it server-side,
// so the banner never nags again on any device.
//
// Non-purpose: no wizard logic here; this is only the entry affordance.
"use client";

import { Button, Card } from "@meridian/ui";
import Link from "next/link";
import { useEffect, useState } from "react";
import { api, useAuth } from "../lib/api";

export function SetupBanner() {
  const auth = useAuth();
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/org");
      if (cancelled || !data) return;
      const done = (data ?? []).some(
        (v) => v.key === "onboarding.wizard_completed" && v.value === true,
      );
      setVisible(!done);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  if (!visible) return null;

  const dismiss = async () => {
    setVisible(false);
    // Fire-and-forget: dismissal persists via the registry (idempotent).
    void api.api.PUT("/api/v1/settings/org", {
      body: { key: "onboarding.wizard_completed", value: true },
    });
  };

  return (
    <Card className="p-4 flex flex-wrap items-center justify-between gap-3">
      <div className="grid gap-0.5">
        <p
          className="text-[13px] font-medium"
          style={{ color: "var(--m-text)" }}
        >
          Finish setting up your organization
        </p>
        <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Two minutes: name and ticket prefix. Skipping is safe — defaults are
          already running.
        </p>
      </div>
      <div className="flex items-center gap-2">
        <Link
          href="/welcome"
          className="rounded-[var(--m-radius-container)] px-3 py-1.5 text-[13px] font-medium"
          style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
        >
          Set up now
        </Link>
        <Button variant="ghost" size="sm" onClick={() => void dismiss()}>
          Dismiss
        </Button>
      </div>
    </Card>
  );
}
