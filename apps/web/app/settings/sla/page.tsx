// /settings/sla — placeholder pending the SLA-engine wave.
//
// Purpose: honest disabled state for a nav section whose editor (SLA
// policies, business-hours calendars) ships with another lane.
// Non-purpose: no SLA logic.

import { SettingsPendingPanel } from "../../../components/settings-pending-panel";

export default function SlaSettingsPage() {
  return <SettingsPendingPanel title="SLA" owner="SLA engine wave" />;
}
