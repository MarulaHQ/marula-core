// /settings/boards — board administration (Workspace-core UI, T3).
//
// Purpose: create boards, assign them to board groups, manage board
// members (role upsert + removal on the board×BoardRole matrix), and edit
// each board's checklist template (name, ordered required steps, close
// enforcement toggle). Gated by the org-level `special:manage_boards`
// permission — without it the surface renders read-only, mirroring the
// server's verdicts before they land.
// Non-purpose: no board rename/description editing (the API exposes no
// board-field PATCH; statuses/transitions/fields keep living on
// /settings/workflow and the board page); adding a member needs the
// caller to supply the user id — the org-user directory endpoint does not
// exist yet (recorded in TASKLIST).
"use client";

import {
  Button,
  Card,
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
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { MANAGE_BOARDS_PERMISSION } from "../../../lib/admin/board-access";

type Board = components["schemas"]["Board"];
type BoardGroup = components["schemas"]["BoardGroup"];
type BoardMemberView = components["schemas"]["BoardMemberView"];
type BoardRole = components["schemas"]["BoardRole"];
type ChecklistTemplateView = components["schemas"]["ChecklistTemplateView"];
type NavEntry = components["schemas"]["BoardNavEntry"];
type OrgUser = components["schemas"]["OrgUser"];

export default function BoardAdminPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const canAdmin = me?.permissions.includes(MANAGE_BOARDS_PERMISSION) ?? false;

  const [boards, setBoards] = useState<Board[] | null>(null);
  const [groups, setGroups] = useState<BoardGroup[]>([]);
  const [boardGroupOf, setBoardGroupOf] = useState<Record<string, string | null>>({});
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [members, setMembers] = useState<BoardMemberView[] | null>(null);
  const [template, setTemplate] = useState<ChecklistTemplateView | null>(null);
  const [busy, setBusy] = useState(false);

  const [createOpen, setCreateOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newDescription, setNewDescription] = useState("");
  const [newIsDefault, setNewIsDefault] = useState(false);
  const [creating, setCreating] = useState(false);

  const [memberUserId, setMemberUserId] = useState("");
  const [memberRole, setMemberRole] = useState<BoardRole>("member");
  const [directory, setDirectory] = useState<OrgUser[] | null>(null);

  // Org-user directory (0047): the member picker reads the same paged
  // directory the Team page uses; read access covers manage_boards.
  useEffect(() => {
    if (auth.status !== "authed" || !canAdmin) return;
    let cancelled = false;
    void api.api
      .GET("/api/v1/org/users", { params: { query: { limit: 200 } } })
      .then(({ data }) => {
        if (cancelled || data === undefined) return;
        setDirectory(data.users);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [auth.status, canAdmin]);

  const [templateName, setTemplateName] = useState("");
  const [templateItems, setTemplateItems] = useState<string[]>([]);
  const [templateRequire, setTemplateRequire] = useState(false);
  const [newItemText, setNewItemText] = useState("");

  const refresh = useCallback(async () => {
    const [b, g] = await Promise.all([
      api.api.GET("/api/v1/boards"),
      api.api.GET("/api/v1/board-groups"),
    ]);
    if (b.error !== undefined || b.data === undefined) {
      setLoadError("Failed to load boards.");
      setBoards([]);
      return;
    }
    setBoards(b.data);
    setLoadError(null);
    const nav = await api.api.GET("/api/v1/boards/nav");
    if (nav.error === undefined && nav.data !== undefined) {
      const map: Record<string, string | null> = {};
      for (const entry of nav.data as NavEntry[]) map[entry.id] = entry.group_id ?? null;
      setBoardGroupOf(map);
    }
    if (g.error === undefined && g.data !== undefined) setGroups([...g.data]);
  }, []);

  const refreshBoard = useCallback(async (id: string) => {
    const [m, t] = await Promise.all([
      api.api.GET("/api/v1/boards/{id}/members", { params: { path: { id } } }),
      api.api.GET("/api/v1/boards/{id}/checklist-template", { params: { path: { id } } }),
    ]);
    setMembers(m.data ?? []);
    const tv = t.data ?? null;
    setTemplate(tv);
    setTemplateName(tv?.template?.name ?? "");
    setTemplateItems((tv?.items ?? []).map((i) => i.text));
    setTemplateRequire(tv?.require_checklist_close ?? false);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  useEffect(() => {
    if (auth.status === "authed" && selectedId !== null) void refreshBoard(selectedId);
  }, [auth.status, selectedId, refreshBoard]);

  const createBoard = async () => {
    if (newName.trim() === "") return;
    setCreating(true);
    setActionError(null);
    const { data, error } = await api.api.POST("/api/v1/boards", {
      body: {
        name: newName.trim(),
        is_default: newIsDefault,
        description: newDescription.trim() === "" ? null : newDescription.trim(),
      },
    });
    setCreating(false);
    if (error !== undefined || data === undefined) {
      setActionError("The board was rejected. Check the fields and retry.");
      return;
    }
    setCreateOpen(false);
    setNewName("");
    setNewDescription("");
    setNewIsDefault(false);
    setSelectedId(data.id);
    void refresh();
  };

  const assignGroup = async (board: Board, groupId: string) => {
    setActionError(null);
    const { error } = await api.api.PUT("/api/v1/boards/{id}/group", {
      params: { path: { id: board.id } },
      body: { group_id: groupId === "none" ? null : groupId },
    });
    if (error !== undefined) {
      setActionError("The group change was rejected.");
      return;
    }
    void refresh();
  };

  const upsertMember = async () => {
    if (selectedId === null || memberUserId.trim() === "") return;
    setBusy(true);
    setActionError(null);
    const { error } = await api.api.PUT("/api/v1/boards/{id}/members", {
      params: { path: { id: selectedId } },
      body: { user_id: memberUserId.trim(), role: memberRole },
    });
    setBusy(false);
    if (error !== undefined) {
      setActionError("The member change was rejected — is that user id in this org?");
      return;
    }
    setMemberUserId("");
    void refreshBoard(selectedId);
  };

  const removeMember = async (userId: string) => {
    if (selectedId === null) return;
    setActionError(null);
    const { error } = await api.api.DELETE("/api/v1/boards/{id}/members/{user_id}", {
      params: { path: { id: selectedId, user_id: userId } },
    });
    if (error !== undefined) {
      setActionError("The member removal was rejected.");
      return;
    }
    void refreshBoard(selectedId);
  };

  const saveTemplate = async () => {
    if (selectedId === null || templateName.trim() === "") return;
    setBusy(true);
    setActionError(null);
    const { error } = await api.api.PUT("/api/v1/boards/{id}/checklist-template", {
      params: { path: { id: selectedId } },
      body: {
        name: templateName.trim(),
        items: templateItems.filter((t) => t.trim() !== ""),
        require_checklist_close: templateRequire,
      },
    });
    setBusy(false);
    if (error !== undefined) {
      setActionError("The checklist template was rejected.");
      return;
    }
    void refreshBoard(selectedId);
  };

  const deleteTemplate = async () => {
    if (selectedId === null) return;
    setBusy(true);
    setActionError(null);
    const { error } = await api.api.DELETE("/api/v1/boards/{id}/checklist-template", {
      params: { path: { id: selectedId } },
    });
    setBusy(false);
    if (error !== undefined) {
      setActionError("The template deletion was rejected.");
      return;
    }
    void refreshBoard(selectedId);
  };

  if (auth.status !== "authed") return null;

  const selected = boards?.find((b) => b.id === selectedId) ?? null;

  return (
    <div className="grid gap-6">
      <div className="flex items-start justify-between">
        <SettingsHeader
          title="Boards"
          description="Create boards, group them, manage members and checklist templates."
        />
        {canAdmin ? (
          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogTrigger asChild>
              <Button>New board</Button>
            </DialogTrigger>
            <DialogContent title="New board">
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="nb-name">Name</Label>
                  <Input id="nb-name" value={newName} onChange={(e) => setNewName(e.target.value)} />
                </div>
                <div>
                  <Label htmlFor="nb-desc">Description</Label>
                  <Input id="nb-desc" value={newDescription} onChange={(e) => setNewDescription(e.target.value)} placeholder="Optional" />
                </div>
                <label className="flex items-center gap-2 text-[13px]">
                  <input
                    type="checkbox"
                    aria-label="Make default board"
                    checked={newIsDefault}
                    onChange={(e) => setNewIsDefault(e.target.checked)}
                  />
                  Make this the org's default board
                </label>
                {actionError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {actionError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void createBoard()} disabled={creating || newName.trim() === ""}>
                    {creating ? <Spinner /> : null} Create board
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        ) : null}
      </div>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      ) : null}
      {actionError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      ) : null}
      {!canAdmin ? (
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          You can view board configuration but not change it (missing {MANAGE_BOARDS_PERMISSION}).
        </p>
      ) : null}

      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Boards</h2>
        {boards === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : boards.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No boards yet. Create the first board for the org." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Group</Th>
                <Th>Flags</Th>
                <Th className="text-right">Administer</Th>
              </tr>
            </thead>
            <tbody>
              {boards.map((board) => (
                <Tr key={board.id}>
                  <Td>{board.name}</Td>
                  <Td>
                    {canAdmin ? (
                      <Select
                        value={boardGroupOf[board.id] ?? "none"}
                        onValueChange={(v) => void assignGroup(board, v)}
                      >
                        <SelectTrigger aria-label={`Group for ${board.name}`} className="w-40">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="none">Ungrouped</SelectItem>
                          {groups.map((g) => (
                            <SelectItem key={g.id} value={g.id}>
                              {g.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    ) : (
                      <span className="text-[13px]">
                        {groups.find((g) => g.id === (boardGroupOf[board.id] ?? ""))?.name ??
                          "Ungrouped"}
                      </span>
                    )}
                  </Td>
                  <Td>
                    {board.is_default ? <Tag>default</Tag> : null}
                    {board.require_checklist_close ? <Tag>checklist enforced</Tag> : null}
                  </Td>
                  <Td className="text-right">
                    <Button
                      variant={selectedId === board.id ? "secondary" : "ghost"}
                      size="sm"
                      onClick={() => setSelectedId(board.id)}
                    >
                      {selectedId === board.id ? "Selected" : "Select"}
                    </Button>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      {selected !== null ? (
        <>
          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Members — {selected.name}</h2>
            <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              A board role can only narrow the org matrix, never expand it.
            </p>
            {members === null ? (
              <div className="p-4">
                <Spinner />
              </div>
            ) : members.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No member rows: everyone uses their org role passthrough on this board." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>User</Th>
                    <Th>Board role</Th>
                    {canAdmin ? <Th className="text-right">Actions</Th> : null}
                  </tr>
                </thead>
                <tbody>
                  {members.map((member) => (
                    <Tr key={member.user_id}>
                      <Td>{member.display_name}</Td>
                      <Td>
                        <Tag>{member.role}</Tag>
                      </Td>
                      {canAdmin ? (
                        <Td className="text-right">
                          <Button variant="ghost" size="sm" onClick={() => void removeMember(member.user_id)}>
                            Remove
                          </Button>
                        </Td>
                      ) : null}
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
            {canAdmin ? (
              <div className="flex items-end gap-2 p-4">
                <div className="flex-1">
                  <Label htmlFor="bm-user">Member</Label>
                  {directory === null ? (
                    <Input id="bm-user" value={memberUserId} onChange={(e) => setMemberUserId(e.target.value)} placeholder="Org user uuid" />
                  ) : (
                    <Select value={memberUserId} onValueChange={setMemberUserId}>
                      <SelectTrigger id="bm-user" className="w-full" aria-label="Directory member">
                        <SelectValue placeholder="Pick a member from the directory" />
                      </SelectTrigger>
                      <SelectContent>
                        {directory.map((user) => (
                          <SelectItem key={user.id} value={user.id}>
                            {user.display_name} — {user.email}
                            {user.is_active ? "" : " (inactive)"}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                  <FieldHint>
                    {directory === null
                      ? "Loading the org directory — the paste fallback stays while it loads."
                      : "Picked from the org directory (Settings → Team)."}
                  </FieldHint>
                </div>
                <div className="w-40">
                  <Label htmlFor="bm-role">Board role</Label>
                  <Select value={memberRole} onValueChange={(v) => setMemberRole(v as BoardRole)}>
                    <SelectTrigger id="bm-role" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="viewer">viewer</SelectItem>
                      <SelectItem value="member">member</SelectItem>
                      <SelectItem value="admin">admin</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <Button onClick={() => void upsertMember()} disabled={busy || memberUserId.trim() === ""}>
                  {busy ? <Spinner /> : null} Set member
                </Button>
              </div>
            ) : null}
          </Card>

          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Checklist template — {selected.name}</h2>
            {template === null ? (
              <div className="p-4">
                <Spinner />
              </div>
            ) : template.template === null || template.template === undefined ? (
              <div className="p-4">
                <EmptyState message="No template yet. A template defines required steps that copy onto tickets and gate closing when enforcement is on." />
              </div>
            ) : (
              <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Existing template “{template.template.name}” with {template.items.length} step(s).
              </p>
            )}
            {canAdmin ? (
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="ct-name">Template name</Label>
                  <Input
                    id="ct-name"
                    value={templateName}
                    onChange={(e) => setTemplateName(e.target.value)}
                    placeholder={template === null ? "Offboarding checklist" : (template.template?.name ?? "Offboarding checklist")}
                  />
                </div>
                <div>
                  <Label>Required steps (in order)</Label>
                  <ol className="grid gap-1">
                    {templateItems.map((text, i) => (
                      <li key={i} className="flex items-center gap-2 text-[13px]">
                        <span style={{ color: "var(--m-text-muted)" }}>{i + 1}.</span>
                        <Input
                          aria-label={`Step ${i + 1}`}
                          value={text}
                          onChange={(e) =>
                            setTemplateItems(templateItems.map((t, j) => (j === i ? e.target.value : t)))
                          }
                        />
                        <Button variant="ghost" size="sm" onClick={() => setTemplateItems(templateItems.filter((_, j) => j !== i))}>
                          Remove
                        </Button>
                      </li>
                    ))}
                  </ol>
                  <div className="mt-1 flex items-end gap-2">
                    <div className="flex-1">
                      <Input
                        aria-label="New step"
                        placeholder="Next required step…"
                        value={newItemText}
                        onChange={(e) => setNewItemText(e.target.value)}
                      />
                    </div>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={newItemText.trim() === ""}
                      onClick={() => {
                        setTemplateItems([...templateItems, newItemText.trim()]);
                        setNewItemText("");
                      }}
                    >
                      Add step
                    </Button>
                  </div>
                </div>
                <FieldHint>
                  Saving replaces the whole template and item list; tickets already carry their
                  instances, so enforcement follows item identity, not text.
                </FieldHint>
                <div className="flex gap-2">
                  <Button onClick={() => void saveTemplate()} disabled={busy || templateName.trim() === ""}>
                    {busy ? <Spinner /> : null} Save template
                  </Button>
                  {template !== null && template.template !== null && template.template !== undefined ? (
                    <Button variant="ghost" onClick={() => void deleteTemplate()} disabled={busy}>
                      Delete template
                    </Button>
                  ) : null}
                </div>
              </div>
            ) : null}
          </Card>
        </>
      ) : null}

      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        Statuses, transitions, and custom fields stay on{" "}
        <Link href="/settings/workflow" className="underline decoration-[var(--m-border)] underline-offset-2">
          Workflow
        </Link>
        .
      </p>
    </div>
  );
}
