import { StockLookupScreen } from "@/fieldops/stock-lookup-screen";
import { RequireAuth } from "@/ui/require-auth";

export default function StockRoute() {
  return (
    <RequireAuth>
      <StockLookupScreen />
    </RequireAuth>
  );
}
