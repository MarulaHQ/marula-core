// /settings/mail — placeholder pending full mail administration.
//
// Purpose: honest disabled state; inbound routing already has a working
// admin API but its dedicated manager surface ships with a later wave
// (deep-linkable today via /settings/security-adjacent admin tooling).
// Non-purpose: no mail logic.
//
// Note: org-level inbound routing IS configurable through
// /api/email/inbound-settings; this page defers to that surface rather
// than duplicating it.

import { SettingsPendingPanel } from "../../../components/settings-pending-panel";

export default function MailSettingsPage() {
  return <SettingsPendingPanel title="Mail" owner="mail administration wave" />;
}
