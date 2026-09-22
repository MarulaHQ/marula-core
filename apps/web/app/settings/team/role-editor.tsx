// The custom-role matrix editor (Wave 22): name + module×action grant
// toggles for CUSTOM rows through PUT /api/v1/roles/{id} — builtins are
// typed refusals ("fork instead"), so the editor only lists custom rows.
"use client";

import { Button, Input } from "@meridian/ui";
import { useState } from "react";

export type EditableRole = {
  id: string;
  name: string;
  permissions: string[];
};

/** The org permission matrix the backend exposes — mirrors
 * core_domain::policy Module × Action (the same "module:action" shape
 * presets use today). Additive modules land here automatically — the
 * editor never hardcodes beyond the matrix and the special scopes the
 * roles surface itself already accepts. */
export const ROLE_MODULES = [
  "tickets",
  "clients",
  "time",
  "scheduling",
  "projects",
  "commercial",
  "invoicing",
  "kb",
  "documents",
  "passwords",
  "reporting",
] as const;

export const ROLE_ACTIONS = ["view", "create", "edit", "delete"] as const;

export const ROLE_SPECIALS = [
  "special:manage_roles",
  "special:manage_users",
  "special:manage_boards",
  "settings:edit",
  "settings:view",
] as const;

export function RoleMatrixEditor({
  role,
  busy,
  busyError,
  onCancel,
  onSave,
  onChange,
}: {
  role: EditableRole;
  busy: boolean;
  busyError: string | null;
  onCancel: () => void;
  onSave: (next: EditableRole) => void;
  onChange: (next: EditableRole) => void;
}) {
  const [nameTouched, setNameTouched] = useState(false);
  const toggle = (permit: string, on: boolean) => {
    const next = new Set(role.permissions);
    if (on) next.add(permit);
    else next.delete(permit);
    onChange({ ...role, permissions: [...next].sort() });
  };

  return (
    <div className="grid gap-3 rounded-lg border p-3 overflow-x-auto" style={{ borderColor: "var(--m-border)" }} data-testid={`role-editor-${role.id}`}>
      <div className="grid gap-1">
        <label htmlFor={`role-name-${role.id}`} className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Role name
        </label>
        <Input
          id={`role-name-${role.id}`}
          value={role.name}
          onChange={(e) => onChange({ ...role, name: e.target.value })}
          maxLength={64}
          disabled={busy}
          aria-label="Role name"
          data-testid={`role-name-input-${role.id}`}
          onBlur={() => setNameTouched(true)}
          required={nameTouched}
        />
      </div>
      <table className="w-full text-[12px]" style={{ borderCollapse: "collapse" }}>
        <thead>
          <tr>
            <th className="text-left p-1" style={{ color: "var(--m-text-secondary)" }}>
              Module
            </th>
            {ROLE_ACTIONS.map((a) => (
              <th key={a} className="p-1 text-center" style={{ color: "var(--m-text-secondary)" }}>
                {a}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {ROLE_MODULES.map((m) => (
            <tr key={m}>
              <td className="p-1 font-medium">{m}</td>
              {ROLE_ACTIONS.map((a) => {
                const permit = `${m}:${a}`;
                const on = role.permissions.includes(permit);
                return (
                  <td key={`${m}:${a}`} className="p-1 text-center">
                    <input
                      type="checkbox"
                      checked={on}
                      disabled={busy}
                      onChange={(e) => toggle(permit, e.target.checked)}
                      aria-label={`${m} ${a}`}
                      data-testid={`perm-${m}-${a}-${role.id}`}
                      style={{ accentColor: "var(--m-accent)" }}
                    />
                  </td>
                );
              })}
            </tr>
          ))}
          {ROLE_SPECIALS.map((s) => {
            const on = role.permissions.includes(s);
            return (
              <tr key={s}>
                <td className="p-1 font-medium">{s}</td>
                <td
                  colSpan={ROLE_ACTIONS.length}
                  className="p-1 text-center"
                  style={{ color: "var(--m-text-secondary)" }}
                >
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={busy}
                    data-testid={`perm-special-${s.replace("special:", "")}-${role.id}`}
                    onChange={(e) => toggle(s, e.target.checked)}
                    aria-label={`Grant ${s}`}
                    style={{ accentColor: "var(--m-accent)" }}
                  />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {busyError !== null ? (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {busyError}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy} data-testid={`role-cancel-${role.id}`}>
          Cancel
        </Button>
        <Button
          size="sm"
          variant="primary"
          disabled={busy || role.name.trim() === "" || role.permissions.length === 0}
          onClick={() => onSave(role)}
          data-testid={`role-save-${role.id}`}
        >
          Save role
        </Button>
      </div>
    </div>
  );
}
