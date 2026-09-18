// /settings/portal — Portal Admin: portal-user directory & board exposure config (Wave 11 lane 8).
//
// Purpose:
//  - Manage portal users: provision with initial credentials, update role/status, disable/enable, reset password.
//  - Board exposure configuration: per-client board visibility toggle and allowed status list.
//
// Instant-first: inline validation, typed empty states, Workbench tokens, light+dark, <=120ms transitions.
"use client";

import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";

type Client = components["schemas"]["Client"];
type BoardNavEntry = components["schemas"]["BoardNavEntry"];
type BoardExposureView = components["schemas"]["BoardExposureView"];
type PortalUser = components["schemas"]["PortalUser"];
type PortalRole = components["schemas"]["PortalRole"];
type PortalUserStatus = components["schemas"]["PortalUserStatus"];

const PORTAL_ROLES: PortalRole[] = ["contact", "approver", "it_manager"];

export default function PortalAdminSettingsPage() {
  const auth = useRequireAuth();

  const [clients, setClients] = useState<Client[]>([]);
  const [selectedClientId, setSelectedClientId] = useState<string>("");
  const [boards, setBoards] = useState<BoardNavEntry[]>([]);

  // Portal users for selected client
  const [portalUsers, setPortalUsers] = useState<PortalUser[] | null>(null);
  const [loadingUsers, setLoadingUsers] = useState(false);
  const [usersError, setUsersError] = useState<string | null>(null);

  // Board exposures for selected client
  const [exposures, setExposures] = useState<BoardExposureView[] | null>(null);
  const [loadingExposures, setLoadingExposures] = useState(false);
  const [exposuresError, setExposuresError] = useState<string | null>(null);

  // Provision user modal
  const [createOpen, setCreateOpen] = useState(false);
  const [newEmail, setNewEmail] = useState("");
  const [newName, setNewName] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newRole, setNewRole] = useState<PortalRole>("contact");
  const [creatingUser, setCreatingUser] = useState(false);
  const [createUserError, setCreateUserError] = useState<string | null>(null);

  // Password reset modal
  const [resetModalOpen, setResetModalOpen] = useState(false);
  const [targetUserId, setTargetUserId] = useState<string | null>(null);
  const [resetPassword, setResetPassword] = useState("");
  const [resetting, setResetting] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);

  const refreshClientsAndBoards = useCallback(async () => {
    const [clientsRes, boardsRes] = await Promise.all([
      api.api.GET("/api/v1/clients"),
      api.api.GET("/api/v1/boards/nav"),
    ]);

    if (clientsRes.data) {
      setClients(clientsRes.data);
      if (!selectedClientId && clientsRes.data.length > 0) {
        setSelectedClientId(clientsRes.data[0]!.id);
      }
    }
    if (boardsRes.data) {
      setBoards(boardsRes.data);
    }
  }, [selectedClientId]);

  const refreshClientData = useCallback(async (clientId: string) => {
    if (!clientId) return;

    setLoadingUsers(true);
    setUsersError(null);
    const usersRes = await api.api.GET("/api/v1/clients/{id}/portal-users", {
      params: { path: { id: clientId } },
    });
    setLoadingUsers(false);
    if (usersRes.data) {
      setPortalUsers(usersRes.data);
    } else {
      setUsersError("Failed to load portal users for this client.");
    }

    setLoadingExposures(true);
    setExposuresError(null);
    const exposuresRes = await api.api.GET("/api/v1/clients/{id}/board-exposures", {
      params: { path: { id: clientId } },
    });
    setLoadingExposures(false);
    if (exposuresRes.data) {
      setExposures(exposuresRes.data);
    } else {
      setExposuresError("Failed to load board exposure settings.");
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") {
      void refreshClientsAndBoards();
    }
  }, [auth.status, refreshClientsAndBoards]);

  useEffect(() => {
    if (auth.status === "authed" && selectedClientId) {
      void refreshClientData(selectedClientId);
    }
  }, [auth.status, selectedClientId, refreshClientData]);

  const handleCreateUser = async () => {
    if (!newEmail.trim() || !newName.trim() || !newPassword.trim()) return;
    setCreatingUser(true);
    setCreateUserError(null);

    const { error } = await api.api.POST("/api/v1/clients/{id}/portal-users", {
      params: { path: { id: selectedClientId } },
      body: {
        email: newEmail.trim(),
        display_name: newName.trim(),
        password: newPassword,
        role: newRole,
      },
    });

    setCreatingUser(false);
    if (error) {
      setCreateUserError("Failed to provision portal user. Ensure password has at least 10 characters.");
      return;
    }

    setCreateOpen(false);
    setNewEmail("");
    setNewName("");
    setNewPassword("");
    void refreshClientData(selectedClientId);
  };

  const handleToggleStatus = async (user: PortalUser) => {
    const nextStatus: PortalUserStatus = user.status === "disabled" ? "active" : "disabled";
    await api.api.PATCH("/api/v1/portal-users/{id}", {
      params: { path: { id: user.id } },
      body: {
        status: nextStatus,
      },
    });
    void refreshClientData(selectedClientId);
  };

  const handleRoleChange = async (user: PortalUser, role: PortalRole) => {
    await api.api.PATCH("/api/v1/portal-users/{id}", {
      params: { path: { id: user.id } },
      body: {
        role,
      },
    });
    void refreshClientData(selectedClientId);
  };

  const handleResetPassword = async () => {
    if (!targetUserId || !resetPassword.trim()) return;
    setResetting(true);
    setResetError(null);

    const { error } = await api.api.PATCH("/api/v1/portal-users/{id}", {
      params: { path: { id: targetUserId } },
      body: {
        password: resetPassword,
      },
    });

    setResetting(false);
    if (error) {
      setResetError("Failed to reset password. Minimum 10 characters required.");
      return;
    }

    setResetModalOpen(false);
    setTargetUserId(null);
    setResetPassword("");
  };

  const handleDeleteUser = async (id: string) => {
    await api.api.DELETE("/api/v1/portal-users/{id}", {
      params: { path: { id } },
    });
    void refreshClientData(selectedClientId);
  };

  const handleToggleExposure = async (boardId: string, currentlyExposed: boolean) => {
    await api.api.PUT("/api/v1/clients/{id}/board-exposures", {
      params: { path: { id: selectedClientId } },
      body: {
        board_id: boardId,
        exposed: !currentlyExposed,
        visible_status_ids: [],
      },
    });
    void refreshClientData(selectedClientId);
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-5xl mx-auto p-6">
      <SettingsHeader
        title="Portal"
        description="Manage client portal user access credentials and configure per-client board visibility exposures."
      />

      {/* Client Picker */}
      <Card className="p-4">
        <div className="flex flex-wrap items-center gap-4">
          <div className="w-72">
            <Label htmlFor="portal-client-select">Select Client</Label>
            <Select value={selectedClientId} onValueChange={setSelectedClientId}>
              <SelectTrigger id="portal-client-select" className="w-full">
                <SelectValue placeholder="Pick a client..." />
              </SelectTrigger>
              <SelectContent>
                {clients.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <p className="text-[12px] pt-4" style={{ color: "var(--m-text-muted)" }}>
            Portal settings and users are isolated per client organization.
          </p>
        </div>
      </Card>

      {/* Board Exposure Configuration */}
      <Card className="p-4 grid gap-3">
        <div>
          <h2 className="text-sm font-semibold">Board Exposure Controls</h2>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Select which ticket boards are exposed in this client’s portal view.
          </p>
        </div>

        {exposuresError && (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {exposuresError}
          </p>
        )}

        {loadingExposures ? (
          <div className="p-4 flex justify-center">
            <Spinner />
          </div>
        ) : (
          <div className="grid gap-2">
            {boards.map((b) => {
              const exp = exposures?.find((e) => e.board_id === b.id);
              const isExposed = exp ? exp.exposed : false;

              return (
                <div
                  key={b.id}
                  className="flex items-center justify-between p-3 rounded border"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <div>
                    <div className="font-medium text-[13px]">{b.name}</div>
                    <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      Board ID: {b.id}
                    </div>
                  </div>

                  <label className="flex items-center gap-2 cursor-pointer text-[13px]">
                    <Checkbox
                      checked={isExposed}
                      onCheckedChange={() => void handleToggleExposure(b.id, isExposed)}
                    />
                    <span>{isExposed ? "Exposed to client" : "Hidden"}</span>
                  </label>
                </div>
              );
            })}
          </div>
        )}
      </Card>

      {/* Portal Users Directory */}
      <Card className="p-4 grid gap-3">
        <div className="flex justify-between items-center">
          <div>
            <h2 className="text-sm font-semibold">Portal Users Directory</h2>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Contacts provisioned with portal login credentials for this client.
            </p>
          </div>

          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogTrigger asChild>
              <Button disabled={!selectedClientId}>Provision portal user</Button>
            </DialogTrigger>
            <DialogContent title="Provision Client Portal User">
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="pu-email">Contact Email</Label>
                  <Input
                    id="pu-email"
                    type="email"
                    placeholder="contact@client.example"
                    value={newEmail}
                    onChange={(e) => setNewEmail(e.target.value)}
                    required
                  />
                </div>

                <div>
                  <Label htmlFor="pu-name">Display Name</Label>
                  <Input
                    id="pu-name"
                    placeholder="Full name"
                    value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    required
                  />
                </div>

                <div>
                  <Label htmlFor="pu-pass">Initial Password</Label>
                  <Input
                    id="pu-pass"
                    type="password"
                    placeholder="Min 10 chars"
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    required
                  />
                  <FieldHint>Hashed with argon2id. Hand off to the contact out-of-band.</FieldHint>
                </div>

                <div>
                  <Label htmlFor="pu-role">Portal Role</Label>
                  <Select value={newRole} onValueChange={(v) => setNewRole(v as PortalRole)}>
                    <SelectTrigger id="pu-role" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PORTAL_ROLES.map((r) => (
                        <SelectItem key={r} value={r}>
                          {r}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {createUserError && (
                  <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {createUserError}
                  </p>
                )}

                <DialogActions>
                  <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void handleCreateUser()} disabled={creatingUser || !newEmail.trim()}>
                    {creatingUser ? <Spinner /> : null} Provision
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        </div>

        {usersError && (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {usersError}
          </p>
        )}

        <div className="overflow-x-auto">
          {loadingUsers ? (
            <div className="p-6 flex justify-center">
              <Spinner />
            </div>
          ) : portalUsers === null || portalUsers.length === 0 ? (
            <div className="p-6">
              <EmptyState message="No portal users provisioned for this client yet." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Email</Th>
                  <Th>Display Name</Th>
                  <Th>Role</Th>
                  <Th>Status</Th>
                  <Th className="text-right">Actions</Th>
                </tr>
              </thead>
              <tbody>
                {portalUsers.map((u) => (
                  <Tr key={u.id}>
                    <Td className="font-medium text-[13px]">{u.email}</Td>
                    <Td className="text-[13px]">{u.display_name}</Td>
                    <Td>
                      <Select value={u.role} onValueChange={(v) => void handleRoleChange(u, v as PortalRole)}>
                        <SelectTrigger className="w-32 h-7 text-[12px]">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {PORTAL_ROLES.map((r) => (
                            <SelectItem key={r} value={r}>
                              {r}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </Td>
                    <Td>
                      <Tag tone={u.status === "active" || u.status === "invited" ? "success" : "danger"}>
                        {u.status}
                      </Tag>
                    </Td>
                    <Td className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void handleToggleStatus(u)}
                        >
                          {u.status === "disabled" ? "Enable" : "Disable"}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => {
                            setTargetUserId(u.id);
                            setResetPassword("");
                            setResetError(null);
                            setResetModalOpen(true);
                          }}
                        >
                          Reset PW
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void handleDeleteUser(u.id)}
                        >
                          Delete
                        </Button>
                      </div>
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
        </div>
      </Card>

      {/* Reset Password Modal */}
      <Dialog open={resetModalOpen} onOpenChange={setResetModalOpen}>
        <DialogContent title="Reset Portal User Password">
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="rp-pass">New Password (min 10 characters)</Label>
              <Input
                id="rp-pass"
                type="password"
                placeholder="New password"
                value={resetPassword}
                onChange={(e) => setResetPassword(e.target.value)}
              />
            </div>

            {resetError && (
              <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {resetError}
              </p>
            )}

            <DialogActions>
              <Button variant="ghost" onClick={() => setResetModalOpen(false)}>
                Cancel
              </Button>
              <Button onClick={() => void handleResetPassword()} disabled={resetting || resetPassword.length < 10}>
                {resetting ? <Spinner /> : null} Update Password
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
