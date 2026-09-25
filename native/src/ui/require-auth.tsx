// Auth guard for routes outside the tab shell (deep links, push routes).

import { Redirect } from "expo-router";
import type { ReactNode } from "react";

import { useAuth } from "@/auth/auth-context";
import { BootScreen } from "@/ui/boot";

export function RequireAuth({ children }: { children: ReactNode }) {
  const { status } = useAuth();
  if (status === "loading") return <BootScreen />;
  if (status === "locked") return <Redirect href="/locked" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "signedOut") return <Redirect href="/login" />;
  return <>{children}</>;
}
