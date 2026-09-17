// /boards/[id]/fields — custom-field editor (Wave 17 lane 3).
//
// Purpose: the board's typed custom-field definitions as a manageable
// matrix: add via a field-type picker (the closed FieldKindWire set),
// edit name/options/required with typed validation (choice kinds must
// carry a non-empty, unique option set), reorder with keyboard-reachable
// up/down controls that persist sort_order, and delete with an inline
// confirm. All writes hit the ConfigureBoard field endpoints directly —
// these are board-config writes, not envelope-gated ticket mutations.
// Non-purpose: no ticket values here (the ticket workspace renders
// values); no org-wide fields (v1 creates board-scoped only).
"use client";

import type { components } from "@meridian/sdk";
import {
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  Input,
  Label,
  Spinner,
  StatusTag,
} from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { BoardNav } from "../../../../components/board-nav";
import { api, useRequireAuth } from "../../../../lib/api";

type FieldDef = components["schemas"]["FieldDefView"];
type FieldKind = components["schemas"]["FieldKindWire"];

const KINDS: { kind: FieldKind; label: string; hint: string; glyph: string }[] = [
  { kind: "text", label: "Short text", hint: "One line of text", glyph: "T" },
  { kind: "number", label: "Number", hint: "Numeric value", glyph: "#" },
  { kind: "date", label: "Date", hint: "Calendar date", glyph: "▦" },
  { kind: "select", label: "Select", hint: "One of several options", glyph: "▾" },
  { kind: "multiselect", label: "Multi-select", hint: "Several of several options", glyph: "☰" },
  { kind: "bool", label: "Checkbox", hint: "Yes / no toggle", glyph: "✓" },
  { kind: "url", label: "URL", hint: "Link to a resource", glyph: "∞" },
  { kind: "user", label: "User", hint: "A staff assignee", glyph: "@" },
];

function kindMeta(kind: string): (typeof KINDS)[number] {
  return KINDS.find((k) => k.kind === kind) ?? { kind: "text", label: kind, hint: "", glyph: "?" };
}

const CHOICE_KINDS: FieldKind[] = ["select", "multiselect"];

interface OptionsEditorProps {
  options: string[];
  onChange(options: string[]): void;
  error: string | null;
}

/// Chip editor for select/multiselect option sets: add by Enter or button,
/// remove per chip. Validation (non-empty, unique) is enforced by the parent.
function OptionsEditor({ options, onChange, error }: OptionsEditorProps): ReactNode {
  const [draft, setDraft] = useState("");

  const add = () => {
    const value = draft.trim();
    if (value === "") return;
    onChange([...options, value]);
    setDraft("");
  };

  return (
    <div className="grid gap-1.5">
      <Label htmlFor="field-option-input">Options</Label>
      {options.length > 0 && (
        <ul className="flex flex-wrap gap-1.5" aria-label="Current options">
          {options.map((o, i) => (
            <li
              key={`${o}-${i}`}
              className="flex items-center gap-1 rounded-[var(--m-radius-control)] border px-2 py-0.5 text-xs"
              style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
            >
              {o}
              <button
                type="button"
                aria-label={`Remove option ${o}`}
                className="font-semibold hover:opacity-70"
                style={{ color: "var(--m-text-muted)" }}
                onClick={() => onChange(options.filter((_, idx) => idx !== i))}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-center gap-2">
        <Input
          id="field-option-input"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            }
          }}
          placeholder="Add an option…"
          className="h-8 text-xs"
          aria-invalid={error !== null}
        />
        <Button size="sm" variant="secondary" onClick={add}>
          Add
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      )}
    </div>
  );
}

interface FieldDialogProps {
  boardId: string;
  existing: FieldDef | null;
  nextSortOrder: number;
  onSaved(def: FieldDef): void;
  onClose(): void;
  open: boolean;
}

/// Add/edit dialog. The kind matrix shows on create only — kind is
/// immutable after create (PATCH carries no kind). Typed validation:
/// required name; choice kinds need ≥1 unique non-empty option.
function FieldDialog({
  boardId,
  existing,
  nextSortOrder,
  onSaved,
  onClose,
  open,
}: FieldDialogProps) {
  const [kind, setKind] = useState<FieldKind>(existing?.kind as FieldKind ?? "text");
  const [name, setName] = useState(existing?.name ?? "");
  const [required, setRequired] = useState(existing?.required ?? false);
  const [options, setOptions] = useState<string[]>(existing?.options ?? []);
  const [busy, setBusy] = useState(false);
  const [nameError, setNameError] = useState<string | null>(null);
  const [optionError, setOptionError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const isChoice = CHOICE_KINDS.includes(kind);

  const save = async () => {
    const trimmed = name.trim();
    if (trimmed === "") {
      setNameError("A field name is required");
      return;
    }
    setNameError(null);
    if (isChoice) {
      if (options.length === 0) {
        setOptionError("Choice fields need at least one option");
        return;
      }
      const dupes = options.some(
        (o, i) => options.findIndex((x) => x.toLowerCase() === o.toLowerCase()) !== i,
      );
      if (dupes) {
        setOptionError("Options must be unique");
        return;
      }
    }
    setOptionError(null);
    setBusy(true);
    setSaveError(null);
    const res =
      existing === null
        ? await api.api.POST("/api/v1/boards/{id}/fields", {
            params: { path: { id: boardId } },
            body: {
              name: trimmed,
              kind,
              required,
              sort_order: nextSortOrder,
              ...(isChoice ? { options } : {}),
            },
          })
        : await api.api.PATCH("/api/v1/boards/{id}/fields/{field_id}", {
            params: { path: { id: boardId, field_id: existing.id } },
            body: {
              name: trimmed,
              required,
              ...(isChoice ? { options } : {}),
            },
          });
    setBusy(false);
    if (res.error !== undefined || res.data === undefined) {
      setSaveError("The field could not be saved. Check the board role and try again.");
      return;
    }
    onSaved(res.data);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent
        title={existing === null ? "Add field" : `Edit ${existing.name}`}
        description={
          existing === null
            ? "Pick a type, name it, and it joins the ticket form on this board."
            : "The type is fixed after create; name, options, and required can change."
        }
      >
        <div className="grid max-h-[70vh] gap-3 overflow-y-auto p-4">
          {existing === null && (
            <div className="grid gap-1.5">
              <Label>Field type</Label>
              <div
                role="radiogroup"
                aria-label="Field type"
                data-testid="field-kind-matrix"
                className="grid grid-cols-2 gap-1.5 sm:grid-cols-4"
              >
                {KINDS.map((k) => {
                  const active = kind === k.kind;
                  return (
                    <button
                      key={k.kind}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      data-testid={`field-kind-${k.kind}`}
                      className="relative grid gap-0.5 rounded-[var(--m-radius-container)] border p-2 text-left"
                      style={{
                        borderColor: active ? "var(--m-accent)" : "var(--m-border)",
                        boxShadow: active ? "0 0 0 1px var(--m-accent)" : "none",
                      }}
                      onClick={() => setKind(k.kind)}
                    >
                      {active && (
                        <span
                          aria-hidden
                          className="absolute right-1.5 top-1.5 grid h-4 w-4 place-items-center rounded-full text-[10px] font-bold"
                          style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
                        >
                          ✓
                        </span>
                      )}
                      <span className="pr-4 text-sm font-semibold" style={{ color: active ? "var(--m-accent)" : "var(--m-text)" }}>
                        {k.glyph} {k.label}
                      </span>
                      <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                        {k.hint}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
          <div>
            <Label htmlFor="field-name">Name</Label>
            <Input
              id="field-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              aria-invalid={nameError !== null}
              autoFocus
            />
            {nameError !== null && (
              <p role="alert" className="mt-1 text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
                {nameError}
              </p>
            )}
          </div>
          {isChoice && (
            <OptionsEditor
              options={options}
              onChange={setOptions}
              error={optionError}
            />
          )}
          <label className="flex items-center gap-2 text-xs">
            <Checkbox
              checked={required}
              onCheckedChange={(v) => setRequired(v === true)}
              aria-label="Required"
              data-testid="field-required"
            />
            Required on every ticket
          </label>
          {saveError !== null && (
            <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
              {saveError}
            </p>
          )}
          <DialogActions>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button onClick={() => void save()} disabled={busy}>
              {busy ? <Spinner /> : null} {existing === null ? "Create field" : "Save changes"}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function BoardFieldsPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const boardId = typeof params?.id === "string" ? params.id : "";

  const [fields, setFields] = useState<FieldDef[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<FieldDef | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const res = await api.api.GET("/api/v1/boards/{id}/fields", {
      params: { path: { id: boardId } },
    });
    if (res.error !== undefined || res.data === undefined) {
      setError("Fields could not be loaded for this board.");
      return;
    }
    setFields(res.data);
  }, [boardId]);

  useEffect(() => {
    if (boardId !== "") void refresh();
  }, [boardId, refresh]);

  const sorted = useMemo(
    () => [...(fields ?? [])].sort((a, b) => a.sort_order - b.sort_order),
    [fields],
  );

  const onSaved = useCallback(
    (def: FieldDef) => {
      setFields((prev) => {
        const base = prev ?? [];
        const idx = base.findIndex((f) => f.id === def.id);
        if (idx === -1) return [...base, def];
        const next = [...base];
        next[idx] = def;
        return next;
      });
      setDialogOpen(false);
      setEditing(null);
    },
    [],
  );

  /// Reorder: swap with the neighbor in the sorted list, then persist both
  /// affected sort_order values. Optimistic local swap, revert on failure.
  const reorder = useCallback(
    async (index: number, dir: -1 | 1) => {
      const list = sorted;
      const a = list[index];
      const b = list[index + dir];
      if (a === undefined || b === undefined) return;
      setBusyId(a.id);
      const swap = (arr: FieldDef[]): FieldDef[] =>
        arr.map((f) =>
          f.id === a.id ? { ...f, sort_order: b.sort_order } : f.id === b.id ? { ...f, sort_order: a.sort_order } : f,
        );
      const before = fields;
      setFields((prev) => (prev === null ? prev : swap(prev)));
      const resA = await api.api.PATCH("/api/v1/boards/{id}/fields/{field_id}", {
        params: { path: { id: boardId, field_id: a.id } },
        body: { sort_order: b.sort_order },
      });
      const resB = await api.api.PATCH("/api/v1/boards/{id}/fields/{field_id}", {
        params: { path: { id: boardId, field_id: b.id } },
        body: { sort_order: a.sort_order },
      });
      setBusyId(null);
      if (resA.error !== undefined || resB.error !== undefined) {
        setFields(before);
        setError("The order could not be saved. Nothing changed.");
        return;
      }
      if (resA.data !== undefined && resB.data !== undefined) {
        setFields((prev) =>
          prev === null ? prev : prev.map((f) => (f.id === resA.data?.id ? resA.data : f.id === resB.data?.id ? resB.data : f)),
        );
      }
    },
    [boardId, fields, sorted],
  );

  const remove = useCallback(
    async (field: FieldDef) => {
      setBusyId(field.id);
      const res = await api.api.DELETE("/api/v1/boards/{id}/fields/{field_id}", {
        params: { path: { id: boardId, field_id: field.id } },
      });
      setBusyId(null);
      if (res.error !== undefined) {
        setError("The field could not be removed. Check the board role and try again.");
        return;
      }
      setFields((prev) => (prev === null ? prev : prev.filter((f) => f.id !== field.id)));
      setConfirmingDelete(null);
    },
    [boardId],
  );

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <BoardNav />
        <main
          className="flex min-w-0 flex-1 flex-col gap-3 overflow-auto p-4 md:p-6"
          style={{ borderLeft: "1px solid var(--m-border)" }}
        >
          <div className="flex flex-wrap items-center gap-2">
            <div className="grid">
              <h1 className="text-sm font-semibold">Custom fields</h1>
              <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                Typed fields on every ticket of this board.
              </p>
            </div>
            <span className="ml-auto" />
            <Button size="sm" variant="ghost" onClick={() => router.push(`/boards/${boardId}`)}>
              Back to board
            </Button>
            <Button
              size="sm"
              data-testid="add-field"
              onClick={() => {
                setEditing(null);
                setDialogOpen(true);
              }}
            >
              Add field
            </Button>
            <FieldDialog
              boardId={boardId}
              existing={editing}
              nextSortOrder={
                sorted.length === 0 ? 0 : (sorted[sorted.length - 1]?.sort_order ?? 0) + 1
              }
              onSaved={onSaved}
              onClose={() => {
                setDialogOpen(false);
                setEditing(null);
              }}
              open={dialogOpen}
            />
          </div>

          {error !== null && (
            <p
              role="alert"
              className="rounded-[var(--m-radius-control)] px-3 py-1.5 text-xs"
              style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
            >
              {error}
            </p>
          )}

          {fields === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : sorted.length === 0 ? (
            <div
              className="grid place-items-center gap-2 rounded-[var(--m-radius-container)] border p-8 text-center"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              data-testid="fields-empty"
            >
              <p className="text-sm font-medium">No custom fields yet</p>
              <p className="max-w-sm text-xs" style={{ color: "var(--m-text-muted)" }}>
                Add typed fields — short text, number, select, date, and more — and they appear
                on every ticket of this board.
              </p>
              <Button size="sm" onClick={() => setDialogOpen(true)}>
                Add the first field
              </Button>
            </div>
          ) : (
            <ol
              aria-label="Custom fields"
              data-testid="field-list"
              className="flex flex-col gap-2"
            >
              {sorted.map((f, i) => {
                const meta = kindMeta(f.kind);
                return (
                  <li
                    key={f.id}
                    data-testid={`field-row-${f.name}`}
                    className="flex items-center gap-2 rounded-[var(--m-radius-container)] border px-3 py-2"
                    style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                  >
                    <span
                      aria-hidden
                      className="grid h-7 w-7 shrink-0 place-items-center rounded-[var(--m-radius-control)] text-sm"
                      style={{ background: "var(--m-accent-soft)", color: "var(--m-accent)" }}
                    >
                      {meta.glyph}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-[13px] font-medium">{f.name}</span>
                      <span className="block text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                        {meta.label}
                        {CHOICE_KINDS.includes(f.kind as FieldKind) ? ` · ${f.options.length} options` : ""}
                      </span>
                    </span>
                    {f.required && <StatusTag tone="warning">Required</StatusTag>}
                    <span className="ml-auto" />
                    <span className="flex items-center gap-0.5">
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Move ${f.name} up`}
                        disabled={i === 0 || busyId === f.id}
                        data-testid={`field-up-${f.name}`}
                        onClick={() => void reorder(i, -1)}
                      >
                        ↑
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Move ${f.name} down`}
                        disabled={i === sorted.length - 1 || busyId === f.id}
                        data-testid={`field-down-${f.name}`}
                        onClick={() => void reorder(i, 1)}
                      >
                        ↓
                      </Button>
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-label={`Edit ${f.name}`}
                      onClick={() => {
                        setEditing(f);
                        setDialogOpen(true);
                      }}
                    >
                      Edit
                    </Button>
                    {confirmingDelete === f.id ? (
                      <span className="flex items-center gap-1">
                        <Button
                          size="sm"
                          data-testid={`field-confirm-delete-${f.name}`}
                          onClick={() => void remove(f)}
                        >
                          Confirm remove
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => setConfirmingDelete(null)}>
                          Keep
                        </Button>
                      </span>
                    ) : (
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Remove ${f.name}`}
                        data-testid={`field-delete-${f.name}`}
                        style={{ color: "var(--m-status-danger-fg)" }}
                        onClick={() => setConfirmingDelete(f.id)}
                      >
                        Remove
                      </Button>
                    )}
                  </li>
                );
              })}
            </ol>
          )}
        </main>
      </div>
    </div>
  );
}
