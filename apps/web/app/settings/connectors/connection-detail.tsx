// Connection detail (§3): settings, destinations, health, access — in order.
//
// Purpose: the four things §3 says a modal cannot hold. §3 asks for a page; the
// roster reaches this panel from the card and a Back control returns, so the
// interaction is a detail step rather than an inline expansion of the card.
//
// What this round can and cannot show, stated plainly rather than faked:
//
// - **Settings.** `GET /api/v1/connections` carries no `settings_schema`, and
//   there is no per-field renderer in this lane. The section renders the typed
//   "not available yet" note. The next lane's work is `SchemaForm` over the
//   manifest schema; inventing a form from the connection label would be a
//   form that ignores the connector's own declarations.
// - **Destinations.** The response carries `bindings_count` and no binding
//   rows, so the section shows the count and says what is missing. A
//   destination that is disabled cannot be dimmed if it was never returned.
// - **Health.** Status, last success and the vendor's own failure text are all
//   in the response; a last-failure *timestamp* is not. `Test connection` is
//   wired only for the connector kinds the existing canary endpoint accepts
//   (`parse_kind`: slack, teams, rewst, voip). Every other connector gets the
//   typed note instead of a button that answers 400 — a dead control is worse
//   than an honest gap.
// - **Access.** Read-only this round: the audience is not in the response, so
//   the section names the default and the rule that install and change are
//   owner/admin. It does not claim to know this connection's audience.
"use client";

import { useState, type ReactNode } from "react";
import {
  Button,
  Card,
  StatusPill,
  statusOfConnection,
} from "@marula-core/ui";
import { LocalTime } from "../../../components/local-time";
import { NotAvailable } from "../../../components/settings/settings-shell";
import { api } from "../../../lib/api";
import { friendlyMutationText, resultErrorText } from "../../../lib/error-copy";
import type { components } from "@marula-core/sdk";
import type { ConnectionView } from "../../../lib/connectors/types";

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
  children,
}: {
  id: string;
  title: string;
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
      <h3 id={id} style={{ margin: 0, fontSize: 13, fontWeight: 600, color: "var(--m-text)" }}>
        {title}
      </h3>
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

export function ConnectionDetail({
  connection,
  siblings,
  onSelect,
  onBack,
}: {
  connection: ConnectionView;
  /** Other connections on the same connector (the per-client list, §3). */
  siblings: ConnectionView[];
  onSelect: (connection: ConnectionView) => void;
  onBack: () => void;
}): React.JSX.Element {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<TestResult | null>(null);
  const [testError, setTestError] = useState<string | null>(null);

  const status = statusOfConnection(connection.status);
  const canTest = TESTABLE_CONNECTORS.has(connection.connector.trim().toLowerCase());

  const runTest = async (): Promise<void> => {
    setTesting(true);
    setTestError(null);
    setResult(null);
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
      setResult(res.data);
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
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              flexWrap: "wrap",
            }}
          >
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
                    {sibling.scope_client_name !== null
                      ? ` · ${sibling.scope_client_name}`
                      : ""}
                  </Button>
                ))}
              </div>
            </div>
          ) : null}

          <Section id="settings" title="Settings">
            <NotAvailable
              what="A settings form"
              hint="this response carries no settings schema, and the schema renderer is not part of this round. Install-time credentials are entered in the plugin's consent flow."
            />
          </Section>

          <Section id="destinations" title="Destinations">
            <p style={{ margin: 0, fontSize: 13, color: "var(--m-text)" }}>
              {connection.bindings_count === 0
                ? "No destinations bound. A share from this connection is absent until one is added."
                : `${connection.bindings_count} destination${connection.bindings_count === 1 ? "" : "s"} bound.`}
            </p>
            <NotAvailable
              what="The destination list"
              hint="the contract returns a count and not the rows, so a disabled destination cannot be drawn dimmed yet."
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
                <NotAvailable
                  what={`A canary for ${connection.connector}`}
                  hint="POST /api/v1/connectors/{kind}/test accepts slack, teams, rewst and voip today. This connector has no canary endpoint, so there is no button to press."
                />
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

              {result !== null ? (
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
                        TEST_OUTCOME[result.outcome]?.tone === "success"
                          ? "var(--m-status-success-fg)"
                          : TEST_OUTCOME[result.outcome]?.tone === "danger"
                            ? "var(--m-status-danger-fg)"
                            : "var(--m-status-warning-fg)",
                    }}
                  >
                    {TEST_OUTCOME[result.outcome]?.label ?? result.outcome}
                  </span>
                  {result.detail !== null && result.detail !== undefined ? (
                    <span style={{ fontSize: 12, color: "var(--m-text-secondary)" }}>
                      {result.detail}
                    </span>
                  ) : null}
                </div>
              ) : null}
            </div>
          </Section>

          <Section id="access" title="Access">
            <div style={{ display: "grid", gap: 6 }}>
              <Row label="Use">
                All staff (the default). Per-connection audiences are not carried by this
                response yet, so this is the rule, not a read of this connection.
              </Row>
              <Row label="Install and change">
                Owner/admin, through the plugin consent flow. Not editable here.
              </Row>
            </div>
          </Section>
        </div>
      </Card>
    </div>
  );
}
