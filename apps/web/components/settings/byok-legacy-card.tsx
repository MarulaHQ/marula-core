"use client";

// The legacy BYOK surface (Wave 9 lane 8) preserved verbatim inside the
// rebuilt settings/ai page: per-provider presence-only keys, replace,
// live connection test, remove. Keys are sealed at rest and never
// render back.
import {
  Badge,
  Button,
  Card,
  Input,
  Spinner,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { SettingsHeader } from "./settings-shell";
import { ErrorNote } from "../knowledge/ledger";
import { rejectionText } from "../../lib/error-copy";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type ByokOverviewResponse = components["schemas"]["ByokOverviewResponse"];
type ByokSecretMutationResponse = components["schemas"]["ByokSecretMutationResponse"];
type TestConnectionResponse = components["schemas"]["TestConnectionResponse"];

const AI_PROVIDERS = ["openai", "anthropic", "ollama", "custom"] as const;
const CONNECTORS = ["qbo", "xero"] as const;

export function ByokLegacyCard() {
  const auth = useRequireAuth();
  const [overview, setOverview] = useState<ByokOverviewResponse | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [keyDrafts, setKeyDrafts] = useState<Record<string, string>>({});
  const [busyProvider, setBusyProvider] = useState<string | null>(null);
  const [tested, setTested] = useState<Record<string, TestConnectionResponse>>({});

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/settings/ai/status");
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load AI settings.");
      return;
    }
    setOverview(data);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const configured = new Set(overview?.configured_providers ?? []);

  const saveKey = async (provider: string) => {
    const apiKey = (keyDrafts[provider] ?? "").trim();
    if (apiKey === "") {
      setActionError(`Enter a key for ${provider} first.`);
      return;
    }
    setActionError(null);
    setBusyProvider(provider);
    const previous = overview;
    // Optimistic presence flip — the key itself never renders back.
    if (overview !== null) {
      setOverview({
        ...overview,
        configured_providers: configured.has(provider)
          ? overview.configured_providers
          : [...overview.configured_providers, provider],
      });
    }
    const result = await runEnvelopeMutation<
      components["schemas"]["PutByokSecretRequest"],
      ByokSecretMutationResponse
    >({
      describe: `Save ${provider} key`,
      entityId: `byok:${provider}`,
      optimistic: () => {},
      revert: () => setOverview(previous),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        provider,
        api_key: apiKey,
      }),
      request: (client, body) => client.PUT("/api/v1/settings/ai/byok", { body }),
    });
    setBusyProvider(null);
    if (!result.ok) {
      setActionError(rejectionText(result, `Saving the ${provider} key was rejected.`));
      return;
    }
    setKeyDrafts((prev) => ({ ...prev, [provider]: "" }));
    void refresh();
  };

  const removeKey = async (provider: string) => {
    setActionError(null);
    setBusyProvider(provider);
    const previous = overview;
    if (overview !== null) {
      setOverview({
        ...overview,
        configured_providers: overview.configured_providers.filter((p) => p !== provider),
      });
    }
    const { error } = await api.api.DELETE("/api/v1/settings/ai/byok/{provider}", {
      params: { path: { provider } },
    });
    setBusyProvider(null);
    if (error !== undefined) {
      setOverview(previous);
      setActionError(`Removing the ${provider} key failed.`);
      return;
    }
    void refresh();
  };

  const testProvider = async (provider: string) => {
    setActionError(null);
    setBusyProvider(provider);
    const { data, error } = await api.api.POST("/api/v1/settings/ai/test", {
      body: { provider },
    });
    setBusyProvider(null);
    if (error !== undefined || data === undefined) {
      setActionError(`The ${provider} connection test could not run.`);
      return;
    }
    setTested((prev) => ({ ...prev, [provider]: data }));
  };

  if (auth.status !== "authed") return null;

  const row = (provider: string, kind: "ai" | "connector") => {
    const isConfigured = configured.has(provider);
    const test = tested[provider];
    return (
      <div
        key={provider}
        className="flex flex-wrap items-center gap-3 rounded-[var(--m-radius-container)] border p-3"
        style={{ borderColor: "var(--m-border)" }}
      >
        <div className="min-w-40">
          <p className="text-[13px] font-semibold">{provider}</p>
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            {kind === "connector" ? "export-sync connector" : "AI provider"}
          </p>
        </div>
        {isConfigured ? (
          <Badge>
            {kind === "connector" ? "key sealed" : "key sealed"} · never displayed
          </Badge>
        ) : (
          <Badge>not configured</Badge>
        )}
        {overview !== null && overview.active_provider === provider && kind === "ai" ? (
          <Badge>active · {overview.active_model}</Badge>
        ) : null}
        <div className="flex flex-1 items-center gap-2">
          <Input
            type="password"
            placeholder={isConfigured ? "Replace key (paste to overwrite)" : "Paste API key"}
            value={keyDrafts[provider] ?? ""}
            onChange={(e) => setKeyDrafts((prev) => ({ ...prev, [provider]: e.target.value }))}
            aria-label={`${provider} API key`}
            className="max-w-72 flex-1"
            autoComplete="off"
          />
          <Button
            size="sm"
            onClick={() => void saveKey(provider)}
            disabled={busyProvider === provider || (keyDrafts[provider] ?? "").trim() === ""}
          >
            {busyProvider === provider ? <Spinner /> : null} Save
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void testProvider(provider)}
            disabled={!isConfigured || busyProvider === provider}
          >
            Test
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void removeKey(provider)}
            disabled={!isConfigured || busyProvider === provider}
          >
            Remove
          </Button>
        </div>
        {test !== undefined ? (
          <p
            role="status"
            className="w-full text-[13px]"
            style={{ color: test.ok ? "var(--m-status-success-fg)" : "var(--m-status-danger-fg)" }}
          >
            {test.ok ? "Connected" : "Failed"}: {test.message}
            {test.latency_ms !== null && test.latency_ms !== undefined
              ? ` (${test.latency_ms} ms)`
              : ""}
          </p>
        ) : null}
      </div>
    );
  };

  return (
    <Card className="mb-4">
      <h2 className="text-sm font-semibold">Bring-your-own providers & connectors</h2>
      <p className="mb-3 mt-1 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
        Embeddings run on the bundled local model when AI features are on and no provider
        below replaces it, so the active line names what would actually serve them. Keys are
        sealed at rest and never rendered back — presence only. Removing a key takes effect
        immediately.
      </p>

      {loadError !== null ? (
        <ErrorNote>{loadError}</ErrorNote>
      ) : overview === null ? (
        <Spinner />
      ) : (
        <>
          {actionError !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {actionError}
            </p>
          ) : null}
          <Card>
            <h2 className="p-4 pb-2 text-sm font-semibold">
              AI providers
              <span className="ml-3 font-normal" style={{ color: "var(--m-text-secondary)" }}>
                active: {overview.active_provider} ({overview.active_model})
              </span>
            </h2>
            <div className="grid gap-3 px-4 pb-4">
              {AI_PROVIDERS.map((p) => row(p, "ai"))}
            </div>
          </Card>
          <Card>
            <h2 className="p-4 pb-2 text-sm font-semibold">Export-sync connectors</h2>
            <div className="grid gap-3 px-4 pb-4">
              {CONNECTORS.map((p) => row(p, "connector"))}
            </div>
          </Card>
        </>
      )}
    </Card>
  );
}
