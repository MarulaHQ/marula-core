// TicketCustomFields — the Details section of the ticket workspace.
//
// Purpose: render every field definition governing the ticket's board with
// a kind-appropriate input, write each change through the validated
// set/clear endpoints, and surface server validation errors inline next to
// the offending field (TASKLIST 2.17). User-reference fields render
// read-only until an org user directory exists.
// Non-purpose: defining fields (board settings own that) and no optimistic
// writes — the API verdict always wins.
"use client";

import type { components } from "@meridian/sdk";
import { Button, Input, Label, Spinner } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";

type FieldDef = components["schemas"]["FieldDefView"];
type FieldValue = components["schemas"]["TicketCustomFieldValue"];

const isChoice = (kind: string) => kind === "select" || kind === "multiselect";

/// Server errors are `{error: {message}}`; untyped when no body schema.
function errorMessage(err: unknown, fallback: string): string {
  const message = (err as { error?: { message?: string } } | undefined)?.error?.message;
  return typeof message === "string" ? message : fallback;
}

/// The stored value for one def, already canonical JSON from the API.
function valueOf(values: FieldValue[], fieldId: string): unknown | undefined {
  return values.find((v) => v.field_id === fieldId)?.value;
}

export function TicketCustomFields({
  ticketId,
  boardId,
  values,
  onChanged,
}: {
  ticketId: string;
  boardId: string;
  values: FieldValue[];
  onChanged(): void;
}) {
  const [defs, setDefs] = useState<FieldDef[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // One draft per editable def id; drafts are local until they validate.
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [savingId, setSavingId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (boardId === "") return;
    const res = await api.api.GET("/api/boards/{id}/fields", {
      params: { path: { id: boardId } },
    });
    if (res.error !== undefined) {
      setError("Could not load this board's custom fields.");
      return;
    }
    setDefs(res.data);
    // Seed drafts from stored values so text-ish inputs are controllable.
    const seeded: Record<string, string> = {};
    for (const def of res.data) {
      if (isChoice(def.kind) || def.kind === "bool") continue;
      const raw = valueOf(values ?? [], def.id);
      seeded[def.id] =
        def.kind === "multiselect" || raw === null || raw === undefined
          ? ""
          : String(raw);
    }
    setDrafts(seeded);
  }, [boardId]); // eslint-disable-line react-hooks/exhaustive-deps -- values seed drafts once per mount

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- load once per board
  }, [boardId]);

  async function put(def: FieldDef, value: unknown) {
    setSavingId(def.id);
    setFieldErrors((e) => ({ ...e, [def.id]: "" }));
    const res = await api.api.PUT("/api/tickets/{id}/fields/{field_id}", {
      params: { path: { id: ticketId, field_id: def.id } },
      body: { value: value as never },
    });
    setSavingId(null);
    if (res.error !== undefined) {
      const message = errorMessage(res.error, "This value was rejected.");
      setFieldErrors((e) => ({ ...e, [def.id]: message }));
      return;
    }
    onChanged();
  }

  async function clear(def: FieldDef) {
    setSavingId(def.id);
    const res = await api.api.DELETE("/api/tickets/{id}/fields/{field_id}", {
      params: { path: { id: ticketId, field_id: def.id } },
    });
    setSavingId(null);
    if (res.error !== undefined) {
      const message = errorMessage(res.error, "This field cannot be cleared.");
      setFieldErrors((e) => ({ ...e, [def.id]: message }));
      return;
    }
    onChanged();
  }

  if (defs === null) {
    return error === null ? (
      <Spinner />
    ) : (
      <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
        {error}
      </p>
    );
  }
  if (defs.length === 0) return null;

  return (
    <fieldset className="grid gap-3">
      <legend className="mb-1 text-xs font-medium">Details</legend>
      {[...defs]
        .sort((a, b) => (a.sort_order ?? 0) - (b.sort_order ?? 0))
        .map((def) => {
          const current = valueOf(values, def.id);
          const err = fieldErrors[def.id];
          const requiredMark = def.required ? (
            <span aria-hidden="true" style={{ color: "var(--m-status-danger-fg)" }}>
              *
            </span>
          ) : null;
          return (
            <div key={def.id} className="grid gap-1">
              <Label>
                {def.name} {requiredMark}
              </Label>
              {def.kind === "text" || def.kind === "url" || def.kind === "user" ? (
                <div className="flex items-center gap-2">
                  <Input
                    aria-label={def.name}
                    value={drafts[def.id] ?? ""}
                    readOnly={def.kind === "user"}
                    placeholder={def.kind === "url" ? "https://…" : undefined}
                    onChange={(e) =>
                      setDrafts((d) => ({ ...d, [def.id]: e.target.value }))
                    }
                  />
                  {def.kind !== "user" && (
                    <Button
                      size="sm"
                      disabled={savingId === def.id}
                      onClick={() => {
                        const draft = (drafts[def.id] ?? "").trim();
                        void draft === "" ? clear(def) : put(def, draft);
                      }}
                    >
                      Save
                    </Button>
                  )}
                </div>
              ) : null}
              {def.kind === "number" ? (
                <div className="flex items-center gap-2">
                  <Input
                    aria-label={def.name}
                    type="number"
                    step="any"
                    value={drafts[def.id] ?? ""}
                    onChange={(e) => setDrafts((d) => ({ ...d, [def.id]: e.target.value }))}
                  />
                  <Button
                    size="sm"
                    disabled={savingId === def.id}
                    onClick={() =>
                      void drafts[def.id] === "" ? clear(def) : put(def, Number(drafts[def.id]))
                    }
                  >
                    Save
                  </Button>
                </div>
              ) : null}
              {def.kind === "date" ? (
                <div className="flex items-center gap-2">
                  <Input
                    aria-label={def.name}
                    type="date"
                    value={drafts[def.id] ?? ""}
                    onChange={(e) => setDrafts((d) => ({ ...d, [def.id]: e.target.value }))}
                  />
                  <Button
                    size="sm"
                    disabled={savingId === def.id || drafts[def.id] === ""}
                    onClick={() => void put(def, drafts[def.id])}
                  >
                    Save
                  </Button>
                </div>
              ) : null}
              {def.kind === "bool" ? (
                <label className="flex items-center gap-2 text-xs">
                  <input
                    type="checkbox"
                    aria-label={def.name}
                    checked={current === true}
                    onChange={(e) => void put(def, e.target.checked)}
                  />
                  {current === true ? "Yes" : "No"}
                </label>
              ) : null}
              {def.kind === "select" ? (
                <select
                  aria-label={def.name}
                  className="rounded-[2px] border px-2 py-1.5 text-sm"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                  value={typeof current === "string" ? current : ""}
                  onChange={(e) =>
                    void e.target.value === "" ? clear(def) : put(def, e.target.value)
                  }
                >
                  <option value="">—</option>
                  {(def.options ?? []).map((o) => (
                    <option key={o} value={o}>
                      {o}
                    </option>
                  ))}
                </select>
              ) : null}
              {def.kind === "multiselect" ? (
                <div className="flex flex-wrap gap-3">
                  {(def.options ?? []).map((o) => {
                    const picked = Array.isArray(current) ? current : [];
                    const checked = picked.includes(o);
                    return (
                      <label key={o} className="flex items-center gap-1 text-xs">
                        <input
                          type="checkbox"
                          aria-label={`${def.name}: ${o}`}
                          checked={checked}
                          onChange={(e) => {
                            const next = e.target.checked
                              ? [...picked, o]
                              : picked.filter((p) => p !== o);
                            void put(def, next);
                          }}
                        />
                        {o}
                      </label>
                    );
                  })}
                </div>
              ) : null}
              {err !== undefined && err !== "" && (
                <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                  {err}
                </p>
              )}
            </div>
          );
        })}
      <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
        <span aria-hidden="true" style={{ color: "var(--m-status-danger-fg)" }}>*</span>{" "}
        required fields must have a value when the ticket is created.
      </p>
    </fieldset>
  );
}
