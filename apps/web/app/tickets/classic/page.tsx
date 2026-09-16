// /tickets/classic — the pre-workbench-v2 three-pane workspace, preserved
// for old-vs-new comparison during the Wave 15 lane 7 transition (the new
// workbench lives at /tickets). Removal is a follow-up after the vision
// pass signs off — never this lane's.
import { TicketWorkspaceClassic } from "../../../components/ticket-workspace/classic-workspace";

export default function TicketClassicPage() {
  return <TicketWorkspaceClassic />;
}
