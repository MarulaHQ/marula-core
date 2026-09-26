import { MovementScreen } from "@/fieldops/movement-screen";
import { RequireAuth } from "@/ui/require-auth";

export default function MoveRoute() {
  return (
    <RequireAuth>
      <MovementScreen />
    </RequireAuth>
  );
}
