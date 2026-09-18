// /settings/team — the org user directory (Wave 11 lane 10 close-out of
// the lane 2 backend, migration 0047) + the preset-role roster (Wave 13
// lane 6, RB1 — ADR 0004).
//
// Purpose: staff directory (paged GET /org/users), the invite flow (POST
// /org/users returns the one-time `invite.setup_token` — copy/link dialog,
// NO read-back), org-role assignment (builtin roles via the role picker),
// and activate/deactivate with the honest confirmations: deactivation
// signs the user out immediately, self-deactivation is a typed 422, and
// touching the last active owner is a typed 409 — all rendered inline.
// Reads are manager+ (manage_users OR manage_boards); mutations are
// owner/admin (manage_users) — mutation controls render only for that
// role and a 403 on read renders a typed not-available state, never a
// dead button.
// Preset roles render through THIS surface (no parallel roles page): the
// role pickers list the full eight-preset roster, and the Preset roles
// card shows the canonical matrix, fork origins, and fork/restore actions
// for manage_roles holders riding the instant-first envelope.
// Non-purpose: no custom-role matrix editor (Phase 5) and no auth/session admin.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { ConfirmDestructive, SaveBar, SettingsHeader } from "../../../components/settings/settings-shell";
import { OFFLINE_RETRY_COPY, reasonCopy } from "../../../lib/error-copy";
import { runEnvelopeMutation } from "../../../lib/envelope-write";

type OrgUser = components["schemas"]["OrgUser"];
type OrgUserListResponse = components["schemas"]["OrgUserListResponse"];
type InviteInfo = components["schemas"]["InviteInfo"];
type RolesListResponse = components["schemas"]["RolesListResponse"];
type RoleView = components["schemas"]["RoleView"];
type PresetView = components["schemas"]["PresetView"];
type PortalTierView = components["schemas"]["PortalTierView"];

const PAGE_SIZE = 50;

const BUILTIN_ROLES = [
  "owner",
  "admin",
  "dispatcher",
  "senior_tech",
  "technician",
  "bookkeeper",
  "auditor",
  "read_only",
] as const;
type BuiltinRole = (typeof BUILTIN_ROLES)[number];

const MANAGE_USERS = "special:manage_users";
const MANAGE_BOARDS = "special:manage_boards";

const ROLE_HINT: Record<BuiltinRole, string> = {
  owner: "Full control incl. billing and the org itself",
  admin: "Everything except org-level ownership actions",
  dispatcher: "Schedules and assigns work",
  senior_tech: "Escalation tier: full ticket + KB control, time approvals, no financial visibility",
  technician: "Works tickets",
  bookkeeper: "Invoicing control and financial visibility, no ticket work",
  auditor: "Read everything plus the audit trail; writes nothing",
  read_only: "View only",
};

function roleTone(role: string | null | undefined): "success" | "warning" | "neutral" {
  if (role === "owner") return "success";
  if (role === "admin") return "warning";
  return "neutral";
}

/** Friendly copy for the typed self-deactivation (422) and
 * last-active-owner (409) rejections; other typed reasons fall through to
 * the central reason-code map (lib/error-copy) instead of raw messages. */
function friendlyUserError(reason: string | undefined, message: string | undefined): string {
  const m = (message ?? "").toLowerCase();
  if (m.includes("cannot deactivate your own account")) {
    return "You cannot deactivate your own account — ask another owner or admin.";
  }
  if (m.includes("last active owner")) {
    return "That would leave the org without an active owner. Promote another owner first.";
  }
  if (m.includes("already in org")) {
    return "That email is already in this org's directory.";
  }
  if (reason === "forbidden") {
    return "Only owners and admins can manage users.";
  }
  if (reason !== undefined) {
    return reasonCopy(reason, { message });
  }
  return message ?? "The change was rejected.";
}

export default function TeamSettingsPage() {
  const auth = useRequireAuth();
  const me = auth.me;

  const canRead = (me?.permissions ?? []).includes(MANAGE_USERS) || (me?.permissions ?? []).includes(MANAGE_BOARDS);
  const canManage = (me?.permissions ?? []).includes(MANAGE_USERS) ?? false;

  const [users, setUsers] = useState<OrgUser[] | null>(null);
  const [total, setTotal] = useState<number | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [forbidden, setForbidden] = useState(false);

  // Invite dialog state
  const [inviteOpen, setInviteOpen] = useState(false);
  const [inviteName, setInviteName] = useState("");
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState<BuiltinRole>("technician");
  const [inviteBusy, setInviteBusy] = useState(false);
  const [inviteError, setInviteError] = useState<string | null>(null);
  const [inviteResult, setInviteResult] = useState<{ user: OrgUser; invite: InviteInfo } | null>(null);
  // Per-row action state
  const [busyUserId, setBusyUserId] = useState<string | null>(null);
  const [rowError, setRowError] = useState<{ userId: string; text: string } | null>(null);
  // Deactivate confirmation (shared destructive pattern — never window.confirm)
  const [pendingDeactivate, setPendingDeactivate] = useState<OrgUser | null>(null);

  // Preset-role roster (RB1): rows + canonical presets + portal tiers.
  const [rolesData, setRolesData] = useState<RolesListResponse | null>(null);
  const [rolesError, setRolesError] = useState<string | null>(null);
  const [busyRoleId, setBusyRoleId] = useState<string | null>(null);
  const [roleActionError, setRoleActionError] = useState<string | null>(null);

  const fetchPage = useCallback(
    async (offset: number, replace: boolean) => {
      setLoadError(null);
      const { data, error, response } = await api.api.GET("/api/v1/org/users", {
        params: { query: { limit: PAGE_SIZE, offset } },
      });
      if (error !== undefined || data === undefined) {
        if (response.status === 403) {
          setForbidden(true);
          return;
        }
        setLoadError("Failed to load the directory.");
        return;
      }
      setForbidden(false);
      const page = data as OrgUserListResponse;
      setTotal(page.total);
      setUsers((prev) =>
        replace || prev === null ? page.users : [...prev, ...page.users.filter((u) => !prev.some((p) => p.id === u.id))],
      );
    },
    [],
  );

  useEffect(() => {
    if (auth.status === "authed" && canRead) void fetchPage(0, true);
  }, [auth.status, canRead, fetchPage]);

  const MANAGE_ROLES = "special:manage_roles";
  const canManageRoles = (me?.permissions ?? []).includes(MANAGE_ROLES) ?? false;

  const fetchRoles = useCallback(async () => {
    setRolesError(null);
    const { data, error, response } = await api.api.GET("/api/v1/roles");
    if (error !== undefined || data === undefined) {
      if (response.status === 403) {
        // Readers without roles-admin still see the directory; the roster
        // card is manager+ via manage_users too — a 403 here means the
        // caller holds neither grant, so leave the card absent.
        setRolesData(null);
        return;
      }
      setRolesError("Failed to load the role roster.");
      return;
    }
    setRolesData(data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed" && canRead) void fetchRoles();
  }, [auth.status, canRead, fetchRoles]);

  /** Fork a preset into a custom copy on the same matrix (RB1). */
  const forkRole = async (preset: PresetView, roleId: string) => {
    const name = window.prompt(`Duplicate "${preset.name}" as a custom role`, `${preset.name} (custom)`);
    if (name === null || name.trim() === "") return;
    setBusyRoleId(roleId);
    setRoleActionError(null);
    const result = await runEnvelopeMutation({
      describe: "Fork role",
      entityId: roleId,
      optimistic: () => setRolesData((prev) => prev),
      revert: () => setRolesData((prev) => prev),
      buildBody: (base) => ({ ...base, name: name.trim() }),
      request: (client, body) =>
        client.POST("/api/v1/roles/{id}/fork", { params: { path: { id: roleId } }, body }),
      versions: (response) => [[roleId, response.mutation.version]],
    });
    setBusyRoleId(null);
    if (!result.ok) {
      setRoleActionError(
        result.reason === "role_name_taken"
          ? `A role named "${name.trim()}" already exists.`
          : result.message ?? "The fork was rejected.",
      );
      return;
    }
    void fetchRoles();
  };

  /** Restore a drifted preset row to the canonical shipped matrix (RB1). */
  const restoreRole = async (role: RoleView) => {
    setBusyRoleId(role.id);
    setRoleActionError(null);
    const result = await runEnvelopeMutation({
      describe: "Restore role",
      entityId: role.id,
      optimistic: () => setRolesData((prev) => prev),
      revert: () => setRolesData((prev) => prev),
      buildBody: (base) => ({ ...base }),
      request: (client, body) =>
        client.POST("/api/v1/roles/{id}/restore", { params: { path: { id: role.id } }, body }),
      versions: (response) => [[role.id, response.mutation.version]],
    });
    setBusyRoleId(null);
    if (!result.ok) {
      setRoleActionError(result.message ?? "The restore was rejected.");
      return;
    }
    void fetchRoles();
  };

  /** Direct invite call that KEEPS the response body — the one-time setup
   * token exists ONLY in the POST response (no read-back), so this cannot
   * go through runEnvelopeMutation's reconcile-and-discard flow. */
  const inviteDirect = async () => {
    setInviteBusy(true);
    setInviteError(null);
    try {
      const { data, error } = await api.api.POST("/api/v1/org/users", {
        body: {
          display_name: inviteName.trim(),
          email: inviteEmail.trim(),
          role: inviteRole,
        },
      });
      if (error !== undefined || data === undefined) {
        const err = error as { error?: { reason?: string; message?: string } } | undefined;
        setInviteError(friendlyUserError(err?.error?.reason, err?.error?.message));
        return;
      }
      // `invite` is optional in the type but always present on a fresh
      // invite; if the server ever omits it, fail honestly rather than
      // show a dead link.
      if (data.invite == null) {
        setInviteError("The invite was created but no setup token came back — try again.");
        return;
      }
      setInviteResult({ user: data, invite: data.invite });
      setInviteName("");
      setInviteEmail("");
      setInviteRole("technician");
      void fetchPage(0, true);
    } catch {
      setInviteError(OFFLINE_RETRY_COPY);
    } finally {
      setInviteBusy(false);
    }
  };

  const setStatus = async (user: OrgUser, active: boolean) => {
    setBusyUserId(user.id);
    setRowError(null);
    const before = users ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["SetUserStatusRequest"],
      components["schemas"]["OrgUserMutationResponse"]
    >({
      describe: active ? "Activate user" : "Deactivate user",
      entityId: user.id,
      optimistic: () =>
        setUsers((prev) => (prev ?? []).map((u) => (u.id === user.id ? { ...u, is_active: active } : u))),
      revert: () => setUsers(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, active }),
      request: (client, body) =>
        client.PATCH("/api/v1/org/users/{id}/status", { params: { path: { id: user.id } }, body }),
      versions: (response) => [[user.id, response.mutation.version]],
    });
    setBusyUserId(null);
    if (!result.ok) {
      setRowError({ userId: user.id, text: friendlyUserError(result.reason, result.message) });
    }
  };

  const setRole = async (user: OrgUser, role: string) => {
    setBusyUserId(user.id);
    setRowError(null);
    const before = users ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["SetUserRoleRequest"],
      components["schemas"]["OrgUserMutationResponse"]
    >({
      describe: "Set org role",
      entityId: user.id,
      optimistic: () =>
        setUsers((prev) => (prev ?? []).map((u) => (u.id === user.id ? { ...u, org_role: role } : u))),
      revert: () => setUsers(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, role }),
      request: (client, body) =>
        client.PATCH("/api/v1/org/users/{id}/role", { params: { path: { id: user.id } }, body }),
      versions: (response) => [[user.id, response.mutation.version]],
    });
    setBusyUserId(null);
    if (!result.ok) {
      setRowError({ userId: user.id, text: friendlyUserError(result.reason, result.message) });
    }
  };

  if (auth.status !== "authed") return null;

  if (!canRead) {
    return (
      <div className="grid gap-4">
        <h1 className="text-lg font-semibold">Team</h1>
        <Card>
          <EmptyState message="The staff directory is available to managers (dispatchers, admins, owners)." />
        </Card>
      </div>
    );
  }

  return (
    <div className="grid gap-4">
      <div className="flex items-start justify-between gap-4">
        <SettingsHeader
          title="Team"
          description={total !== null ? `${total} member${total === 1 ? "" : "s"} in this org.` : "Staff directory for this org."}
        />
        {canManage ? (
          <Dialog
            open={inviteOpen}
            onOpenChange={(o) => {
              setInviteOpen(o);
              setInviteError(null);
              if (!o) setInviteResult(null);
            }}
          >
            <DialogTrigger asChild>
              <Button>Invite member</Button>
            </DialogTrigger>
            <DialogContent
              title={inviteResult ? "Invite created — deliver the link now" : "Invite a member"}
              description={
                inviteResult
                  ? "This setup link is shown ONCE and cannot be fetched again. Copy it and hand it to the member out-of-band."
                  : "The member receives a one-time setup link; they pick a password on the accept page and land logged in."
              }
            >
              {inviteResult ? (
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="invite-link">Accept-invite link</Label>
                    <Input
                      id="invite-link"
                      readOnly
                      value={`${typeof window !== "undefined" ? window.location.origin : ""}/accept-invite?token=${inviteResult.invite.setup_token}`}
                      onFocus={(e) => e.currentTarget.select()}
                    />
                  </div>
                  <div>
                    <Label htmlFor="invite-token">Setup token (equivalent)</Label>
                    <Input
                      id="invite-token"
                      readOnly
                      value={inviteResult.invite.setup_token}
                      onFocus={(e) => e.currentTarget.select()}
                    />
                  </div>
                  {inviteResult.invite.expires_at !== null && inviteResult.invite.expires_at !== undefined ? (
                    <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      Expires {new Date(inviteResult.invite.expires_at).toLocaleString()}.
                    </p>
                  ) : null}
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    If the link expires or is used, invite the member again with a fresh one.
                  </p>
                  <DialogActions>
                    <Button
                      onClick={() => {
                        void navigator.clipboard.writeText(
                          `${window.location.origin}/accept-invite?token=${inviteResult.invite.setup_token}`,
                        );
                      }}
                      variant="secondary"
                    >
                      Copy link
                    </Button>
                    <Button onClick={() => setInviteOpen(false)}>Done</Button>
                  </DialogActions>
                </div>
              ) : (
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="invite-name">Display name</Label>
                    <Input id="invite-name" value={inviteName} onChange={(e) => setInviteName(e.target.value)} />
                  </div>
                  <div>
                    <Label htmlFor="invite-email">Email</Label>
                    <Input
                      id="invite-email"
                      type="email"
                      value={inviteEmail}
                      onChange={(e) => setInviteEmail(e.target.value)}
                    />
                  </div>
                  <div>
                    <Label htmlFor="invite-role">Org role</Label>
                    <Select value={inviteRole} onValueChange={(v) => setInviteRole(v as BuiltinRole)}>
                      <SelectTrigger id="invite-role" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {BUILTIN_ROLES.map((r) => (
                          <SelectItem key={r} value={r}>
                            {r} — {ROLE_HINT[r]}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <DialogActions>
                    <Button variant="ghost" onClick={() => setInviteOpen(false)}>
                      Cancel
                    </Button>
                    <SaveBar
                      dirty={inviteName.trim() !== "" && inviteEmail.trim() !== ""}
                      saving={inviteBusy}
                      error={inviteError}
                      onSave={() => void inviteDirect()}
                      saveTestId="create-invite"
                      saveLabel="Create invite"
                    />
                  </DialogActions>
                </div>
              )}
            </DialogContent>
          </Dialog>
        ) : null}
      </div>

      {forbidden ? (
        <Card>
          <EmptyState message="Your role cannot read the directory." />
        </Card>
      ) : loadError !== null ? (
        <p role="alert" className="text-[13px]">
          {loadError}
        </p>
      ) : users === null ? (
        <Card>
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        </Card>
      ) : users.length === 0 ? (
        <Card>
          <EmptyState message="No members yet — invite your first teammate." />
        </Card>
      ) : (
        <Card className="overflow-x-auto">
          <Table>
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Email</Th>
                <Th>Org role</Th>
                <Th>Status</Th>
                {canManage ? <Th className="text-right">Actions</Th> : null}
              </tr>
            </thead>
            <tbody>
              {users.map((user) => {
                const isSelf = me?.user_id === user.id;
                return (
                  <Tr key={user.id}>
                    <Td className="text-[13px] font-medium">
                      {user.display_name}
                      {isSelf ? <span className="ml-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>(you)</span> : null}
                    </Td>
                    <Td className="text-[13px]">{user.email}</Td>
                    <Td>
                      {canManage ? (
                        <Select
                          value={user.org_role ?? ""}
                          onValueChange={(v) => void setRole(user, v)}
                          disabled={busyUserId === user.id}
                        >
                          <SelectTrigger className="w-44" aria-label={`Org role for ${user.display_name}`}>
                            <SelectValue placeholder="Custom role" />
                          </SelectTrigger>
                          <SelectContent>
                            {BUILTIN_ROLES.map((r) => (
                              <SelectItem key={r} value={r}>
                                {r}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      ) : user.org_role === null || user.org_role === undefined ? (
                        <Tag tone="outline">custom</Tag>
                      ) : (
                        <Tag tone={roleTone(user.org_role)}>{user.org_role}</Tag>
                      )}
                    </Td>
                    <Td>
                      {user.is_active ? (
                        <StatusTag tone="success">Active</StatusTag>
                      ) : (
                        <span className="inline-flex items-center gap-1">
                          <StatusTag tone="neutral">Inactive</StatusTag>
                          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                            invited or deactivated
                          </span>
                        </span>
                      )}
                    </Td>
                    {canManage ? (
                      <Td className="text-right">
                        <div className="flex justify-end gap-2">
                          {user.is_active ? (
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={busyUserId === user.id || isSelf}
                              title={isSelf ? "You cannot deactivate your own account" : undefined}
                              onClick={() => setPendingDeactivate(user)}
                            >
                              Deactivate
                            </Button>
                          ) : (
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={busyUserId === user.id}
                              onClick={() => void setStatus(user, true)}
                            >
                              Activate
                            </Button>
                          )}
                        </div>
                      </Td>
                    ) : null}
                  </Tr>
                );
              })}
            </tbody>
          </Table>
          {rowError !== null ? (
            <p role="alert" className="px-4 pb-3 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {rowError.text}
            </p>
          ) : null}
          {total !== null && users.length < total ? (
            <div className="flex justify-center p-3">
              <Button variant="secondary" size="sm" onClick={() => void fetchPage(users.length, false)}>
                Load more ({total - users.length} remaining)
              </Button>
            </div>
          ) : null}
          {!canManage ? (
            <p className="px-4 pb-3 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Role and status changes are owner/admin actions.
            </p>
          ) : null}
        </Card>
      )}

      {rolesError !== null ? (
        <p role="alert" className="text-[13px]">
          {rolesError}
        </p>
      ) : null}

      {rolesData !== null ? (
        <Card className="overflow-x-auto">
          <div className="grid gap-3 p-4">
            <div>
              <h2 className="text-[15px] font-semibold">Preset roles</h2>
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                The shipped roles on the same permission matrix as custom roles. Fork to customize —
                presets stay canonical and restorable.
              </p>
            </div>
            {roleActionError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {roleActionError}
              </p>
            ) : null}
            <Table>
              <thead>
                <tr>
                  <Th>Preset</Th>
                  <Th>Purpose</Th>
                  <Th>Grants</Th>
                  <Th>Members</Th>
                  {canManageRoles ? <Th className="text-right">Actions</Th> : null}
                </tr>
              </thead>
              <tbody>
                {rolesData.presets.map((preset) => {
                  const row = rolesData.roles.find((r) => r.builtin === preset.builtin);
                  const drifted =
                    row !== undefined &&
                    JSON.stringify(row.permissions) !== JSON.stringify(preset.permissions);
                  return (
                    <Tr key={preset.builtin}>
                      <Td className="text-[13px] font-medium">
                        {preset.name}
                        {row?.forked_from != null ? (
                          <span
                            className="ml-1 text-[11px]"
                            style={{ color: "var(--m-text-muted)" }}
                          >
                            (fork of {row.forked_from})
                          </span>
                        ) : null}
                      </Td>
                      <Td className="text-[13px]">{preset.description}</Td>
                      <Td className="text-[13px]">
                        {preset.permissions === undefined ||
                        !Array.isArray(preset.permissions)
                          ? 0
                          : (preset.permissions as unknown[]).length}{" "}
                        grants
                      </Td>
                      <Td className="text-[13px]">{row?.holders ?? 0}</Td>
                      {canManageRoles ? (
                        <Td className="text-right">
                          <div className="flex justify-end gap-2">
                            {row ? (
                              <Button
                                size="sm"
                                variant="secondary"
                                disabled={busyRoleId === row.id}
                                onClick={() => void forkRole(preset, row.id)}
                              >
                                Duplicate
                              </Button>
                            ) : null}
                            {drifted && row ? (
                              <Button
                                size="sm"
                                variant="secondary"
                                disabled={busyRoleId === row.id}
                                onClick={() => void restoreRole(row)}
                              >
                                Restore
                              </Button>
                            ) : null}
                          </div>
                        </Td>
                      ) : null}
                    </Tr>
                  );
                })}
              </tbody>
            </Table>
            <div>
              <h3 className="text-[13px] font-semibold">Portal tiers</h3>
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Client-contact tiers — a separate authorization model from staff roles; assigned per
                portal user in the portal admin.
              </p>
            </div>
            <Table>
              <thead>
                <tr>
                  <Th>Tier</Th>
                  <Th>Capabilities</Th>
                </tr>
              </thead>
              <tbody>
                {rolesData.portal_tiers.map((tier: PortalTierView) => (
                  <Tr key={tier.tier}>
                    <Td className="text-[13px] font-medium">{tier.name}</Td>
                    <Td className="text-[13px]">
                      {tier.capabilities.map((c) => (
                        <Tag key={c} tone="outline">
                          {c}
                        </Tag>
                      ))}
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          </div>
        </Card>
      ) : null}

      <ConfirmDestructive
        open={pendingDeactivate !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDeactivate(null);
        }}
        title={`Deactivate ${pendingDeactivate?.display_name ?? ""}?`}
        body="Their sessions are signed out immediately and login is blocked. The account is never deleted — activation restores access."
        confirmLabel="Deactivate"
        onConfirm={() => {
          if (pendingDeactivate === null) return;
          const user = pendingDeactivate;
          setPendingDeactivate(null);
          void setStatus(user, false);
        }}
      />
    </div>
  );
}
