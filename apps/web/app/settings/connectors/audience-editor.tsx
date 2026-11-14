// Audience editor — §3 Access: who may see and act on this connection.
//
// Purpose: the audience is a closed door, and the copy has to say which door.
// Empty is the shipped default and it means "all staff the module permission
// already allows"; it does NOT mean "no access". So the two modes are drawn as
// two answers to one question, with the default first, rather than as a list
// where a blank list looks like a locked door.
//
// `audience_roles` holds `roles.id`, never names (connector-surface §3), so a
// role renamed later keeps its membership. The editor therefore reads the org's
// roles from `/api/v1/roles` and toggles ids, and its summary names them.
//
// Design research (lazyweb `lazyweb_search_screens`, 2026-11-14): Hygraph,
// Mercury and Doppler all draw role access as a list of named rows with a
// one-line description and a control per row, and all three state the broadest
// option first. That is the composition here; the palette, type scale and the
// 120 ms motion ceiling are `docs/theme.md`'s.
//
// A caller who cannot read roles sees why, in place, with a retry — not an
// empty picker that reads as "this org has no roles".
"use client";

import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Button, Checkbox, FieldError, Skeleton } from "@marula-core/ui";
import { api } from "../../../lib/api";
import { resultErrorText } from "../../../lib/error-copy";
import {
  audienceSummary,
  readRoles,
  toggleRole,
  type RoleOption,
} from "../../../lib/connectors/forms";

type Mode = "all" | "named";

function RoleSkeleton(): ReactNode {
  return (
    <div style={{ display: "grid", gap: 8 }} aria-hidden="true">
      {[0, 1, 2].map((row) => (
        <div key={row} style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <Skeleton shape="block" width={16} height={16} />
          <Skeleton width="42%" height={12} />
        </div>
      ))}
    </div>
  );
}

export function AudienceEditor({
  value,
  onChange,
  disabled = false,
  idPrefix,
  /** Server-side refusal for the audience field, if one landed on it. */
  onIncompleteChange,
  error,
}: {
  value: string[];
  onChange: (next: string[]) => void;
  /**
   * True while "Only these roles" is chosen with nothing ticked. The caller
   * blocks Save: posting an empty list means all staff, which is the opposite
   * of the mode the operator picked.
   */
  onIncompleteChange?: (incomplete: boolean) => void;
  disabled?: boolean;
  idPrefix?: string;
  /** Server-side refusal for the audience field, if one landed on it. */
  error?: string;
}): ReactNode {
  const generated = useId();
  const prefix = idPrefix ?? `audience-${generated.replace(/[^a-zA-Z0-9_-]+/g, "")}`;
  const [roles, setRoles] = useState<RoleOption[] | null>(null);
  const [rolesError, setRolesError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>(value.length === 0 ? "all" : "named");
  const seenRoles = useRef(value.length);

  // The server is the source of the audience: a save that cleared it returns
  // an empty list, and the editor follows rather than keeping its old mode.
  // Only a transition counts — choosing "Only these roles" before ticking one
  // is a legitimate (if incomplete) state, not a reset.
  useEffect(() => {
    if (value.length === 0 && seenRoles.current > 0) setMode("all");
    seenRoles.current = value.length;
  }, [value]);

  const incomplete = mode === "named" && value.length === 0;
  useEffect(() => {
    onIncompleteChange?.(incomplete);
  }, [incomplete, onIncompleteChange]);

  const loadRoles = useCallback(async () => {
    setRolesError(null);
    try {
      const result = await api.api.GET("/api/v1/roles");
      if (result.data === undefined) {
        setRolesError(resultErrorText(result, "The role list could not be loaded."));
        return;
      }
      setRoles(readRoles(result.data));
    } catch (failure) {
      setRolesError(
        failure instanceof Error
          ? `The role list could not be loaded (${failure.message}).`
          : "The role list could not be loaded.",
      );
    }
  }, []);

  useEffect(() => {
    void loadRoles();
  }, [loadRoles]);

  const chooseAll = (): void => {
    setMode("all");
    onChange([]);
  };

  const chooseNamed = (): void => {
    setMode("named");
    // Do not clear the selection: switching to "named" with nothing picked is
    // the empty list again, which would silently mean "all staff". The hint
    // below asks for a role instead.
  };

  const errorId = `${prefix}-error`;
  const hintId = `${prefix}-hint`;

  return (
    <div
      style={{ display: "grid", gap: 10 }}
      role="group"
      aria-labelledby={`${prefix}-label`}
      aria-describedby={error !== undefined ? `${hintId} ${errorId}` : hintId}
      data-testid="audience-editor"
    >
      <span id={`${prefix}-label`} style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
        Who may use this connection
      </span>

      <div style={{ display: "grid", gap: 8 }}>
        <label style={{ display: "flex", alignItems: "flex-start", gap: 8, cursor: "pointer" }}>
          <input
            type="radio"
            name={`${prefix}-mode`}
            checked={mode === "all"}
            disabled={disabled}
            onChange={chooseAll}
            style={{ marginTop: 3 }}
          />
          <span>
            <span style={{ display: "block", fontSize: 13, color: "var(--m-text)" }}>
              All staff with access
            </span>
            <span style={{ display: "block", fontSize: 12, color: "var(--m-text-muted)" }}>
              Everyone the connector module already lets in. This is the default.
            </span>
          </span>
        </label>

        <label style={{ display: "flex", alignItems: "flex-start", gap: 8, cursor: "pointer" }}>
          <input
            type="radio"
            name={`${prefix}-mode`}
            checked={mode === "named"}
            disabled={disabled}
            onChange={chooseNamed}
            style={{ marginTop: 3 }}
          />
          <span>
            <span style={{ display: "block", fontSize: 13, color: "var(--m-text)" }}>
              Only these roles
            </span>
            <span style={{ display: "block", fontSize: 12, color: "var(--m-text-muted)" }}>
              Outside this list the connection does not exist for that person.
            </span>
          </span>
        </label>
      </div>

      {mode === "named" ? (
        <div
          style={{
            display: "grid",
            gap: 8,
            paddingLeft: 24,
          }}
          data-testid="audience-roles"
        >
          {rolesError !== null ? (
            <div
              role="alert"
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "8px 10px",
                borderRadius: "var(--m-radius-control)",
                background: "var(--m-status-danger-bg)",
                color: "var(--m-status-danger-fg)",
                fontSize: 12,
              }}
              data-testid="audience-roles-error"
            >
              <span>{rolesError}</span>
              <Button size="sm" variant="secondary" onClick={() => void loadRoles()}>
                Retry
              </Button>
            </div>
          ) : roles === null ? (
            <RoleSkeleton />
          ) : roles.length === 0 ? (
            <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-secondary)" }}>
              This org has no roles to name yet, so all staff with access is the only audience
              available.
            </p>
          ) : (
            roles.map((role) => {
              const checked = value.includes(role.id);
              return (
                <label
                  key={role.id}
                  style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}
                >
                  <Checkbox
                    checked={checked}
                    disabled={disabled}
                    aria-label={role.name}
                    onCheckedChange={(next) => {
                      onChange(toggleRole(value, role.id, next === true));
                    }}
                  />
                  <span style={{ fontSize: 13 }}>{role.name}</span>
                </label>
              );
            })
          )}
          {mode === "named" && value.length === 0 ? (
            <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
              Pick at least one role, or choose all staff with access.
            </p>
          ) : null}
        </div>
      ) : null}

      <p id={hintId} style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
        {audienceSummary(value, roles ?? [])}. Owners and admins always keep access so they can
        repair a connection.
      </p>
      {error !== undefined ? (
        <FieldError id={errorId}>{error}</FieldError>
      ) : null}
    </div>
  );
}
