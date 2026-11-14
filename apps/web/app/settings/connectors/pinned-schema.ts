// The pinned settings schema, loaded once per installation.
//
// Why a separate read: the roster SQL selects `settings_schema` and the view
// does not carry it, while every connection under an installation shares the
// pinned schema. The route that does serve it is
// `GET /plugins/installed/{id}/settings` — the install's own settings view —
// and `installation_id` on a connection is that install's id. One extra GET per
// open panel is cheaper than a schema per connection row, and it keeps this
// client honest about what the server actually declared.
//
// The result is deliberately three-shaped: a connector that declares NO
// settings is a complete connector and should still be configurable, while a
// schema outside the accepted subset is a manifest the write route would refuse
// — the two must not collapse into one empty form.
"use client";

import { useCallback, useEffect, useState } from "react";
import { readSettingsSchema, type SettingsSchemaNode } from "@marula-core/ui";
import { fetchPinnedSettingsSchema } from "../../../lib/connectors/write";

export type PinnedSchemaState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; schema: SettingsSchemaNode | null }
  | { status: "error"; message: string };

export function usePinnedSchema(installationId: string | null): {
  state: PinnedSchemaState;
  reload: () => void;
} {
  const [state, setState] = useState<PinnedSchemaState>({ status: "idle" });
  const [attempt, setAttempt] = useState(0);

  const reload = useCallback(() => {
    setAttempt((value) => value + 1);
  }, []);

  useEffect(() => {
    if (installationId === null) {
      setState({ status: "idle" });
      return;
    }
    let cancelled = false;
    setState({ status: "loading" });
    void (async () => {
      const result = await fetchPinnedSettingsSchema(installationId);
      if (cancelled) return;
      if (!result.ok) {
        setState({ status: "error", message: result.failure.message });
        return;
      }
      // No declared schema is not an error: the connector contributes
      // credentials only, or nothing at all.
      if (result.data === null) {
        setState({ status: "ready", schema: null });
        return;
      }
      const read = readSettingsSchema(result.data);
      if (!read.ok) {
        setState({
          status: "error",
          message: `This connector's settings schema cannot be rendered: ${read.reason}`,
        });
        return;
      }
      setState({ status: "ready", schema: read.schema });
    })();
    return () => {
      cancelled = true;
    };
  }, [installationId, attempt]);

  return { state, reload };
}
