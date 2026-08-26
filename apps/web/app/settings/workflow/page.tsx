// /settings/workflow — placeholder pending the workflow-customization wave.
//
// Purpose: honest disabled state for a nav section whose editor (custom
// statuses/transitions) ships with another lane.
// Non-purpose: no workflow logic.

import { SettingsPendingPanel } from "../../../components/settings-pending-panel";

export default function WorkflowSettingsPage() {
  return (
    <SettingsPendingPanel
      title="Workflow"
      owner="workflow customization wave"
    />
  );
}
