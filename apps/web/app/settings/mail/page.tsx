// /settings/mail — permanent redirect to /settings/email.
//
// The mail administration surface shipped as /settings/email (Wave 34 lane
// 8); this route stays as a redirect so old deep links keep working.
"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { Spinner } from "@marula-core/ui";

export default function MailSettingsRedirect() {
  const router = useRouter();
  useEffect(() => {
    router.replace("/settings/email");
  }, [router]);
  return <Spinner />;
}
