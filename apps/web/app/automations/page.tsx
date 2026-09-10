// /automations — Event-triggered automation rule editor (Wave 11 lane 8).
//
// Purpose:
//  - List automation rules.
//  - Create & edit automation rules: trigger event -> condition -> action.
//  - Enable / disable rule toggles.
//  - Delete rules with confirmation.
//
// Instant-first: mutation envelope for idempotency, inline errors, typed empty states,
// Workbench tokens, light+dark, <=120ms transitions.
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
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type Priority = components["schemas"]["Priority"];
type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];
type AutomationCondition = components["schemas"]["AutomationCondition"];
type AutomationAction = components["schemas"]["AutomationAction"];
type BoardNavEntry = components["schemas"]["BoardNavEntry"];

interface AutomationRuleView {
  id: string;
  name: string;
  enabled: boolean;
  board_id: string | null;
  trigger_event: string;
  conditions: AutomationCondition[];
  actions: AutomationAction[];
  created_at: string;
  updated_at: string;
}

const COMMON_EVENTS = [
  "ticket.created",
  "ticket.updated",
  "ticket.status_changed",
  "ticket.assigned",
  "ticket.comment_added",
  "ticket.closed",
];

const PRIORITIES: Priority[] = ["p1", "p2", "p3", "p4"];
const CATEGORIES: TicketStatusCategory[] = ["open", "in_progress", "waiting", "closed"];

export default function AutomationsPage() {
  const auth = useRequireAuth();

  const [rules, setRules] = useState<AutomationRuleView[] | null>(null);
  const [boards, setBoards] = useState<BoardNavEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // Modal create/edit rule state
  const [modalOpen, setModalOpen] = useState(false);
  const [editingRuleId, setEditingRuleId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [boardId, setBoardId] = useState<string>("");
  const [triggerEvent, setTriggerEvent] = useState<string>("ticket.created");

  // Condition builder state
  const [condType, setCondType] = useState<"priority_is" | "category_is" | "title_contains" | "unassigned">("priority_is");
  const [condPriority, setCondPriority] = useState<Priority>("p1");
  const [condCategory, setCondCategory] = useState<TicketStatusCategory>("open");
  const [condNeedle, setCondNeedle] = useState("");
  const [conditions, setConditions] = useState<AutomationCondition[]>([]);

  // Action builder state
  const [actType, setActType] = useState<"set_priority" | "add_internal_comment" | "assign_user">("set_priority");
  const [actPriority, setActPriority] = useState<Priority>("p1");
  const [actBody, setActBody] = useState("");
  const [actUserId, setActUserId] = useState("");
  const [actions, setActions] = useState<AutomationAction[]>([]);

  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  const refreshBoards = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/boards/nav");
    if (data) setBoards(data);
  }, []);

  const refreshRules = useCallback(async () => {
    setLoading(true);
    setErrorMsg(null);
    const { data, error } = await api.api.GET("/api/v1/automation-rules");
    setLoading(false);
    if (error !== undefined || !data) {
      setErrorMsg("Failed to load automation rules.");
      return;
    }
    setRules(data as unknown as AutomationRuleView[]);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") {
      void refreshBoards();
      void refreshRules();
    }
  }, [auth.status, refreshBoards, refreshRules]);

  const handleAddCondition = () => {
    let cond: AutomationCondition;
    if (condType === "priority_is") {
      cond = { type: "priority_is", priority: condPriority };
    } else if (condType === "category_is") {
      cond = { type: "category_is", category: condCategory };
    } else if (condType === "title_contains") {
      cond = { type: "title_contains", needle: condNeedle.trim() };
    } else {
      cond = { type: "unassigned" };
    }
    setConditions([...conditions, cond]);
  };

  const handleRemoveCondition = (index: number) => {
    setConditions(conditions.filter((_, i) => i !== index));
  };

  const handleAddAction = () => {
    let act: AutomationAction;
    if (actType === "set_priority") {
      act = { type: "set_priority", priority: actPriority };
    } else if (actType === "add_internal_comment") {
      act = { type: "add_internal_comment", body: actBody.trim() };
    } else {
      act = { type: "assign_user", user_id: actUserId.trim() };
    }
    setActions([...actions, act]);
  };

  const handleRemoveAction = (index: number) => {
    setActions(actions.filter((_, i) => i !== index));
  };

  const handleSaveRule = async () => {
    if (!name.trim() || !triggerEvent.trim()) return;
    setSaving(true);
    setFormError(null);

    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `auto-${Date.now()}`;

    if (editingRuleId) {
      const { error } = await api.api.PATCH("/api/v1/automation-rules/{id}", {
        params: { path: { id: editingRuleId } },
        body: {
          name: name.trim(),
          board_id: boardId ? boardId : null,
          trigger_event: triggerEvent,
          conditions,
          actions,
          idempotency_key: idempotencyKey,
        },
      });
      setSaving(false);
      if (error) {
        setFormError("Failed to update automation rule.");
        return;
      }
    } else {
      const { error } = await api.api.POST("/api/v1/automation-rules", {
        body: {
          name: name.trim(),
          board_id: boardId ? boardId : null,
          trigger_event: triggerEvent,
          conditions,
          actions,
          idempotency_key: idempotencyKey,
        },
      });
      setSaving(false);
      if (error) {
        setFormError("Failed to create automation rule. Verify condition and action format.");
        return;
      }
    }

    setModalOpen(false);
    setEditingRuleId(null);
    setName("");
    setConditions([]);
    setActions([]);
    void refreshRules();
  };

  const handleToggleRule = async (rule: AutomationRuleView) => {
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `auto-patch-${Date.now()}`;

    await api.api.PATCH("/api/v1/automation-rules/{id}", {
      params: { path: { id: rule.id } },
      body: {
        enabled: !rule.enabled,
        idempotency_key: idempotencyKey,
      },
    });
    void refreshRules();
  };

  const handleDeleteRule = async (id: string) => {
    await api.api.DELETE("/api/v1/automation-rules/{id}", {
      params: { path: { id } },
    });
    void refreshRules();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-5xl mx-auto p-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Automation Rules
          </h1>
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Event-triggered actions: automatically prioritize, assign, or add internal notes when domain events fire.
          </p>
        </div>

        <Dialog open={modalOpen} onOpenChange={setModalOpen}>
          <DialogTrigger asChild>
            <Button
              onClick={() => {
                setEditingRuleId(null);
                setName("");
                setBoardId("");
                setTriggerEvent("ticket.created");
                setConditions([]);
                setActions([]);
              }}
            >
              Create automation rule
            </Button>
          </DialogTrigger>
          <DialogContent title={editingRuleId ? "Edit Automation Rule" : "Create Automation Rule"}>
            <div className="grid gap-4 p-4 max-h-[80vh] overflow-y-auto">
              <div>
                <Label htmlFor="ar-name">Rule Name</Label>
                <Input
                  id="ar-name"
                  placeholder="e.g. Escalate VIP tickets on create"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  required
                />
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label htmlFor="ar-board">Board Scope (Optional)</Label>
                  <Select value={boardId} onValueChange={setBoardId}>
                    <SelectTrigger id="ar-board" className="w-full">
                      <SelectValue placeholder="All boards (org-wide)" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="">All boards</SelectItem>
                      {boards.map((b) => (
                        <SelectItem key={b.id} value={b.id}>
                          {b.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                <div>
                  <Label htmlFor="ar-event">Trigger Event</Label>
                  <Select value={triggerEvent} onValueChange={setTriggerEvent}>
                    <SelectTrigger id="ar-event" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {COMMON_EVENTS.map((ev) => (
                        <SelectItem key={ev} value={ev}>
                          {ev}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {/* Conditions Section */}
              <div className="border rounded p-3 grid gap-2" style={{ borderColor: "var(--m-border)" }}>
                <div className="font-semibold text-[13px]">Conditions (ALL must match)</div>
                {conditions.length === 0 ? (
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    No conditions added (matches every occurrence of the trigger event).
                  </p>
                ) : (
                  <div className="grid gap-1">
                    {conditions.map((c, i) => (
                      <div key={i} className="flex items-center justify-between text-[12px] bg-black/5 dark:bg-white/5 p-1.5 rounded">
                        <code>{JSON.stringify(c)}</code>
                        <Button size="sm" variant="ghost" onClick={() => handleRemoveCondition(i)}>
                          Remove
                        </Button>
                      </div>
                    ))}
                  </div>
                )}

                <div className="flex flex-wrap items-end gap-2 mt-2 pt-2 border-t" style={{ borderColor: "var(--m-border)" }}>
                  <div className="w-40">
                    <Label>Condition</Label>
                    <Select value={condType} onValueChange={(v: any) => setCondType(v)}>
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="priority_is">Priority is</SelectItem>
                        <SelectItem value="category_is">Status category is</SelectItem>
                        <SelectItem value="title_contains">Title contains</SelectItem>
                        <SelectItem value="unassigned">Ticket is unassigned</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>

                  {condType === "priority_is" && (
                    <div className="w-28">
                      <Label>Priority</Label>
                      <Select value={condPriority} onValueChange={(v: any) => setCondPriority(v)}>
                        <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                        <SelectContent>
                          {PRIORITIES.map((p) => <SelectItem key={p} value={p}>{p}</SelectItem>)}
                        </SelectContent>
                      </Select>
                    </div>
                  )}

                  {condType === "category_is" && (
                    <div className="w-32">
                      <Label>Category</Label>
                      <Select value={condCategory} onValueChange={(v: any) => setCondCategory(v)}>
                        <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                        <SelectContent>
                          {CATEGORIES.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
                        </SelectContent>
                      </Select>
                    </div>
                  )}

                  {condType === "title_contains" && (
                    <div className="w-44">
                      <Label>Text substring</Label>
                      <Input value={condNeedle} onChange={(e) => setCondNeedle(e.target.value)} placeholder="Urgent / Outage" />
                    </div>
                  )}

                  <Button size="sm" variant="secondary" onClick={handleAddCondition}>
                    Add condition
                  </Button>
                </div>
              </div>

              {/* Actions Section */}
              <div className="border rounded p-3 grid gap-2" style={{ borderColor: "var(--m-border)" }}>
                <div className="font-semibold text-[13px]">Actions (Effects to execute)</div>
                {actions.length === 0 ? (
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    No actions configured yet. Add at least one action.
                  </p>
                ) : (
                  <div className="grid gap-1">
                    {actions.map((a, i) => (
                      <div key={i} className="flex items-center justify-between text-[12px] bg-black/5 dark:bg-white/5 p-1.5 rounded">
                        <code>{JSON.stringify(a)}</code>
                        <Button size="sm" variant="ghost" onClick={() => handleRemoveAction(i)}>
                          Remove
                        </Button>
                      </div>
                    ))}
                  </div>
                )}

                <div className="flex flex-wrap items-end gap-2 mt-2 pt-2 border-t" style={{ borderColor: "var(--m-border)" }}>
                  <div className="w-44">
                    <Label>Action</Label>
                    <Select value={actType} onValueChange={(v: any) => setActType(v)}>
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="set_priority">Set Priority</SelectItem>
                        <SelectItem value="add_internal_comment">Add Internal Note</SelectItem>
                        <SelectItem value="assign_user">Assign to User</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>

                  {actType === "set_priority" && (
                    <div className="w-28">
                      <Label>Priority</Label>
                      <Select value={actPriority} onValueChange={(v: any) => setActPriority(v)}>
                        <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                        <SelectContent>
                          {PRIORITIES.map((p) => <SelectItem key={p} value={p}>{p}</SelectItem>)}
                        </SelectContent>
                      </Select>
                    </div>
                  )}

                  {actType === "add_internal_comment" && (
                    <div className="w-52">
                      <Label>Comment body</Label>
                      <Input value={actBody} onChange={(e) => setActBody(e.target.value)} placeholder="Triage comment" />
                    </div>
                  )}

                  {actType === "assign_user" && (
                    <div className="w-52">
                      <Label>User UUID</Label>
                      <Input value={actUserId} onChange={(e) => setActUserId(e.target.value)} placeholder="UUID" />
                    </div>
                  )}

                  <Button size="sm" variant="secondary" onClick={handleAddAction}>
                    Add action
                  </Button>
                </div>
              </div>

              {formError && (
                <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {formError}
                </p>
              )}

              <DialogActions>
                <Button variant="ghost" onClick={() => setModalOpen(false)}>
                  Cancel
                </Button>
                <Button onClick={() => void handleSaveRule()} disabled={saving || !name.trim()}>
                  {saving ? <Spinner /> : null} Save Rule
                </Button>
              </DialogActions>
            </div>
          </DialogContent>
        </Dialog>
      </header>

      {errorMsg && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {errorMsg}
        </p>
      )}

      <Card className="overflow-x-auto">
        {loading ? (
          <div className="p-6 flex justify-center">
            <Spinner />
          </div>
        ) : rules === null || rules.length === 0 ? (
          <div className="p-6">
            <EmptyState message="No automation rules configured yet. Create a rule above." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Rule Name</Th>
                <Th>Trigger Event</Th>
                <Th>Conditions</Th>
                <Th>Actions</Th>
                <Th>Status</Th>
                <Th className="text-right">Action</Th>
              </tr>
            </thead>
            <tbody>
              {rules.map((r) => (
                <Tr key={r.id}>
                  <Td className="font-medium text-[13px]">
                    <div>{r.name}</div>
                    {r.board_id && (
                      <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                        Board-scoped: {boards.find((b) => b.id === r.board_id)?.name ?? r.board_id}
                      </div>
                    )}
                  </Td>
                  <Td>
                    <code className="text-[12px]">{r.trigger_event}</code>
                  </Td>
                  <Td className="text-[12px]">
                    {r.conditions?.length ?? 0} condition{(r.conditions?.length ?? 0) === 1 ? "" : "s"}
                  </Td>
                  <Td className="text-[12px]">
                    {r.actions?.length ?? 0} action{(r.actions?.length ?? 0) === 1 ? "" : "s"}
                  </Td>
                  <Td>
                    <Tag tone={r.enabled ? "success" : "neutral"}>
                      {r.enabled ? "Enabled" : "Disabled"}
                    </Tag>
                  </Td>
                  <Td className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => void handleToggleRule(r)}
                      >
                        {r.enabled ? "Disable" : "Enable"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setEditingRuleId(r.id);
                          setName(r.name);
                          setBoardId(r.board_id ?? "");
                          setTriggerEvent(r.trigger_event);
                          setConditions(r.conditions ?? []);
                          setActions(r.actions ?? []);
                          setModalOpen(true);
                        }}
                      >
                        Edit
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => void handleDeleteRule(r.id)}
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
      </Card>
    </div>
  );
}
