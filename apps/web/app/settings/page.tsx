// /settings — index redirect to the first IA section.
//
// Purpose: keep /settings a valid, predictable entry point.
// Non-purpose: no content of its own.

import { redirect } from "next/navigation";

export default function SettingsIndexPage() {
  redirect("/settings/general");
}
