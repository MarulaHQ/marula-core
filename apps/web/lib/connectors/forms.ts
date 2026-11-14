// Pure helpers for the connector write forms (§3).
//
// Purpose: two decisions here are easy to get wrong in a component and are
// worth testing without rendering.
//
// 1. **The settings patch.** PATCH merges per key: an absent key is untouched,
//    `null` removes it. A form that posts the whole document cannot remove a
//    field (an absent key means "keep"), and a form that posts only changed
//    values must send `null` for a cleared one. This module computes exactly
//    that difference — and never includes a credential, which the server
//    refuses in `settings`.
// 2. **What an empty audience means.** Empty is "all staff with access", the
//    shipped default. Reading it as "no access" is the opposite claim, and the
//    copy is decided once, here.
//
// Non-purpose: no React, no fetch, no validation (that is `validateSettingsValue`
// in `@marula-core/ui`, which mirrors the server).
import type { SettingsObject } from "@marula-core/ui";

/** One org role as `/api/v1/roles` returns it, narrowed to what Access reads. */
export interface RoleOption {
  id: string;
  name: string;
}

function deepEqual(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (Array.isArray(left) && Array.isArray(right)) {
    if (left.length !== right.length) return false;
    return left.every((item, index) => deepEqual(item, right[index]));
  }
  if (
    typeof left === "object" &&
    left !== null &&
    !Array.isArray(left) &&
    typeof right === "object" &&
    right !== null &&
    !Array.isArray(right)
  ) {
    const a = left as Record<string, unknown>;
    const b = right as Record<string, unknown>;
    const keys = Object.keys(a);
    if (keys.length !== Object.keys(b).length) return false;
    return keys.every((key) => key in b && deepEqual(a[key], b[key]));
  }
  return false;
}

/**
 * The `settings` half of a PATCH.
 *
 * - a key present in both and unchanged is omitted (the stored value stays);
 * - a key present only in `next` is sent (an addition);
 * - a key present only in `original` is sent as `null` (a removal);
 * - a credential key is never sent, whatever the form did.
 */
export function settingsPatch(
  original: SettingsObject,
  next: SettingsObject,
  credentialKeys: readonly string[] = [],
): Record<string, unknown> {
  const credentials = new Set(credentialKeys);
  const patch: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(next)) {
    if (credentials.has(key)) continue;
    if (!(key in original) || !deepEqual(original[key], value)) {
      patch[key] = value;
    }
  }
  for (const key of Object.keys(original)) {
    if (credentials.has(key)) continue;
    if (!(key in next)) patch[key] = null;
  }
  return patch;
}

/**
 * The `secrets` half of a PATCH: only fields the operator typed a replacement
 * into. A blank field means "leave the stored credential alone", never "erase
 * it", because the surface cannot show the value and so cannot ask someone to
 * confirm losing it.
 */
export function secretsPatch(drafts: Readonly<Record<string, string>>): Record<string, string> {
  const patch: Record<string, string> = {};
  for (const [key, value] of Object.entries(drafts)) {
    if (typeof value === "string" && value !== "") patch[key] = value;
  }
  return patch;
}

/** Rename/re-point payload for one destination. Identity is never included. */
export function bindingPatch(input: {
  label: string;
  enabled: boolean;
  external_parent: string | null;
}): { label: string; enabled: boolean; external_parent: string | null } {
  return {
    label: input.label.trim(),
    enabled: input.enabled,
    external_parent:
      input.external_parent !== null && input.external_parent.trim() !== ""
        ? input.external_parent.trim()
        : null,
  };
}

/**
 * The Access line. Empty is the default, and it reads as a grant rather than
 * as an absence — "no access" and "all staff" must never share a sentence.
 */
export function audienceSummary(roleIds: readonly string[], roles: readonly RoleOption[]): string {
  if (roleIds.length === 0) return "All staff with access";
  const byId = new Map(roles.map((role) => [role.id, role.name]));
  const named = roleIds.map((id) => byId.get(id)).filter((name): name is string => name !== undefined);
  if (named.length === 0) {
    return roleIds.length === 1 ? "1 named role" : `${String(roleIds.length)} named roles`;
  }
  if (named.length < roleIds.length) {
    const missing = roleIds.length - named.length;
    return `${named.join(", ")} and ${String(missing)} role${missing === 1 ? "" : "s"} this page cannot name`;
  }
  return named.join(", ");
}

/** Toggle one role id, preserving the order the operator picked them in. */
export function toggleRole(roleIds: readonly string[], id: string, on: boolean): string[] {
  if (on) return roleIds.includes(id) ? [...roleIds] : [...roleIds, id];
  return roleIds.filter((candidate) => candidate !== id);
}

/** Read a role list defensively: the route answers an array of role rows. */
export function readRoles(body: unknown): RoleOption[] {
  const rows = Array.isArray(body)
    ? body
    : typeof body === "object" && body !== null && "roles" in body
      ? (body as { roles?: unknown }).roles
      : undefined;
  if (!Array.isArray(rows)) return [];
  return rows
    .filter(
      (row): row is { id: string; name: string } =>
        typeof row === "object" &&
        row !== null &&
        typeof (row as { id?: unknown }).id === "string" &&
        typeof (row as { name?: unknown }).name === "string",
    )
    .map((row) => ({ id: row.id, name: row.name }));
}
