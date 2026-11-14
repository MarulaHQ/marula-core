// Destination editor — §3 Destinations (bindings CRUD).
//
// Purpose: a destination is a named channel or target the operator chose, and
// the connection's own `bindings` route is the only source for the rows. The
// roster carries a count, not the rows, so this section loads them itself and
// keeps the count honest by telling the card when an enabled row appears or
// disappears.
//
// Three rules from the contract that are decided here rather than at a call
// site:
//
// - **`external_id` is not editable.** It is the destination's identity at the
//   vendor; changing it would be a new destination, and the PATCH route refuses
//   to accept one. The row shows it in mono, read-only, with the reason.
// - **A destination can be disabled without being deleted**, and a disabled
//   one stays visible here (the share dialog is what omits it).
// - **A duplicate answers 409**, and the refusal lands on the paste field that
//   caused it, in the server's own words.
//
// §3 also asks for a picker that queries the vendor's own channel list "where
// the credential allows it". No route proxies that today, so the paste field is
// the only honest path; it is not drawn as a picker that would always fail.
//
// Design research (lazyweb `lazyweb_search_screens`, 2026-11-14): GitGuardian's
// integrations screen groups rows by section and puts one toggle on each; the
// Qatalog and Knock pickers put a single labelled row per destination. The row
// below is that shape — label, identity, one enable control, one overflow — and
// the add form is a short labelled column, per Auth0's create-user modal. Tokens
// and motion stay `docs/theme.md`'s.
"use client";

import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import {
  Badge,
  Button,
  ConfirmDialog,
  FieldError,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Switch,
} from "@marula-core/ui";
import {
  createBinding,
  deleteBinding,
  listBindings,
  updateBinding,
  type BindingView,
} from "../../../lib/connectors/write";
import { bindingPatch } from "../../../lib/connectors/forms";

/** The kinds `parse_binding_kind` accepts, with what each id means. */
const BINDING_KINDS: ReadonlyArray<{ value: string; label: string; hint: string }> = [
  {
    value: "slack_channel",
    label: "Slack channel",
    hint: "The channel id from Slack (starts with C), never the #name.",
  },
  {
    value: "teams_conversation",
    label: "Microsoft Teams conversation",
    hint: "The conversation id, with the tenant id as the parent.",
  },
  {
    value: "rewst_workflow",
    label: "Rewst workflow",
    hint: "The workflow id Rewst assigned.",
  },
  {
    value: "voip_route",
    label: "Phone route",
    hint: "The route id the PBX assigned.",
  },
];

function kindLabel(kind: string): string {
  return BINDING_KINDS.find((entry) => entry.value === kind)?.label ?? kind.replace(/_/g, " ");
}

function RowsSkeleton(): ReactNode {
  return (
    <div style={{ display: "grid", gap: 10 }} aria-hidden="true">
      {[0, 1].map((row) => (
        <div
          key={row}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "10px 12px",
            border: "1px solid var(--m-border)",
            borderRadius: "var(--m-radius-container)",
          }}
        >
          <div style={{ display: "grid", gap: 6, flex: 1 }}>
            <Skeleton width="34%" />
            <Skeleton width="58%" height={10} />
          </div>
          <Skeleton shape="block" width={44} height={26} />
        </div>
      ))}
    </div>
  );
}

export function DestinationEditor({
  connectionId,
  /** Enabled destinations, so the connection card's count follows the server. */
  onCountChange,
}: {
  connectionId: string;
  onCountChange?: (enabledCount: number) => void;
}): ReactNode {
  const generated = useId();
  const prefix = `dest-${generated.replace(/[^a-zA-Z0-9_-]+/g, "")}`;
  const [bindings, setBindings] = useState<BindingView[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [kind, setKind] = useState(BINDING_KINDS[0]?.value ?? "slack_channel");
  const [label, setLabel] = useState("");
  const [externalId, setExternalId] = useState("");
  const [parent, setParent] = useState("");
  const [adding, setAdding] = useState(false);
  const [labelError, setLabelError] = useState<string | null>(null);
  const [externalError, setExternalError] = useState<string | null>(null);
  const [addFailure, setAddFailure] = useState<string | null>(null);

  const [busyId, setBusyId] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [removing, setRemoving] = useState<BindingView | null>(null);

  // The caller passes this inline, so hold it in a ref: otherwise the load
  // effect would re-run on every parent render and refetch the list in a loop.
  const countCallback = useRef(onCountChange);
  useEffect(() => {
    countCallback.current = onCountChange;
  }, [onCountChange]);
  const publishCount = useCallback((rows: BindingView[]) => {
    countCallback.current?.(rows.filter((row) => row.enabled).length);
  }, []);

  const load = useCallback(async () => {
    setLoadError(null);
    const result = await listBindings(connectionId);
    if (!result.ok) {
      setLoadError(result.failure.message);
      return;
    }
    setBindings(result.data);
    publishCount(result.data);
  }, [connectionId, publishCount]);

  useEffect(() => {
    setBindings(null);
    void load();
  }, [load]);

  const submit = async (): Promise<void> => {
    setAddFailure(null);
    setLabelError(null);
    setExternalError(null);
    if (label.trim() === "") {
      setLabelError("Give the destination a label you will recognise.");
      return;
    }
    if (externalId.trim() === "") {
      setExternalError("The vendor's own id is required — a name cannot resolve to a channel.");
      return;
    }
    setAdding(true);
    const result = await createBinding(connectionId, {
      binding_kind: kind,
      label: label.trim(),
      external_id: externalId.trim(),
      external_parent: parent.trim() === "" ? null : parent.trim(),
    });
    setAdding(false);
    if (!result.ok) {
      // 409 means this destination is already bound. It is a refusal of the
      // paste field, so it lands there rather than in a banner nobody reads.
      if (result.failure.status === 409) {
        setExternalError(result.failure.message);
      } else {
        setAddFailure(result.failure.message);
      }
      return;
    }
    const next = [...(bindings ?? []), result.data];
    setBindings(next);
    publishCount(next);
    setLabel("");
    setExternalId("");
    setParent("");
  };

  const toggle = async (binding: BindingView, enabled: boolean): Promise<void> => {
    setRowError(null);
    setBusyId(binding.id);
    const result = await updateBinding(binding.id, { enabled });
    setBusyId(null);
    if (!result.ok) {
      setRowError(result.failure.message);
      return;
    }
    const next = (bindings ?? []).map((row) => (row.id === result.data.id ? result.data : row));
    setBindings(next);
    publishCount(next);
  };

  const saveLabel = async (binding: BindingView): Promise<void> => {
    setRowError(null);
    if (editLabel.trim() === "") {
      setRowError("A destination needs a label.");
      return;
    }
    setBusyId(binding.id);
    const result = await updateBinding(
      binding.id,
      bindingPatch({
        label: editLabel,
        enabled: binding.enabled,
        external_parent: binding.external_parent,
      }),
    );
    setBusyId(null);
    if (!result.ok) {
      setRowError(result.failure.message);
      return;
    }
    setBindings((rows) =>
      (rows ?? []).map((row) => (row.id === result.data.id ? result.data : row)),
    );
    setEditingId(null);
  };

  const remove = async (binding: BindingView): Promise<void> => {
    const result = await deleteBinding(binding.id);
    if (!result.ok) {
      // The confirm dialog renders the server's refusal in place.
      throw new Error(result.failure.message);
    }
    const next = (bindings ?? []).filter((row) => row.id !== binding.id);
    setBindings(next);
    publishCount(next);
    setRemoving(null);
  };

  const labelErrorId = `${prefix}-label-error`;
  const externalIdErrorId = `${prefix}-external-error`;
  const parentHintId = `${prefix}-parent-hint`;

  return (
    <div style={{ display: "grid", gap: 12 }} data-testid="destination-editor">
      {loadError !== null ? (
        <div
          role="alert"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "10px 12px",
            borderRadius: "var(--m-radius-container)",
            background: "var(--m-status-danger-bg)",
            color: "var(--m-status-danger-fg)",
            fontSize: 12,
          }}
          data-testid="destinations-error"
        >
          <span>{loadError}</span>
          <Button size="sm" variant="secondary" onClick={() => void load()}>
            Retry
          </Button>
        </div>
      ) : bindings === null ? (
        <RowsSkeleton />
      ) : bindings.length === 0 ? (
        <p style={{ margin: 0, fontSize: 13, color: "var(--m-text-secondary)" }}>
          No destinations bound. A share from this connection is absent until one is added.
        </p>
      ) : (
        <ul style={{ display: "grid", gap: 8, listStyle: "none", margin: 0, padding: 0 }}>
          {bindings.map((binding) => {
            const isEditing = editingId === binding.id;
            return (
              <li
                key={binding.id}
                style={{
                  display: "grid",
                  gap: 6,
                  padding: "10px 12px",
                  border: "1px solid var(--m-border)",
                  borderRadius: "var(--m-radius-container)",
                  // A disabled destination is a muted surface, not a faded
                  // row: opacity would drag the label under the readability
                  // floor (docs/theme.md §6).
                  background: binding.enabled ? undefined : "var(--m-surface-2)",
                }}
                data-testid={`destination-${binding.binding_kind}`}
              >
                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                  {isEditing ? (
                    <Input
                      aria-label={`Label for ${binding.label}`}
                      value={editLabel}
                      onChange={(event) => setEditLabel(event.target.value)}
                      style={{ maxWidth: 260 }}
                    />
                  ) : (
                    <span style={{ fontSize: 13, fontWeight: 600 }}>{binding.label}</span>
                  )}
                  <Badge tone="neutral">{kindLabel(binding.binding_kind)}</Badge>
                  {!binding.enabled ? <Badge tone="offline">Disabled</Badge> : null}
                  <span style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 8 }}>
                    <Switch
                      aria-label={`${binding.enabled ? "Disable" : "Enable"} ${binding.label}`}
                      checked={binding.enabled}
                      disabled={busyId === binding.id}
                      onCheckedChange={(checked) => void toggle(binding, checked)}
                    />
                    {isEditing ? (
                      <>
                        <Button
                          size="sm"
                          variant="primary"
                          disabled={busyId === binding.id}
                          onClick={() => void saveLabel(binding)}
                        >
                          Save
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => {
                            setEditingId(null);
                            setRowError(null);
                          }}
                        >
                          Cancel
                        </Button>
                      </>
                    ) : (
                      <>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => {
                            setEditingId(binding.id);
                            setEditLabel(binding.label);
                            setRowError(null);
                          }}
                        >
                          Rename
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => setRemoving(binding)}
                          data-testid={`destination-remove-${binding.binding_kind}`}
                        >
                          Remove
                        </Button>
                      </>
                    )}
                  </span>
                </div>
                <span
                  style={{
                    fontFamily: "var(--m-font-mono)",
                    fontSize: 12,
                    color: "var(--m-text-secondary)",
                  }}
                >
                  {binding.external_id}
                  {binding.external_parent !== null && binding.external_parent !== ""
                    ? ` · ${binding.external_parent}`
                    : ""}
                </span>
                <p style={{ margin: 0, fontSize: 11, color: "var(--m-text-muted)" }}>
                  The vendor&apos;s id is fixed: changing it is a new destination.
                </p>
              </li>
            );
          })}
        </ul>
      )}

      {rowError !== null ? (
        <p
          role="alert"
          style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}
          data-testid="destination-row-error"
        >
          {rowError}
        </p>
      ) : null}

      <div
        style={{
          display: "grid",
          gap: 8,
          paddingTop: 10,
          borderTop: "1px solid var(--m-border)",
        }}
      >
        <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>Add destination</span>
        <div style={{ display: "grid", gap: 8 }}>
          <div style={{ display: "grid", gap: 4 }}>
            <Label htmlFor={`${prefix}-kind`}>Kind</Label>
            <Select value={kind} onValueChange={setKind}>
              <SelectTrigger id={`${prefix}-kind`} style={{ maxWidth: 320 }}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {BINDING_KINDS.map((entry) => (
                  <SelectItem key={entry.value} value={entry.value}>
                    {entry.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FieldHint>
              {BINDING_KINDS.find((entry) => entry.value === kind)?.hint ?? ""}
            </FieldHint>
          </div>

          <div style={{ display: "grid", gap: 4 }}>
            <Label htmlFor={`${prefix}-label`}>Label</Label>
            <Input
              id={`${prefix}-label`}
              value={label}
              placeholder="#helpdesk-alerts"
              aria-describedby={labelError !== null ? labelErrorId : undefined}
              aria-invalid={labelError !== null ? true : undefined}
              onChange={(event) => {
                setLabel(event.target.value);
                setLabelError(null);
                setAddFailure(null);
              }}
            />
            {labelError !== null ? <FieldError id={labelErrorId}>{labelError}</FieldError> : null}
          </div>

          <div style={{ display: "grid", gap: 4 }}>
            <Label htmlFor={`${prefix}-external`}>Vendor id</Label>
            <Input
              id={`${prefix}-external`}
              value={externalId}
              placeholder="C01234567"
              aria-describedby={
                externalError !== null ? `${parentHintId} ${externalIdErrorId}` : parentHintId
              }
              aria-invalid={externalError !== null ? true : undefined}
              onChange={(event) => {
                setExternalId(event.target.value);
                setExternalError(null);
                setAddFailure(null);
              }}
            />
            <FieldHint id={parentHintId}>
              The id the vendor assigned, not a label. A duplicate is refused by the server.
            </FieldHint>
            {externalError !== null ? (
              <FieldError id={externalIdErrorId}>{externalError}</FieldError>
            ) : null}
          </div>

          <div style={{ display: "grid", gap: 4 }}>
            <Label htmlFor={`${prefix}-parent`}>Workspace or tenant id (optional)</Label>
            <Input
              id={`${prefix}-parent`}
              value={parent}
              placeholder="T01234567"
              onChange={(event) => setParent(event.target.value)}
            />
          </div>

          {addFailure !== null ? (
            <p
              role="alert"
              style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}
              data-testid="destination-add-error"
            >
              {addFailure}
            </p>
          ) : null}

          <div>
            <Button
              size="sm"
              variant="primary"
              disabled={adding}
              onClick={() => void submit()}
              data-testid="destination-add"
            >
              {adding ? "Adding…" : "Add destination"}
            </Button>
          </div>
        </div>
      </div>

      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null);
        }}
        title="Remove this destination?"
        description={
          removing === null
            ? ""
            : `${removing.label} will no longer be offered in the share dialog. Messages already posted keep their audit row and stay where they were.`
        }
        confirmLabel="Remove destination"
        onConfirm={async () => {
          if (removing !== null) await remove(removing);
        }}
        testId="destination-remove-confirm"
      />
    </div>
  );
}
