// Connection detail (§3): settings, destinations, health, access — in order.
//
// Purpose: the four things §3 says a modal cannot hold. The roster reaches this
// panel from the card and a Back control returns, so the interaction is a detail
// step rather than an inline expansion of the card.
//
// What writes here, and how each answers:
//
// - **Settings** render from the pinned manifest's `settings_schema` through
//   `SchemaForm`. Save sends a per-key patch (absent untouched, `null` clears)
//   and the credentials the operator typed into `secrets`, never into
//   `settings`. A field the manifest marks `writeOnly` shows "Configured" from
//   `secrets_set` and is never pre-filled.
// - **Destinations** load and write their own rows; a duplicate answers 409 on
//   the field that caused it.
// - **Health** names status, last success, the vendor's own failure text and a
//   canary result that updates the section rather than a toast.
// - **Access** writes `audience_roles` (`roles.id`). Empty is "all staff with
//   access", stated as a grant, never as an absence.
//
// Removing a connection is destructive and gated by a confirm. A 409 there
// names the destinations and vendor identities still pointing at it — that
// refusal is rendered in the confirm dialog, because retrying the same delete
// cannot change the answer.
"use client";

import { useEffect, useId, useState, type ReactNode } from "react";
import {
  Button,
  Card,
  ConfirmDialog,
  SchemaForm,
  Skeleton,
  StatusPill,
  statusOfConnection,
  validateSettingsValue,
  writeOnlyKeys,
  type SecretDrafts,
  type SettingsErrors,
  type SettingsObject,
} from "@marula-core/ui";
import { LocalTime } from "../../../components/local-time";
import { api } from "../../../lib/api";
import { friendlyMutationText, resultErrorText } from "../../../lib/error-copy";
import { secretsPatch, settingsPatch } from "../../../lib/connectors/forms";
import { deleteConnection, updateConnection } from "../../../lib/connectors/write";
import type { ConnectionView } from "../../../lib/connectors/types";
import type { components } from "@marula-core/sdk";
import { AudienceEditor } from "./audience-editor";
import { DestinationEditor } from "./destination-editor";
import type { PinnedSchemaState } from "./pinned-schema";

type TestResult = components["schemas"]["TestResult"];

/**
 * The kinds `POST /api/v1/connectors/{kind}/test` accepts today. Read from
 * `crates/core-api/src/routes/integrations.rs::parse_kind`; a kind absent here
 * has no canary to call, and the section says so.
 */
const TESTABLE_CONNECTORS = new Set(["slack", "teams", "rewst", "voip"]);

const TEST_OUTCOME: Record<string, { label: string; tone: "success" | "danger" | "warning" }> = {
  delivered: { label: "Delivered", tone: "success" },
  refused: { label: "Refused", tone: "danger" },
  bad_config: { label: "Configuration rejected", tone: "danger" },
  unreachable: { label: "Unreachable", tone: "warning" },
};

function Section({
  id,
  title,
  aside,
  children,
}: {
  id: string;
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <section
      aria-labelledby={id}
      data-testid={`detail-section-${id}`}
      style={{
        display: "grid",
        gap: 8,
        paddingTop: 14,
        borderTop: "1px solid var(--m-border)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
        <h3 id={id} style={{ margin: 0, fontSize: 13, fontWeight: 600, color: "var(--m-text)" }}>
          {title}
        </h3>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }): React.JSX.Element {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "140px minmax(0, 1fr)", gap: 8 }}>
      <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>{label}</span>
      <div style={{ fontSize: 13, color: "var(--m-text)", minWidth: 0 }}>{children}</div>
    </div>
  );
}

function SettingsSkeleton(): ReactNode {
  return (
    <div style={{ display: "grid", gap: 10 }} aria-hidden="true">
      {[0, 1].map((row) => (
        <div key={row} style={{ display: "grid", gap: 6 }}>
          <Skeleton width="24%" height={10} />
          <Skeleton shape="block" width="100%" height={36} />
        </div>
      ))}
    </div>
  );
}

/** Non-credential settings as the form holds them: no writeOnly, no nulls. */
function editableSettings(connection: ConnectionView, credentials: ReadonlySet<string>): SettingsObject {
  const next: SettingsObject = {};
  for (const [key, value] of Object.entries(connection.settings ?? {})) {
    if (credentials.has(key)) continue;
    if (value === null) continue;
    next[key] = value;
  }
  return next;
}

export function ConnectionDetail({
  connection,
  siblings,
  pinned,
  onReloadSchema,
  onSaved,
  onDeleted,
  onSelect,
  onBindingsCountChange,
  onBack,
}: {
  connection: ConnectionView;
  /** Other connections on the same connector (the per-client list, §3). */
  siblings: ConnectionView[];
  pinned: PinnedSchemaState;
  onReloadSchema: () => void;
  /** The server-confirmed view after a save. Never an optimistic copy. */
  onSaved: (connection: ConnectionView) => void;
  onDeleted: (id: string) => void;
  onSelect: (connection: ConnectionView) => void;
  /** The roster's per-card count follows destination writes. */
  onBindingsCountChange?: (connectionId: string, enabledCount: number) => void;
  onBack: () => void;
}): React.JSX.Element {
  const generated = useId();
  const prefix = `conn-${generated.replace(/[^a-zA-Z0-9_-]+/g, "")}`;

  const [settings, setSettings] = useState<SettingsObject>({});
  const [secrets, setSecrets] = useState<SecretDrafts>({});
  const [settingsErrors, setSettingsErrors] = useState<SettingsErrors>({});
  const [settingsFailure, setSettingsFailure] = useState<string | null>(null);
  const [savingSettings, setSavingSettings] = useState(false);
  const [settingsSaved, setSettingsSaved] = useState(false);

  const [audience, setAudience] = useState<string[]>(connection.audience_roles);
  const [audienceIncomplete, setAudienceIncomplete] = useState(false);
  const [audienceFailure, setAudienceFailure] = useState<string | null>(null);
  const [savingAudience, setSavingAudience] = useState(false);
  const [audienceSaved, setAudienceSaved] = useState(false);

  const [bindingsCount, setBindingsCount] = useState(connection.bindings_count);
  const [removeOpen, setRemoveOpen] = useState(false);

  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);
  const [testError, setTestError] = useState<string | null>(null);

  const schema = pinned.status === "ready" ? pinned.schema : null;
  const credentials = new Set(schema === null ? [] : writeOnlyKeys(schema));
  const original = editableSettings(connection, credentials);

  // A different connection is a different form. Reset rather than carry the
  // previous row's drafts into this one — a retyped credential must never be
  // saved against the wrong connection.
  useEffect(() => {
    setSettings(editableSettings(connection, new Set()));
    setSecrets({});
    setSettingsErrors({});
    setSettingsFailure(null);
    setSettingsSaved(false);
    setAudience(connection.audience_roles);
    setAudienceIncomplete(false);
    setAudienceFailure(null);
    setAudienceSaved(false);
    setBindingsCount(connection.bindings_count);
    setTestResult(null);
    setTestError(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by connection identity only
  }, [connection.id]);

  const status = statusOfConnection(connection.status);
  const canTest = TESTABLE_CONNECTORS.has(connection.connector.trim().toLowerCase());
  const settingsDirty =
    schema !== null &&
    (Object.keys(settingsPatch(original, settings, [...credentials])).length > 0 ||
      Object.keys(secretsPatch(secrets)).length > 0);

  const saveSettings = async (): Promise<void> => {
    if (schema === null) return;
    setSettingsSaved(false);
    setSettingsFailure(null);
    const errors = validateSettingsValue(schema, settings, secrets, connection.secrets_set);
    if (Object.keys(errors).length > 0) {
      setSettingsErrors(errors);
      setSettingsFailure("Some values need attention before this can be saved.");
      return;
    }
    setSettingsErrors({});
    setSavingSettings(true);
    const patch = settingsPatch(original, settings, [...credentials]);
    const secretValues = secretsPatch(secrets);
    const result = await updateConnection(connection.id, {
      ...(Object.keys(patch).length === 0 ? {} : { settings: patch }),
      ...(Object.keys(secretValues).length === 0 ? {} : { secrets: secretValues }),
    });
    setSavingSettings(false);
    if (!result.ok) {
      if (result.failure.field !== undefined) {
        setSettingsErrors({ [result.failure.field]: result.failure.message });
      }
      setSettingsFailure(result.failure.message);
      return;
    }
    const saved = result.data as ConnectionView;
    setSecrets({});
    setSettingsSaved(true);
    onSaved(saved);
  };

  const saveAudience = async (): Promise<void> => {
    setAudienceSaved(false);
    // "Only these roles" with none picked would post an empty list, which the
    // server reads as "all staff" — the opposite of what the operator meant.
    if (audienceIncomplete) {
      setAudienceFailure(
        "Pick at least one role, or choose all staff with access — an empty list is the default, not a lock.",
      );
      return;
    }
    setSavingAudience(true);
    const result = await updateConnection(connection.id, { audience_roles: audience });
    setSavingAudience(false);
    if (!result.ok) {
      setAudienceFailure(result.failure.message);
      return;
    }
    setAudienceSaved(true);
    onSaved(result.data as ConnectionView);
  };

  const runTest = async (): Promise<void> => {
    setTesting(true);
    setTestError(null);
    setTestResult(null);
    try {
      const res = await api.api.POST("/api/v1/connectors/{kind}/test", {
        params: { path: { kind: connection.connector } },
      });
      if (res.data === undefined) {
        setTestError(resultErrorText(res, "The connection test could not run."));
        return;
      }
      // The canary result updates this section, never a toast: a message that
      // is gone before it is read is not a health report (§3).
      setTestResult(res.data);
    } catch (error) {
      setTestError(friendlyMutationText(error, "The connection test could not run."));
    } finally {
      setTesting(false);
    }
  };

  return (
    <div style={{ display: "grid", gap: 12 }} data-testid="connection-detail">
      <div>
        <Button size="sm" variant="ghost" onClick={onBack} data-testid="connection-detail-back">
          ← All connectors
        </Button>
      </div>

      <Card>
        <div style={{ display: "grid", gap: 14, padding: 16 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
            <h2 style={{ margin: 0, fontSize: 18, fontWeight: 600 }}>{connection.label}</h2>
            <StatusPill status={status} />
            <code
              style={{
                fontFamily: "var(--m-font-mono)",
                fontSize: 12,
                color: "var(--m-text-muted)",
              }}
            >
              {connection.connector}
            </code>
            {connection.scope_client_name !== null ? (
              <span
                style={{ fontSize: 12, color: "var(--m-text-secondary)" }}
                data-testid="connection-scope-client"
              >
                Client · {connection.scope_client_name}
              </span>
            ) : connection.scope_client_id !== null ? (
              // The contract has a name for every scoped connection; a null
              // name means the server has not resolved it. Say that rather
              // than print a uuid at a human.
              <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
                Client-scoped connection (name not reported)
              </span>
            ) : null}
            <span style={{ marginLeft: "auto" }}>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setRemoveOpen(true)}
                data-testid="connection-remove"
              >
                Remove connection
              </Button>
            </span>
          </div>

          {siblings.length > 1 ? (
            <div style={{ display: "grid", gap: 4 }} data-testid="connection-siblings">
              <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
                Connections on this connector
              </span>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
                {siblings.map((sibling) => (
                  <Button
                    key={sibling.id}
                    size="sm"
                    variant={sibling.id === connection.id ? "secondary" : "ghost"}
                    aria-current={sibling.id === connection.id ? "true" : undefined}
                    onClick={() => onSelect(sibling)}
                  >
                    {sibling.label}
                    {sibling.scope_client_name !== null ? ` · ${sibling.scope_client_name}` : ""}
                  </Button>
                ))}
              </div>
            </div>
          ) : null}

          <Section id="settings" title="Settings">
            {pinned.status === "loading" || pinned.status === "idle" ? (
              <SettingsSkeleton />
            ) : pinned.status === "error" ? (
              <div
                role="alert"
                style={{
                  display: "grid",
                  gap: 8,
                  padding: "10px 12px",
                  borderRadius: "var(--m-radius-container)",
                  background: "var(--m-status-danger-bg)",
                  color: "var(--m-status-danger-fg)",
                  fontSize: 12,
                }}
                data-testid="connection-schema-error"
              >
                <span>{pinned.message}</span>
                <div>
                  <Button size="sm" variant="secondary" onClick={onReloadSchema}>
                    Retry
                  </Button>
                </div>
              </div>
            ) : pinned.schema === null ? (
              <p style={{ margin: 0, fontSize: 13, color: "var(--m-text-secondary)" }}>
                This connector declares no settings.{" "}
                {connection.secrets_set.length === 0
                  ? "No credential is configured."
                  : `Credentials configured: ${connection.secrets_set.join(", ")}.`}
              </p>
            ) : (
              <div style={{ display: "grid", gap: 10 }}>
                <SchemaForm
                  schema={pinned.schema}
                  value={settings}
                  onChange={(next) => {
                    setSettings(next);
                    setSettingsSaved(false);
                  }}
                  secrets={secrets}
                  onSecretsChange={(next) => {
                    setSecrets(next);
                    setSettingsSaved(false);
                  }}
                  secretsSet={connection.secrets_set}
                  serverErrors={settingsErrors}
                  disabled={savingSettings}
                  idPrefix={prefix}
                />
                {settingsFailure !== null ? (
                  <p
                    role="alert"
                    style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}
                    data-testid="connection-settings-error"
                  >
                    {settingsFailure}
                  </p>
                ) : null}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => void saveSettings()}
                    disabled={savingSettings || !settingsDirty}
                    data-testid="connection-settings-save"
                  >
                    {savingSettings ? "Saving…" : "Save settings"}
                  </Button>
                  {!settingsDirty ? (
                    <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
                      {settingsSaved ? "Saved." : "No changes yet."}
                    </span>
                  ) : null}
                </div>
              </div>
            )}
          </Section>

          <Section id="destinations" title="Destinations">
            <p style={{ margin: 0, fontSize: 13, color: "var(--m-text)" }}>
              {bindingsCount === 0
                ? "No enabled destination. A share from this connection is absent until one is added."
                : `${String(bindingsCount)} enabled destination${bindingsCount === 1 ? "" : "s"}.`}
            </p>
            <DestinationEditor
              key={connection.id}
              connectionId={connection.id}
              onCountChange={(count) => {
                setBindingsCount(count);
                onBindingsCountChange?.(connection.id, count);
              }}
            />
          </Section>

          <Section id="health" title="Health">
            <div style={{ display: "grid", gap: 6 }}>
              <Row label="Status">
                <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                  <StatusPill status={status} />
                  {connection.status_detail !== null ? (
                    <span style={{ color: "var(--m-text-secondary)" }}>
                      {connection.status_detail}
                    </span>
                  ) : null}
                </span>
              </Row>
              <Row label="Last success">
                <LocalTime value={connection.last_success_at} kind="datetime" />
              </Row>
              {/* A null timestamp means the connection has never failed, so
                  the row is absent. Printing "—" here would read as "failed
                  at an unknown time", which is a different claim. */}
              {connection.last_failure_at !== null ? (
                <Row label="Last failure">
                  <LocalTime value={connection.last_failure_at} kind="datetime" />
                </Row>
              ) : null}
              {connection.status_detail_vendor !== null ? (
                <Row label="Vendor message">
                  <span data-testid="connection-vendor-error">
                    {connection.status_detail_vendor}
                  </span>
                </Row>
              ) : null}
            </div>

            <div style={{ display: "grid", gap: 6, justifyItems: "start" }}>
              {canTest ? (
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void runTest()}
                  disabled={testing}
                  data-testid="connection-test"
                >
                  {testing ? "Testing…" : "Test connection"}
                </Button>
              ) : (
                <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
                  This connector has no canary endpoint, so there is no test to run. Its status
                  still moves when it syncs.
                </p>
              )}

              {testError !== null ? (
                <p
                  role="alert"
                  style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}
                  data-testid="connection-test-error"
                >
                  {testError}
                </p>
              ) : null}

              {testResult !== null ? (
                <div
                  role="status"
                  data-testid="connection-test-result"
                  style={{ display: "grid", gap: 2 }}
                >
                  <span
                    style={{
                      fontSize: 12,
                      fontWeight: 600,
                      color:
                        TEST_OUTCOME[testResult.outcome]?.tone === "success"
                          ? "var(--m-status-success-fg)"
                          : TEST_OUTCOME[testResult.outcome]?.tone === "danger"
                            ? "var(--m-status-danger-fg)"
                            : "var(--m-status-warning-fg)",
                    }}
                  >
                    {TEST_OUTCOME[testResult.outcome]?.label ?? testResult.outcome}
                  </span>
                  {testResult.detail !== null && testResult.detail !== undefined ? (
                    <span style={{ fontSize: 12, color: "var(--m-text-secondary)" }}>
                      {testResult.detail}
                    </span>
                  ) : null}
                </div>
              ) : null}
            </div>
          </Section>

          <Section id="access" title="Access">
            <AudienceEditor
              value={audience}
              onChange={(next) => {
                setAudience(next);
                setAudienceSaved(false);
                setAudienceFailure(null);
              }}
              onIncompleteChange={setAudienceIncomplete}
              disabled={savingAudience}
              idPrefix={prefix}
            />
            <Row label="Install and change">
              Owner or admin, through the plugin consent flow. Not editable here.
            </Row>
            {audienceFailure !== null ? (
              <p
                role="alert"
                style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}
                data-testid="connection-audience-error"
              >
                {audienceFailure}
              </p>
            ) : null}
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <Button
                size="sm"
                variant="primary"
                onClick={() => void saveAudience()}
                disabled={savingAudience || audienceIncomplete}
                data-testid="connection-audience-save"
              >
                {savingAudience ? "Saving…" : "Save access"}
              </Button>
              {audienceSaved ? (
                <span style={{ fontSize: 12, color: "var(--m-text-secondary)" }}>Saved.</span>
              ) : null}
            </div>
          </Section>
        </div>
      </Card>

      <ConfirmDialog
        open={removeOpen}
        onOpenChange={setRemoveOpen}
        title={`Remove ${connection.label}?`}
        description="The connection and its destinations stop being offered. A connection that external references or vendor identities still point at cannot be removed — the server names them, and revoking the installation is the path that takes those with it."
        confirmLabel="Remove connection"
        onConfirm={async () => {
          const result = await deleteConnection(connection.id);
          if (!result.ok) {
            // The dialog renders this in place; retrying cannot change a 409.
            throw new Error(result.failure.message);
          }
          setRemoveOpen(false);
          onDeleted(connection.id);
        }}
        testId="connection-remove-confirm"
      />
    </div>
  );
}
