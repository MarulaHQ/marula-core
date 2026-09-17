// /dispatch/scheduling — scheduling-links template admin (Wave 17 lane 3).
//
// Purpose: manage the bookable templates that the ticket-side
// scheduling pod sends to clients: duration, buffer, booking window,
// assignee scope (a single user or a scheduling group), and the
// auto-email subject/body that fires when a link is minted from a
// template with the send enabled. Also manages the scheduling groups
// (availability pools) those templates can target. Meeting-type writes
// are envelope-gated (MutationParams); group writes are plain.
//
// Non-purpose: no link lifecycle here (the ticket scheduling pod owns
// send/manage); no availability windows (calendar surface owns those).
"use client";

import type { components } from "@meridian/sdk";
import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
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
  Tag,
} from "@meridian/ui";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { useRequireAuth } from "../../../lib/api";
import { api } from "../../../lib/api";

type MeetingType = components["schemas"]["MeetingType"];
type SchedulingGroup = components["schemas"]["SchedulingGroup"];
type OrgUser = components["schemas"]["OrgUser"];

const DURATION_MIN = 5;
const DURATION_MAX = 480;
const BUFFER_MAX = 120;
const WINDOW_MIN = 1;
const WINDOW_MAX = 365;

function userName(users: OrgUser[], id: string | null | undefined): string {
  if (id === null || id === undefined) return "—";
  return users.find((u) => u.id === id)?.display_name ?? id.slice(0, 8);
}

function scopeLabel(
  mt: MeetingType,
  users: OrgUser[],
  groups: SchedulingGroup[],
): string {
  if (mt.assignee_scope === "user") return `User · ${userName(users, mt.assignee_user_id)}`;
  if (mt.assignee_scope === "group") {
    const g = groups.find((x) => x.id === mt.assignee_group_id);
    return `Group · ${g?.name ?? mt.assignee_group_id?.slice(0, 8) ?? "—"}`;
  }
  return "Unscoped";
}

interface TemplateDialogProps {
  open: boolean;
  existing: MeetingType | null;
  users: OrgUser[];
  groups: SchedulingGroup[];
  onClose(): void;
  onSaved(mt: MeetingType): void;
}

/// Create/edit dialog for a meeting-type template with typed validation:
/// required name, duration/buffer/window bounds, scope must carry its
/// target id, and auto-email requires subject + body when enabled.
function TemplateDialog({
  open,
  existing,
  users,
  groups,
  onClose,
  onSaved,
}: TemplateDialogProps): ReactNode {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [duration, setDuration] = useState("30");
  const [buffer, setBuffer] = useState("0");
  const [windowDays, setWindowDays] = useState("14");
  const [scope, setScope] = useState<"user" | "group">("user");
  const [userId, setUserId] = useState("");
  const [groupId, setGroupId] = useState("");
  const [autoEmail, setAutoEmail] = useState(false);
  const [emailSubject, setEmailSubject] = useState("");
  const [emailBody, setEmailBody] = useState("");
  const [active, setActive] = useState(true);
  const [errors, setErrors] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setErrors([]);
    if (existing === null) {
      setName("");
      setDescription("");
      setDuration("30");
      setBuffer("0");
      setWindowDays("14");
      setScope(users.length > 0 ? "user" : "group");
      setUserId(users[0]?.id ?? "");
      setGroupId(groups[0]?.id ?? "");
      setAutoEmail(false);
      setEmailSubject("");
      setEmailBody("");
      setActive(true);
      return;
    }
    setName(existing.name);
    setDescription(existing.description ?? "");
    setDuration(String(existing.duration_minutes));
    setBuffer(String(existing.buffer_minutes));
    setWindowDays(String(existing.booking_window_days));
    setScope(existing.assignee_scope === "group" ? "group" : "user");
    setUserId(existing.assignee_user_id ?? users[0]?.id ?? "");
    setGroupId(existing.assignee_group_id ?? groups[0]?.id ?? "");
    setAutoEmail(existing.auto_email_enabled);
    setEmailSubject(existing.auto_email_subject ?? "");
    setEmailBody(existing.auto_email_body ?? "");
    setActive(existing.is_active);
  }, [open, existing, users, groups]);

  const save = async () => {
    const found: string[] = [];
    const trimmed = name.trim();
    const dur = Number(duration);
    const buf = Number(buffer);
    const win = Number(windowDays);
    if (trimmed === "") found.push("Template name is required.");
    if (!Number.isInteger(dur) || dur < DURATION_MIN || dur > DURATION_MAX) {
      found.push(`Duration must be a whole number between ${DURATION_MIN} and ${DURATION_MAX} minutes.`);
    }
    if (!Number.isInteger(buf) || buf < 0 || buf > BUFFER_MAX) {
      found.push(`Buffer must be a whole number between 0 and ${BUFFER_MAX} minutes.`);
    }
    if (!Number.isInteger(win) || win < WINDOW_MIN || win > WINDOW_MAX) {
      found.push(`Booking window must be between ${WINDOW_MIN} and ${WINDOW_MAX} days.`);
    }
    if (scope === "user" && userId === "") found.push("Pick the user this template assigns to.");
    if (scope === "group" && groupId === "") found.push("Pick the group this template assigns to.");
    if (autoEmail && (emailSubject.trim() === "" || emailBody.trim() === "")) {
      found.push("Auto-email needs both a subject and a body.");
    }
    if (found.length > 0) {
      setErrors(found);
      return;
    }
    setErrors([]);
    setBusy(true);
    const body = {
      name: trimmed,
      description: description.trim() === "" ? null : description.trim(),
      duration_minutes: dur,
      buffer_minutes: buf,
      booking_window_days: win,
      assignee_scope: scope,
      assignee_user_id: scope === "user" ? userId : null,
      assignee_group_id: scope === "group" ? groupId : null,
      auto_email_enabled: autoEmail,
      auto_email_subject: autoEmail ? emailSubject.trim() : null,
      auto_email_body: autoEmail ? emailBody.trim() : null,
      ...(existing !== null ? { is_active: active } : {}),
    };
    const res =
      existing === null
        ? await api.api.POST("/api/v1/meeting-types", { body })
        : await api.api.PATCH("/api/v1/meeting-types/{id}", {
            params: { path: { id: existing.id } },
            body,
          });
    setBusy(false);
    if (res.error !== undefined || res.data === undefined) {
      setErrors(["The template could not be saved. Check your role and try again."]);
      return;
    }
    onSaved(res.data.meeting_type);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent
        title={existing === null ? "New meeting type" : `Edit ${existing.name}`}
        description="Clients pick a slot on the shareable booking page built from this template."
      >
        <div className="grid max-h-[70vh] gap-3 overflow-y-auto p-4">
          <div>
            <Label htmlFor="mt-name">Name</Label>
            <Input
              id="mt-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              aria-invalid={errors.length > 0 && name.trim() === ""}
            />
          </div>
          <div>
            <Label htmlFor="mt-desc">Description</Label>
            <Input
              id="mt-desc"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Shown on the booking page (optional)"
            />
          </div>
          <div className="grid grid-cols-3 gap-3">
            <div>
              <Label htmlFor="mt-duration">Duration (min)</Label>
              <Input
                id="mt-duration"
                type="number"
                min={DURATION_MIN}
                max={DURATION_MAX}
                value={duration}
                onChange={(e) => setDuration(e.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="mt-buffer">Buffer (min)</Label>
              <Input
                id="mt-buffer"
                type="number"
                min={0}
                max={BUFFER_MAX}
                value={buffer}
                onChange={(e) => setBuffer(e.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="mt-window">Booking window (days)</Label>
              <Input
                id="mt-window"
                type="number"
                min={WINDOW_MIN}
                max={WINDOW_MAX}
                value={windowDays}
                onChange={(e) => setWindowDays(e.target.value)}
              />
            </div>
          </div>
          <div className="grid gap-1.5">
            <Label>Assignee scope</Label>
            <div role="radiogroup" aria-label="Assignee scope" className="flex items-center gap-2">
                {(["user", "group"] as const).map((s) => (
                  <button
                    key={s}
                    type="button"
                    role="radio"
                    aria-checked={scope === s}
                    data-testid={`mt-scope-${s}`}
                    className="rounded-full border px-3 py-1 text-xs font-medium capitalize"
                    style={{
                      borderColor: scope === s ? "var(--m-accent)" : "var(--m-border)",
                      color: scope === s ? "var(--m-accent)" : "var(--m-text-secondary)",
                      boxShadow: scope === s ? "0 0 0 1px var(--m-accent)" : "none",
                    }}
                    onClick={() => setScope(s)}
                  >
                    {s}
                  </button>
                ))}
            </div>
            {scope === "user" ? (
              <Select value={userId} onValueChange={setUserId}>
                <SelectTrigger aria-label="Assignee user" className="w-64">
                  <SelectValue placeholder="Pick a user" />
                </SelectTrigger>
                <SelectContent>
                  {users.map((u) => (
                    <SelectItem key={u.id} value={u.id}>
                      {u.display_name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <Select value={groupId} onValueChange={setGroupId}>
                <SelectTrigger aria-label="Assignee group" className="w-64">
                  <SelectValue placeholder="Pick a group" />
                </SelectTrigger>
                <SelectContent>
                  {groups.map((g) => (
                    <SelectItem key={g.id} value={g.id}>
                      {g.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>
          <label className="flex items-center gap-2 text-[13px]">
            <Checkbox
              checked={autoEmail}
              onCheckedChange={(v) => setAutoEmail(v === true)}
              aria-label="Auto-send email when a link is minted"
              data-testid="mt-auto-email"
            />
            Auto-send the scheduling email when a link is minted
          </label>
          {autoEmail && (
            <div className="grid gap-2 rounded-[var(--m-radius-container)] border p-3" style={{ borderColor: "var(--m-border)" }}>
              <div>
                <Label htmlFor="mt-email-subject">Email subject</Label>
                <Input
                  id="mt-email-subject"
                  value={emailSubject}
                  onChange={(e) => setEmailSubject(e.target.value)}
                  placeholder="Schedule your service visit"
                />
              </div>
              <div>
                <Label htmlFor="mt-email-body">Email body</Label>
                <textarea
                  id="mt-email-body"
                  value={emailBody}
                  onChange={(e) => setEmailBody(e.target.value)}
                  rows={3}
                  className="w-full rounded-[var(--m-radius-control)] border px-2 py-1.5 text-[13px]"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
                  placeholder="Hi — pick a time that works for you:"
                />
              </div>
            </div>
          )}
          {existing !== null && (
            <label className="flex items-center gap-2 text-[13px]">
              <Checkbox
                checked={active}
                onCheckedChange={(v) => setActive(v === true)}
                aria-label="Template active"
                data-testid="mt-active"
              />
              Active (bookable)
            </label>
          )}
          {errors.length > 0 && (
            <ul role="alert" className="grid gap-1 text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
              {errors.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          )}
          <DialogActions>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button onClick={() => void save()} disabled={busy}>
              {busy ? <Spinner /> : null} {existing === null ? "Create template" : "Save template"}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function DispatchSchedulingPage() {
  const auth = useRequireAuth();
  const [templates, setTemplates] = useState<MeetingType[] | null>(null);
  const [groups, setGroups] = useState<SchedulingGroup[] | null>(null);
  const [users, setUsers] = useState<OrgUser[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<MeetingType | null>(null);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [addMemberId, setAddMemberId] = useState("");

  const refresh = useCallback(async () => {
    const [mt, sg, ou] = await Promise.all([
      api.api.GET("/api/v1/meeting-types"),
      api.api.GET("/api/v1/scheduling-groups"),
      api.api.GET("/api/v1/org/users"),
    ]);
    if (mt.error !== undefined || sg.error !== undefined || ou.error !== undefined) {
      setLoadError("Scheduling admin data could not be loaded.");
      return;
    }
    setTemplates(mt.data ?? []);
    setGroups(sg.data ?? []);
    setUsers(ou.data?.users ?? []);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const selectedGroup = useMemo(
    () => groups?.find((g) => g.id === selectedGroupId) ?? null,
    [groups, selectedGroupId],
  );

  const onTemplateSaved = useCallback((mt: MeetingType) => {
    setTemplates((prev) => {
      const base = prev ?? [];
      const idx = base.findIndex((t) => t.id === mt.id);
      if (idx === -1) return [...base, mt];
      const next = [...base];
      next[idx] = mt;
      return next;
    });
    setDialogOpen(false);
    setEditing(null);
  }, []);

  const addMember = useCallback(async () => {
    if (selectedGroup === null || addMemberId === "") return;
    const idempotencyKey =
      typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `key-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const res = await api.api.POST("/api/v1/scheduling-groups/{id}/members", {
      params: { path: { id: selectedGroup.id } },
      body: { user_id: addMemberId, idempotency_key: idempotencyKey },
    });
    if (res.data !== undefined) {
      setGroups((prev) => (prev ?? []).map((g) => (g.id === res.data?.id ? res.data : g)));
      setAddMemberId("");
    }
  }, [addMemberId, selectedGroup]);

  const removeMember = useCallback(
    async (groupId: string, userId: string) => {
      const res = await api.api.DELETE("/api/v1/scheduling-groups/{id}/members/{user_id}", {
        params: { path: { id: groupId, user_id: userId } },
      });
      if (res.data !== undefined) {
        setGroups((prev) => (prev ?? []).map((g) => (g.id === res.data?.id ? res.data : g)));
      }
    },
    [],
  );

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 p-4 md:p-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">Scheduling templates</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Meeting types and availability pools behind the ticket scheduling links.
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" onClick={() => void refresh()}>
            Refresh
          </Button>
          <Button
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
            data-testid="new-template"
          >
            New meeting type
          </Button>
        </div>
      </div>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : templates === null || groups === null || users === null ? (
        <Spinner />
      ) : (
        <>
          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Meeting types</h2>
            {templates.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No meeting types yet. Create one to start sending scheduling links from tickets." />
              </div>
            ) : (
              <ul className="grid gap-2 p-4" aria-label="Meeting types" data-testid="template-list">
                {templates.map((t) => (
                  <li
                    key={t.id}
                    data-testid={`template-${t.name}`}
                    className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-container)] border px-3 py-2"
                    style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
                  >
                    <span className="min-w-0">
                      <span className="block truncate text-[13px] font-medium">{t.name}</span>
                      {t.description !== null && t.description !== "" && (
                        <span className="block truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          {t.description}
                        </span>
                      )}
                    </span>
                    <Tag>{t.duration_minutes}m</Tag>
                    <Tag>buffer {t.buffer_minutes}m</Tag>
                    <Tag>window {t.booking_window_days}d</Tag>
                    <Tag>{scopeLabel(t, users, groups)}</Tag>
                    {t.auto_email_enabled ? (
                      <StatusTag tone="neutral">auto-email</StatusTag>
                    ) : (
                      <Tag>link only</Tag>
                    )}
                    <span className="ml-auto" />
                    {t.is_active ? (
                      <StatusTag tone="success">Active</StatusTag>
                    ) : (
                      <StatusTag tone="warning">Paused</StatusTag>
                    )}
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-label={`Edit ${t.name}`}
                      onClick={() => {
                        setEditing(t);
                        setDialogOpen(true);
                      }}
                    >
                      Edit
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Scheduling groups</h2>
            {groups.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No groups yet. Groups pool availability across technicians for group-scoped templates." />
              </div>
            ) : (
              <div className="grid gap-3 p-4">
                <Select
                  value={selectedGroupId ?? ""}
                  onValueChange={(v) => setSelectedGroupId(v === "" ? null : v)}
                >
                  <SelectTrigger aria-label="Pick a group to manage" className="w-72">
                    <SelectValue placeholder="Pick a group to manage" />
                  </SelectTrigger>
                  <SelectContent>
                    {groups.map((g) => (
                      <SelectItem key={g.id} value={g.id}>
                        {g.name} ({g.member_user_ids.length})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {selectedGroup !== null && (
                  <div className="grid gap-2" data-testid={`group-panel-${selectedGroup.name}`}>
                    <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                      {selectedGroup.description ?? "Availability pool"}
                    </p>
                    <ul className="flex flex-wrap gap-1.5" aria-label={`Members of ${selectedGroup.name}`}>
                      {selectedGroup.member_user_ids.map((uid) => (
                        <li
                          key={uid}
                          className="flex items-center gap-1 rounded-[var(--m-radius-control)] border px-2 py-0.5 text-xs"
                          style={{ borderColor: "var(--m-border)" }}
                        >
                          {userName(users, uid)}
                          <button
                            type="button"
                            aria-label={`Remove ${userName(users, uid)} from ${selectedGroup.name}`}
                            className="font-semibold hover:opacity-70"
                            style={{ color: "var(--m-text-muted)" }}
                            onClick={() => void removeMember(selectedGroup.id, uid)}
                          >
                            ×
                          </button>
                        </li>
                      ))}
                      {selectedGroup.member_user_ids.length === 0 && (
                        <li className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                          No members yet.
                        </li>
                      )}
                    </ul>
                    <div className="flex items-center gap-2">
                      <Select value={addMemberId} onValueChange={setAddMemberId}>
                        <SelectTrigger aria-label="Add member" className="w-64">
                          <SelectValue placeholder="Add a member…" />
                        </SelectTrigger>
                        <SelectContent>
                          {users
                            .filter((u) => !selectedGroup.member_user_ids.includes(u.id))
                            .map((u) => (
                              <SelectItem key={u.id} value={u.id}>
                                {u.display_name}
                              </SelectItem>
                            ))}
                        </SelectContent>
                      </Select>
                      <Button size="sm" variant="secondary" disabled={addMemberId === ""} onClick={() => void addMember()}>
                        Add member
                      </Button>
                    </div>
                  </div>
                )}
              </div>
            )}
          </Card>
        </>
      )}

      <TemplateDialog
        open={dialogOpen}
        existing={editing}
        users={users ?? []}
        groups={groups ?? []}
        onClose={() => {
          setDialogOpen(false);
          setEditing(null);
        }}
        onSaved={onTemplateSaved}
      />
    </div>
  );
}
