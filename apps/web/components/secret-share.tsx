// Secret per-subject product panels (Wave 19 lane 3): the legacy
// password-side link management surface, the one-time share-link card,
// and the vault-tree record picker (the 0078 category-tree browser mount
// the Wave 17 entity picker could not cover — secret RECORDS are not
// part of the cross-entity search seams).
//
// Posture (docs/theme.md + mockup-v2 tokens): Card rows, Tag states, and
// the token-shown-ONCE copy-once box (the reveal posture of the vault:
// the raw token exists exactly once on the wire; the list NEVER shows
// tokens). Motion stays transform/opacity ≤120 ms; reduced-motion is
// honored by the primitives.
//
// Non-purpose: no claim rendering here — the claim surface is the
// `/share` page (apps/web/app/share/page.tsx), reachable at
// `/share?token=…` from the create response's link_path.

"use client";

import {
  Badge,
  Button,
  Card,
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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { EntityPicker, type EntityPickerProps } from "./entities/entity-picker";

type SecretShareLink = components["schemas"]["SecretShareLink"];

const EXPIRY_OPTIONS: { value: string; label: string; hours: number }[] = [
  { value: "1h", label: "1 hour", hours: 1 },
  { value: "24h", label: "24 hours", hours: 24 },
  { value: "7d", label: "7 days", hours: 24 * 7 },
];

function stateOf(
  link: SecretShareLink,
  now: Date,
): "claimed" | "revoked" | "expired" | "active" {
  if (link.consumed_at != null) return "claimed";
  if (link.revoked_at != null) return "revoked";
  if (link.expires_at != null && new Date(link.expires_at) <= now) return "expired";
  return "active";
}

const STATE_LABEL: Record<string, string> = {
  claimed: "claimed",
  revoked: "revoked",
  expired: "expired",
  active: "active",
};

/** The subject noun for idle copy (“secret record” / “credential”). */
const SUBJECT_NOUN: Record<"secret_record" | "password", string> = {
  secret_record: "secret record",
  password: "credential",
};

/// One-time share link panel for a vault subject. The create response's
/// token is displayed EXACTLY once (the copy-once box collapses to the
/// lifecycle list once dismissed).
export function SecretSharePanel({
  subjectKind,
  subjectId,
  canManage,
}: {
  subjectKind: "secret_record" | "password";
  subjectId: string;
  canManage: boolean;
}) {
  const [links, setLinks] = useState<SecretShareLink[] | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [issued, setIssued] = useState<
    components["schemas"]["SecretShareLinkIssued"] | null
  >(null);
  const [label, setLabel] = useState("");
  const [expiry, setExpiry] = useState("24h");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error: e } =
      subjectKind === "password"
        ? await api.api.GET("/api/v1/passwords/{id}/share-links", {
            params: { path: { id: subjectId } },
          })
        : await api.api.GET("/api/v1/secrets/records/{id}/share-links", {
            params: { path: { id: subjectId } },
          });
    if (e === undefined && data !== undefined) setLinks([...data]);
  }, [subjectKind, subjectId]);

  useEffect(() => {
    if (subjectId === "" || subjectId === "") return;
    void refresh();
  }, [refresh, subjectId]);

  const create = async () => {
    setBusy(true);
    setError(null);
    const hours = EXPIRY_OPTIONS.find((o) => o.value === expiry)?.hours ?? 24;
    const body = {
      label: label.trim(),
      expires_at: new Date(Date.now() + hours * 3600_000).toISOString(),
    };
    const { data, error: e } =
      subjectKind === "password"
        ? await api.api.POST("/api/v1/passwords/{id}/share-links", {
            params: { path: { id: subjectId } },
            body,
          })
        : await api.api.POST("/api/v1/secrets/records/{id}/share-links", {
            params: { path: { id: subjectId } },
            body,
          });
    setBusy(false);
    if (e !== undefined || data === undefined) {
      setError("The share link was rejected — validity must be in the future.");
      return;
    }
    setIssued({
      ...data,
      link_path: `/share?token=${data.token}`,
    });
    setLabel("");
    void refresh();
  };

  const revoke = async (shareId: string) => {
    const { error: e } =
      subjectKind === "password"
        ? await api.api.DELETE("/api/v1/passwords/{id}/share-links/{share_id}", {
            params: { path: { id: subjectId, share_id: shareId } },
          })
        : await api.api.DELETE("/api/v1/secrets/records/{id}/share-links/{share_id}", {
            params: { path: { id: subjectId, share_id: shareId } },
          });
    if (e === undefined) void refresh();
  };

  return (
    <Card>
      <div className="flex items-center justify-between gap-2 p-4 pb-2">
        <h3 className="text-sm font-semibold">One-time share links</h3>
        {canManage ? (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              setIssued(null);
              setCreateOpen(true);
            }}
          >
            Create one-time link
          </Button>
        ) : null}
      </div>
      <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        One claim reveals this {SUBJECT_NOUN[subjectKind]} to a single org user with
        reveal permission — audited, expiring, revocable.
      </p>
      {issued !== null ? (
        <div
          className="mx-4 mb-3 grid gap-1 rounded-[0.5rem] border p-3"
          style={{ borderColor: "var(--m-border)" }}
        >
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Shown once — copy it now; the server stores only a hash:
          </p>
          <div className="flex items-center gap-2">
            <Input
              readOnly
              value={issued.link_path}
              className="font-mono text-[12px]"
              aria-label="One-time share link"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void navigator.clipboard.writeText(issued.link_path)}
            >
              Copy
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setIssued(null)}>
              Done
            </Button>
          </div>
        </div>
      ) : null}
      {links === null ? (
        <div className="p-4">
          <Spinner />
        </div>
      ) : links.length === 0 ? (
        <EmptyState message="No share links yet — nothing reveals ahead of schedule." />
      ) : (
        <ul className="grid gap-1 px-4 pb-4">
          {links.map((l) => {
            const state = stateOf(l, new Date());
            return (
              <li
                key={l.id}
                className="m-entity-row flex items-center justify-between gap-2 rounded-[0.5rem] border px-3 py-2 text-[13px]"
                style={{ borderColor: "var(--m-border)" }}
              >
                <span className="grid min-w-0 gap-0.5">
                  <span className="truncate">{l.label !== "" ? l.label : "share link"}</span>
                  <span className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                    expires{" "}
                    {l.expires_at != null ? new Date(l.expires_at).toLocaleString() : "—"}
                    {l.consumed_at != null
                      ? ` · claimed ${new Date(l.consumed_at).toLocaleString()}`
                      : ""}
                  </span>
                </span>
                <span className="flex items-center gap-2">
                  <Badge tone={state === "active" ? "outline" : "neutral"}>
                    {STATE_LABEL[state]}
                  </Badge>
                  {state === "active" && canManage ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label="Revoke link"
                      onClick={() => void revoke(l.id)}
                    >
                      Revoke
                    </Button>
                  ) : null}
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {error !== null ? (
        <p role="alert" className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent
          title="Create a one-time share link"
          description="Exactly one claim reveals the material; the claiming user and instant are audited, and the link is consumed by that single reveal."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="share-label">Label (the trail's context, optional)</Label>
              <Input
                id="share-label"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder="kiosk handoff, export run…"
              />
            </div>
            <div>
              <Label htmlFor="share-expiry">Valid for</Label>
              <Select value={expiry} onValueChange={setExpiry}>
                <SelectTrigger id="share-expiry">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {EXPIRY_OPTIONS.map((o) => (
                    <SelectItem key={o.value} value={o.value}>
                      {o.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <DialogActions>
              <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <Button
                onClick={() => {
                  setCreateOpen(false);
                  void create();
                }}
                disabled={busy}
              >
                {busy ? <Spinner /> : null} Create link
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </Card>
  );
}

/// The legacy password-entry side of the SAME cross-entity link seam
/// (`/passwords/{id}/links` — the API the lane-7 deferral named), mirroring
/// the shared link-panel pattern. Mounted inside the legacy edit dialog.
export function PasswordLinksPanel({
  entryId,
  onChanged,
}: {
  entryId: string;
  onChanged?: () => void;
}) {
  const [links, setLinks] = useState<
    components["schemas"]["ResolvedLink"][] | null
  >(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const { data, error: e } = await api.api.GET("/api/v1/passwords/{id}/links", {
      params: { path: { id: entryId } },
    });
    if (e === undefined && data !== undefined) setLinks([...data]);
  }, [entryId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const add = async (
    kind: components["schemas"]["LinkKind"],
    id: string,
  ) => {
    setBusy(true);
    setError(null);
    const { error: e } = await api.api.POST("/api/v1/passwords/{id}/links", {
      params: { path: { id: entryId } },
      body: { to_kind: kind, to_id: id },
    });
    setBusy(false);
    if (e !== undefined) {
      setError(
        "The server rejected this link — the target must exist and be visible in your scope.",
      );
      return;
    }
    void refresh();
    onChanged?.();
  };

  const remove = async (linkId: string) => {
    const { error: e } = await api.api.DELETE("/api/v1/documents/links/{id}", {
      params: { path: { id: linkId } },
    });
    if (e === undefined) {
      void refresh();
      onChanged?.();
    }
  };

  return (
    <Card>
      <h3 className="p-4 pb-2 text-sm font-semibold">Links</h3>
      {links === null ? (
        <div className="p-4">
          <Spinner />
        </div>
      ) : links.length === 0 ? (
        <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Link config items, tickets, KB articles, documents, assets, or secret records to this
          credential.
        </p>
      ) : (
        <ul className="grid gap-1 px-4 pb-4">
          {links.map((l) => (
            <li
              key={l.id}
              className="m-entity-row flex items-center justify-between gap-2 rounded-[0.5rem] border px-3 py-2 text-[13px]"
              style={{ borderColor: "var(--m-border)" }}
            >
              <span className="min-w-0">
                <Badge tone="neutral">{l.to_kind.replaceAll("_", " ")}</Badge>{" "}
                {l.target_deleted ? (
                  <span style={{ color: "var(--m-status-danger-fg)" }}>deleted target</span>
                ) : (
                  <span className="truncate">{l.target_label ?? "target"}</span>
                )}
              </span>
              <Button
                variant="ghost"
                size="sm"
                aria-label="Remove link"
                onClick={() => void remove(l.id)}
              >
                ✕
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="px-4 pb-4">
        <Button
          variant="secondary"
          size="sm"
          disabled={busy}
          onClick={() => setPickerOpen(true)}
        >
          Add link — pick a target
        </Button>
      </div>
      {error !== null ? (
        <p role="alert" className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
      <EntityPicker
        open={pickerOpen}
        defaultKind="document"
        clientId={undefined}
        onPick={(t) => {
          setPickerOpen(false);
          void add(t.kind, t.id);
        }}
        onClose={() => setPickerOpen(false)}
      />
    </Card>
  );
}

/// The vault-tree picker: browse the org's secret category tree and pick
/// a native RECORD as a link target (the "secret targets need the
/// vault-tree picker" deferral from Wave 17 lane 2, closed by this lane).
export function SecretTargetPicker({
  onPick,
  onClose,
}: {
  onPick: (recordId: string, title: string, folderLabel: string) => void;
  onClose: () => void;
}) {
  const [folders, setFolders] = useState<
    components["schemas"]["SecretFolder"][] | null
  >(null);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [records, setRecords] = useState<
    components["schemas"]["SecretRecord"][] | null
  >(null);
  const loadFolders = useCallback(async () => {
    const { data, error: e } = await api.api.GET("/api/v1/secrets/folders");
    if (e === undefined && data !== undefined) setFolders([...data]);
    else setFolders([]);
  }, []);

  useEffect(() => {
    void loadFolders();
  }, [loadFolders]);

  useEffect(() => {
    if (selectedFolder === null) return;
    let live = true;
    api.api
      .GET("/api/v1/secrets/records", { params: { query: { folder_id: selectedFolder } } })
      .then(({ data, error }) => {
        if (live) setRecords(error === undefined && data !== undefined ? [...data] : []);
      });
    return () => {
      live = false;
    };
  }, [selectedFolder]);

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        title="Link a secret record"
        description="Browse the secret category trees — the target must exist and be visible in your scope."
      >
        {folders === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : (
          <div className="grid max-h-[60vh] gap-3 overflow-auto p-4">
            <Select
              onValueChange={(v) => {
                setSelectedFolder(v === "none" ? null : v);
              }}
            >
              <SelectTrigger aria-label="Category">
                <SelectValue placeholder="Pick a category…" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">— none —</SelectItem>
                {(folders ?? []).map((f) => (
                  <SelectItem key={f.id} value={f.id}>
                    {f.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {selectedFolder === null ? (
              <EmptyState message="Pick a secret category to browse its records." />
            ) : records === null ? (
              <div className="p-2">
                <Spinner />
              </div>
            ) : records.length === 0 ? (
              <EmptyState message="No records in this category — or none visible to your role." />
            ) : (
              <ul className="grid gap-1">
                {records.map((r) => {
                  const folderLabel =
                     (folders ?? []).find((f) => f.id === r.folder_id)?.label ?? "";
                  return (
                    <li
                      key={r.id}
                      className="m-entity-row flex items-center justify-between gap-2 rounded-[0.5rem] border px-3 py-2 text-[13px]"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      <span className="truncate">{r.title}</span>
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => {
                          onPick(r.id, r.title, folderLabel);
                          onClose();
                        }}
                      >
                        Link
                      </Button>
                    </li>
                  );
                })}
              </ul>
            )}
            <DialogActions>
              <Button variant="ghost" onClick={onClose}>
                Cancel
              </Button>
            </DialogActions>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
