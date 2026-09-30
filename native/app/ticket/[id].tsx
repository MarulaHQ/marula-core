// Ticket route — the pack's read model on a live ticket.
//
// Purpose: a thin route shell. The screen itself lives in
// `src/tickets/pack-detail.tsx` so the Board tab's detail route and this
// deep-link route render ONE read model (the pack draws both: the board's
// detail adds the card's fields above the conversation).

import { useLocalSearchParams } from "expo-router";

import { PackTicketDetail } from "@/tickets/pack-detail";
import { RequireAuth } from "@/ui/require-auth";

export default function TicketRoute() {
  const { id, from } = useLocalSearchParams<{ id: string; from?: string }>();
  return (
    <RequireAuth>
      {id !== undefined ? (
        // `?from=notification` is the deep link a notification row pushes:
        // the screen shows the green "From a notification" banner and its
        // Back returns to the list rather than the Board.
        <PackTicketDetail id={id} fromNotification={from === "notification"} />
      ) : null}
    </RequireAuth>
  );
}
