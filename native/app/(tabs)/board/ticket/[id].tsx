// Board ticket route — the same pack read model the deep-link route shows.
//
// Purpose: one ticket detail in the app. The pack draws the board's detail
// (it adds the card's fields above the conversation) and the deep-linked
// open; both are the same read model, so both render `PackTicketDetail`.
// The board stack keeps the route (deep links and the Maestro flows use it)
// but the screen is no longer a second, older layout.

import { useLocalSearchParams } from "expo-router";

import { PackTicketDetail } from "@/tickets/pack-detail";
import { RequireAuth } from "@/ui/require-auth";

export default function BoardTicket() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return (
    <RequireAuth>
      {id !== undefined ? <PackTicketDetail id={id} /> : null}
    </RequireAuth>
  );
}
