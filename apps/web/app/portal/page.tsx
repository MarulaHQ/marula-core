// /portal — entry point; the portal's only destination is the ticket list.
"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { usePortalAuth } from "../../lib/portal-api";

export default function PortalIndexPage() {
  const { status } = usePortalAuth();
  const router = useRouter();
  useEffect(() => {
    if (status === "authed") router.replace("/portal/tickets");
    if (status === "anonymous") router.replace("/portal/login");
  }, [status, router]);
  return null;
}
