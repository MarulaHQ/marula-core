// Create connection — the first-run write path (§2 Connect, §3 Settings).
//
// Purpose: a connector that is installed and not yet configured is the state
// every install starts in, and the card used to send that operator to Settings
// → Plugins because no write path existed. This dialog is that write path: it
// opens from the card's `Connect`, renders the connector's own schema, and
// sends `{ label, scope_client_id?, settings, secrets, audience_roles? }` to
// `POST /api/v1/connector-installations/{installation_id}/connections`.
//
// The two things that are easy to get wrong, and are decided here:
//
// - **Credentials travel in `secrets`, never in `settings`.** The schema marks
//   them `writeOnly`; the form reports them separately, and the server refuses
//   a credential sent through `settings`. A save that mixed the two would fail
//   visibly, which is the point.
// - **Nothing closes until the server confirms.** The dialog stays open with
//   the field or banner that explains a refusal, and only an answered 201
//   closes it and hands the created connection back.
//
// Design research (lazyweb `lazyweb_search_screens`, 2026-11-14): Auth0's
// create-user modal and Tray's auth drawer both draw a short labelled column
// with one primary action in the footer; both keep the account picker above the
// fields. This dialog keeps that order — identity, scope, settings,
// credentials, audience — and does not become a wizard for a single form.
// `docs/theme.md` owns palette, type, spacing and the ≤120 ms motion ceiling.
"use client";

import { useCallback, useEffect, useId, useState, type ReactNode } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  FieldError,
  FieldHint,
  Input,
  Label,
  SchemaForm,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  validateSettingsValue,
  writeOnlyKeys,
  type SecretDrafts,
  type SettingsErrors,
  type SettingsObject,
  type SettingsSchemaNode,
} from "@marula-core/ui";
import { api } from "../../../lib/api";
import { resultErrorText } from "../../../lib/error-copy";
import { createConnection } from "../../../lib/connectors/write";
import type {
  ClientOption,
  ConnectionView,
  ConnectorInstallationView,
} from "../../../lib/connectors/types";
import { connectorDisplayName } from "../../../lib/connectors/roster";
import { AudienceEditor } from "./audience-editor";
import { usePinnedSchema } from "./pinned-schema";

const EMPTY_SCHEMA: SettingsSchemaNode = { type: "object", properties: {} };

function FormSkeleton(): ReactNode {
  return (
    <div style={{ display: "grid", gap: 12 }} aria-hidden="true">
      <div style={{ display: "grid", gap: 6 }}>
        <Skeleton width="22%" height={10} />
        <Skeleton shape="block" width="100%" height={36} />
      </div>
      <div style={{ display: "grid", gap: 6 }}>
        <Skeleton width="30%" height={10} />
        <Skeleton shape="block" width="100%" height={36} />
      </div>
    </div>
  );
}

export function CreateConnectionDialog({
  installation,
  open,
  onOpenChange,
  onCreated,
}: {
  installation: ConnectorInstallationView | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (connection: ConnectionView) => void;
}): ReactNode {
  const generated = useId();
  const prefix = `create-${generated.replace(/[^a-zA-Z0-9_-]+/g, "")}`;
  const installationId = installation?.installation_id ?? null;
  const pinned = usePinnedSchema(open ? installationId : null);

  const [label, setLabel] = useState("");
  const [labelError, setLabelError] = useState<string | null>(null);
  const [scopeClientId, setScopeClientId] = useState<string>("org");
  const [clients, setClients] = useState<ClientOption[] | null>(null);
  const [clientsError, setClientsError] = useState<string | null>(null);
  const [settings, setSettings] = useState<SettingsObject>({});
  const [secrets, setSecrets] = useState<SecretDrafts>({});
  const [audience, setAudience] = useState<string[]>([]);
  const [fieldErrors, setFieldErrors] = useState<SettingsErrors>({});
  const [failure, setFailure] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Reopening is a fresh create: a half-filled previous attempt must not leak
  // into the next connector's form.
  useEffect(() => {
    if (!open) return;
    setLabel("");
    setLabelError(null);
    setScopeClientId("org");
    setSettings({});
    setSecrets({});
    setAudience([]);
    setFieldErrors({});
    setFailure(null);
    setSaving(false);
  }, [open, installationId]);

  const loadClients = useCallback(async () => {
    setClients(null);
    setClientsError(null);
    try {
      const result = await api.api.GET("/api/v1/clients");
      if (result.data === undefined) {
        setClientsError(resultErrorText(result, "The client list could not be loaded."));
        return;
      }
      setClients(
        (result.data as Array<{ id: string; name: string }>).map((client) => ({
          id: client.id,
          name: client.name,
        })),
      );
    } catch (error) {
      setClientsError(
        error instanceof Error
          ? `The client list could not be loaded (${error.message}).`
          : "The client list could not be loaded.",
      );
    }
  }, []);

  useEffect(() => {
    if (open) void loadClients();
  }, [open, loadClients]);

  if (installation === null) return null;

  const schema = pinned.state.status === "ready" ? (pinned.state.schema ?? EMPTY_SCHEMA) : null;
  const name = connectorDisplayName(installation.connector);
  const labelId = `${prefix}-label`;
  const labelErrorId = `${prefix}-label-error`;
  const scopeHintId = `${prefix}-scope-hint`;

  const save = async (): Promise<void> => {
    setFailure(null);
    setLabelError(null);
    if (label.trim() === "") {
      setLabelError("Give this connection a name you will recognise — it is how the card lists it.");
      return;
    }
    if (schema === null) {
      setFailure("The connector's settings have not loaded yet, so there is nothing to save.");
      return;
    }
    const errors = validateSettingsValue(schema, settings, secrets, []);
    if (Object.keys(errors).length > 0) {
      setFieldErrors(errors);
      setFailure("Some values need attention before this can be saved.");
      return;
    }
    setFieldErrors({});
    setSaving(true);
    const credentialKeys = new Set(writeOnlyKeys(schema));
    const settingsBody: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(settings)) {
      if (credentialKeys.has(key)) continue;
      settingsBody[key] = value;
    }
    const result = await createConnection(installation.installation_id, {
      label: label.trim(),
      scope_client_id: scopeClientId === "org" ? null : scopeClientId,
      settings: settingsBody,
      ...(Object.keys(secrets).length === 0 ? {} : { secrets }),
      audience_roles: audience,
    });
    setSaving(false);
    if (!result.ok) {
      if (result.failure.field === "label") {
        setLabelError(result.failure.message);
      } else if (result.failure.field !== undefined) {
        setFieldErrors({ [result.failure.field]: result.failure.message });
      }
      setFailure(result.failure.message);
      return;
    }
    onCreated(result.data as ConnectionView);
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (saving ? undefined : onOpenChange(next))}>
      <DialogContent
        title={`Connect ${name}`}
        description="One connection is one use of this connector: its credentials, its settings, and who may reach it."
        width={560}
      >
        <div style={{ display: "grid", gap: 12 }} data-testid="create-connection">
          <div style={{ display: "grid", gap: 4 }}>
            <Label htmlFor={labelId}>Name</Label>
            <Input
              id={labelId}
              value={label}
              placeholder={`${name} — Acme`}
              aria-describedby={labelError !== null ? labelErrorId : undefined}
              aria-invalid={labelError !== null ? true : undefined}
              onChange={(event) => {
                setLabel(event.target.value);
                setLabelError(null);
              }}
            />
            {labelError !== null ? <FieldError id={labelErrorId}>{labelError}</FieldError> : null}
          </div>

          <div style={{ display: "grid", gap: 4 }}>
            <Label htmlFor={`${prefix}-scope`}>Client</Label>
            {clientsError !== null ? (
              <div
                style={{ display: "grid", gap: 6 }}
                data-testid="create-clients-error"
              >
                <p role="alert" style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}>
                  {clientsError}
                </p>
                <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
                  A client-scoped connection needs the client list. The org-wide connection below
                  still works.
                </p>
                <div>
                  <Button size="sm" variant="secondary" onClick={() => void loadClients()}>
                    Retry
                  </Button>
                </div>
              </div>
            ) : clients === null ? (
              <Skeleton shape="block" width="100%" height={36} />
            ) : (
              <Select value={scopeClientId} onValueChange={setScopeClientId}>
                <SelectTrigger id={`${prefix}-scope`} aria-describedby={scopeHintId}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="org">Whole organisation</SelectItem>
                  {clients.map((client) => (
                    <SelectItem key={client.id} value={client.id}>
                      {client.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
            <FieldHint id={scopeHintId}>
              A client-scoped connection is still refused to anyone who cannot see that client.
            </FieldHint>
          </div>

          <div style={{ display: "grid", gap: 6 }}>
            <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>Settings</span>
            {pinned.state.status === "loading" || pinned.state.status === "idle" ? (
              <FormSkeleton />
            ) : pinned.state.status === "error" ? (
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
                data-testid="create-schema-error"
              >
                <span>{pinned.state.message}</span>
                <div>
                  <Button size="sm" variant="secondary" onClick={pinned.reload}>
                    Retry
                  </Button>
                </div>
              </div>
            ) : pinned.state.schema === null ? (
              <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-secondary)" }}>
                This connector declares no settings. Its credentials, if any, are the whole
                configuration.
              </p>
            ) : (
              <SchemaForm
                schema={pinned.state.schema}
                value={settings}
                onChange={setSettings}
                secrets={secrets}
                onSecretsChange={setSecrets}
                serverErrors={fieldErrors}
                disabled={saving}
                idPrefix={prefix}
              />
            )}
          </div>

          <AudienceEditor value={audience} onChange={setAudience} disabled={saving} idPrefix={prefix} />

          {failure !== null ? (
            <p
              role="alert"
              style={{
                margin: 0,
                padding: "8px 10px",
                borderRadius: "var(--m-radius-control)",
                background: "var(--m-status-danger-bg)",
                color: "var(--m-status-danger-fg)",
                fontSize: 12,
              }}
              data-testid="create-connection-error"
            >
              {failure}
            </p>
          ) : null}

          <DialogActions>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => onOpenChange(false)}
              disabled={saving}
            >
              Cancel
            </Button>
            <Button
              size="sm"
              variant="primary"
              onClick={() => void save()}
              disabled={saving || schema === null}
              data-testid="create-connection-save"
            >
              {saving ? "Connecting…" : "Connect"}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
