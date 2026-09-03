// /boards/[id]/fields — custom-field definitions for one board.
//
// Purpose: the board-settings surface of dynamic forms (TASKLIST 2.17):
// define ticket fields (kind picker, option editor for choice kinds,
// required toggle, ordering), rename/reorder/re-option them, or remove
// them (stored values cascade server-side). Viewers and non-admins get a
// read-only view; the API's ConfigureBoard gate stays authoritative.
// Non-purpose: filling values on tickets (the ticket workspace owns that).
"use client";

import type { components } from "@meridian/sdk";
import { Button, Input, Label, Spinner } from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";

import { BoardNav } from "../../../../components/board-nav";
import { api, useRequireAuth } from "../../../../lib/api";

type FieldDef = components["schemas"]["FieldDefView"];
type FieldKind = NonNullable<components["schemas"]["CreateFieldDefRequest"]["kind"]>;

/// Server errors are `{error: {message}}`; untyped when no body schema.
function errorMessage(err: unknown, fallback: string): string {
  const message = (err as { error?: { message?: string } } | undefined)?.error?.message;
  return typeof message === "string" ? message : fallback;
}

const KINDS: { value: FieldKind; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "number", label: "Number" },
  { value: "date", label: "Date" },
  { value: "select", label: "Select" },
  { value: "multiselect", label: "Multi-select" },
  { value: "bool", label: "Checkbox" },
  { value: "url", label: "URL" },
  { value: "user", label: "User reference" },
];

const isChoice = (kind: string) => kind === "select" || kind === "multiselect";

export default function BoardFieldsPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const boardId = typeof params?.id === "string" ? params.id : "";

  const [defs, setDefs] = useState<FieldDef[] | null>(null);
  const [canManage, setCanManage] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Create form state.
  const [name, setName] = useState("");
  const [kind, setKind] = useState<FieldKind>("text");
  const [optionsDraft, setOptionsDraft] = useState("");
  const [required, setRequired] = useState(false);
  const [sortOrder, setSortOrder] = useState(0);
  const [formError, setFormError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (boardId === "") return;
    const res = await api.api.GET("/api/v1/boards/{id}/fields", {
      params: { path: { id: boardId } },
    });
    if (res.response.status === 403) {
      setCanManage(false);
      const readBack = res.data;
      setDefs(readBack ?? []);
      return;
    }
    if (res.error !== undefined) {
      setError("Failed to load field definitions");
      return;
    }
    setDefs(res.data);
  }, [boardId]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  async function createDef() {
    setFormError(null);
    const trimmed = name.trim();
    if (trimmed === "") {
      setFormError("Field name is required.");
      return;
    }
    const options =
      isChoice(kind)
        ? optionsDraft
            .split(",")
            .map((o) => o.trim())
            .filter((o) => o !== "")
        : [];
    if (isChoice(kind) && options.length === 0) {
      setFormError("Choice fields need at least one option.");
      return;
    }
    const res = await api.api.POST("/api/v1/boards/{id}/fields", {
      params: { path: { id: boardId } },
      body: {
        name: trimmed,
        kind,
        options,
        required,
        sort_order: sortOrder,
      },
    });
    if (res.error !== undefined) {
      const message = errorMessage(res.error, "Could not create the field.");
      setFormError(message);
      return;
    }
    setName("");
    setOptionsDraft("");
    setRequired(false);
    void refresh();
  }

  async function patchDef(fieldId: string, patch: Record<string, unknown>) {
    setError(null);
    const res = await api.api.PATCH("/api/v1/boards/{id}/fields/{field_id}", {
      params: { path: { id: boardId, field_id: fieldId } },
      body: patch,
    });
    if (res.error !== undefined) {
      setError("That change was rejected by the server.");
      return;
    }
    void refresh();
  }

  async function removeDef(fieldId: string, fieldName: string) {
    if (
      !window.confirm(
        `Delete "${fieldName}"? Every stored value for it is removed too.`,
      )
    )
      return;
    const res = await api.api.DELETE("/api/v1/boards/{id}/fields/{field_id}", {
      params: { path: { id: boardId, field_id: fieldId } },
    });
    if (res.error !== undefined) {
      setError("Delete failed.");
      return;
    }
    void refresh();
  }

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
          className="flex min-w-0 flex-1 flex-col gap-4 overflow-auto p-4"
          style={{ borderLeft: "1px solid var(--m-border)" }}
        >
          <div className="flex items-center gap-3">
            <h1 className="text-sm font-semibold">Custom fields</h1>
            <Button size="sm" variant="ghost" onClick={() => router.push(`/boards/${boardId}`)}>
              Back to board
            </Button>
          </div>

          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}

          {defs === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : (
            <>
              <ol role="list" aria-label="Field definitions" className="flex flex-col">
                {[...defs]
                  .sort((a, b) => (a.sort_order ?? 0) - (b.sort_order ?? 0))
                  .map((def) => (
                    <li key={def.id} className="row-hover grid gap-2 py-2 text-xs"
                        style={{ borderBottom: "1px solid var(--m-border)" }}>
                      <div className="grid grid-cols-[auto_1fr_auto] items-center gap-2">
                        <span className="font-medium">{def.name}</span>
                        <span style={{ color: "var(--m-text-muted)" }}>
                          {KINDS.find((k) => k.value === def.kind)?.label ?? def.kind}
                          {isChoice(def.kind) && (def.options?.length ?? 0) > 0
                            ? ` · ${def.options.join(", ")}`
                            : ""}
                        </span>
                        <label className="flex items-center gap-1">
                          <input
                            type="checkbox"
                            aria-label={`Required toggle for ${def.name}`}
                            checked={def.required ?? false}
                            disabled={!canManage}
                            onChange={(e) =>
                              void patchDef(def.id, { required: e.target.checked })
                            }
                          />
                          Required
                        </label>
                      </div>
                      {canManage && (
                        <div className="flex items-center gap-2">
                          <Input
                            aria-label={`Sort order for ${def.name}`}
                            type="number"
                            value={String(def.sort_order ?? 0)}
                            onChange={(e) =>
                              setDefs((current) =>
                                current === null
                                  ? current
                                  : current.map((d) =>
                                      d.id === def.id
                                        ? { ...d, sort_order: Number(e.target.value) }
                                        : d,
                                    ),
                              )
                            }
                            onBlur={(e) =>
                              void patchDef(def.id, { sort_order: Number(e.target.value) })
                            }
                            className="w-16"
                          />
                          {isChoice(def.kind) && (
                            <Input
                              aria-label={`Options for ${def.name} (comma separated)`}
                              value={(def.options ?? []).join(", ")}
                              placeholder="Option A, Option B…"
                              onChange={(e) =>
                                setDefs((current) =>
                                  current === null
                                    ? current
                                    : current.map((d) =>
                                        d.id === def.id ? { ...d, options: e.target.value.split(",") } : d,
                                      ),
                                )
                              }
                              onBlur={(e) =>
                                void patchDef(def.id, {
                                  options: e.target.value
                                    .split(",")
                                    .map((o) => o.trim())
                                    .filter((o) => o !== ""),
                                })
                              }
                            />
                          )}
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => void removeDef(def.id, def.name)}
                          >
                            Delete
                          </Button>
                        </div>
                      )}
                    </li>
                  ))}
              </ol>
              {defs.length === 0 && (
                <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  No custom fields defined for this board yet.
                </p>
              )}

              {canManage && (
                <section
                  aria-label="New field definition"
                  className="grid max-w-xl gap-2 rounded-[2px] border p-3"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <h2 className="text-xs font-semibold uppercase tracking-wide"
                      style={{ color: "var(--m-text-muted)" }}>
                    New field
                  </h2>
                  <div className="grid gap-2 sm:grid-cols-2">
                    <div className="grid gap-1">
                      <Label>Name</Label>
                      <Input
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="e.g. SLA tier"
                      />
                    </div>
                    <div className="grid gap-1">
                      <Label>Kind</Label>
                      <select
                        aria-label="Field kind"
                        className="rounded-[2px] border px-2 py-1.5 text-sm"
                        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                        value={kind}
                        onChange={(e) => setKind(e.target.value as FieldKind)}
                      >
                        {KINDS.map((k) => (
                          <option key={k.value} value={k.value}>
                            {k.label}
                          </option>
                        ))}
                      </select>
                    </div>
                    {isChoice(kind) && (
                      <div className="grid gap-1 sm:col-span-2">
                        <Label>Options (comma separated)</Label>
                        <Input
                          value={optionsDraft}
                          onChange={(e) => setOptionsDraft(e.target.value)}
                          placeholder="Gold, Silver, Bronze"
                        />
                      </div>
                    )}
                    <label className="flex items-center gap-2 text-xs">
                      <input
                        type="checkbox"
                        checked={required}
                        onChange={(e) => setRequired(e.target.checked)}
                      />
                      Required on every new ticket
                    </label>
                    <div className="grid gap-1">
                      <Label>Sort order</Label>
                      <Input
                        type="number"
                        value={String(sortOrder)}
                        onChange={(e) => setSortOrder(Number(e.target.value))}
                      />
                    </div>
                  </div>
                  {formError !== null && (
                    <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                      {formError}
                    </p>
                  )}
                  <div>
                    <Button variant="primary" size="sm" onClick={() => void createDef()}>
                      Add field
                    </Button>
                  </div>
                </section>
              )}
            </>
          )}
        </main>
      </div>
    </div>
  );
}
