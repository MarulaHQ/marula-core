// 404 — honest state for an unknown route (deep links included).

import { router } from "expo-router";

import { Button, EmptyState, Screen } from "@/ui/primitives";

export default function NotFound() {
  return (
    <Screen testID="not-found-screen">
      <EmptyState
        message="That link doesn't point anywhere in Meridian."
        actionLabel="Go to Today"
        onAction={() => router.replace("/")}
      />
      <Button label="Back" variant="ghost" fullWidth onPress={() => router.back()} />
    </Screen>
  );
}
