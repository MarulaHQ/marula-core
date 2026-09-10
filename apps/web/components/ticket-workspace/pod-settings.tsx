// Pod configuration surfaces — admin per-RBAC visibility matrix + per-user
// hide/reorder controls.
//
// Purpose: the two state layers of the pod system, rendered. The admin
// matrix (owner/admin only) edits the org's per-level pod visibility; the
// user controls edit their own hidden/order overrides. Both persist via
// the panel-state engine contract (see pod-state.ts).
// Non-purpose: no permission enforcement — the API decides what data the
// viewer may actually read inside each pod.

"use client";

import { Button, StatusTag } from "@meridian/ui";
import { useState, type ReactNode } from "react";

import {
  PODS,
  RBAC_LEVELS,
  rbacLevelLabel,
  type PodId,
  type PodUserState,
  type PodVisibilityConfig,
  type RbacLevel,
} from "./pod-state";

export function canConfigurePods(permissions: readonly string[]): boolean {
  return permissions.includes("special:manage_users");
}

// --- Admin visibility matrix ---------------------------------------------------

export function PodAdminSettings({
  level,
  config,
  onConfigure,
  onReset,
}: {
  level: RbacLevel;
  config: PodVisibilityConfig;
  onConfigure(next: PodVisibilityConfig): void;
  onReset(): void;
}): ReactNode {
  const [open, setOpen] = useState(false);

  function toggle(podId: PodId, target: RbacLevel): void {
    const pod = PODS.find((p) => p.id === podId);
    if (pod === undefined || !pod.rearrangeable) return;
    const current = config[podId]?.[target] ?? pod.defaultVisible.includes(target);
    const next: PodVisibilityConfig = {
      ...config,
      [podId]: { ...config[podId], [target]: !current },
    };
    onConfigure(next);
  }

  if (!open) {
    return (
      <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
        Pod visibility
      </Button>
    );
  }
  return (
    <div
      className="grid gap-3 rounded-[2px] border p-3"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="pod-admin-settings"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="grid gap-0.5">
          <p className="text-xs font-semibold">Pod visibility by role</p>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Your level: {rbacLevelLabel(level)} · overrides apply to every member of the role on this device
          </p>
        </div>
        <div className="flex gap-2">
          <Button size="sm" variant="ghost" onClick={onReset}>
            Reset to defaults
          </Button>
          <Button size="sm" onClick={() => setOpen(false)}>
            Done
          </Button>
        </div>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-xs" data-testid="pod-visibility-matrix">
          <thead>
            <tr>
              <th className="p-1 text-left" style={{ color: "var(--m-text-muted)" }}>Pod</th>
              {RBAC_LEVELS.map((lvl) => (
                <th key={lvl} className="p-1 text-center font-medium" style={{ color: "var(--m-text-muted)" }}>
                  {rbacLevelLabel(lvl)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {PODS.map((pod) => (
              <tr key={pod.id} title={pod.description}>
                <td className="p-1">
                  {pod.title}
                  {!pod.rearrangeable && (
                    <span className="ml-1 align-middle">
                      <StatusTag tone="neutral">fixed</StatusTag>
                    </span>
                  )}
                </td>
                {RBAC_LEVELS.map((lvl) => {
                  const disabled = !pod.rearrangeable;
                  const visible = config[pod.id]?.[lvl] ?? pod.defaultVisible.includes(lvl);
                  return (
                    <td key={lvl} className="p-1 text-center">
                      <input
                        type="checkbox"
                        checked={visible}
                        disabled={disabled}
                        aria-label={`${pod.title} visible to ${rbacLevelLabel(lvl)}`}
                        data-testid={`pod-visibility-${pod.id}-${lvl}`}
                        onChange={() => toggle(pod.id, lvl)}
                      />
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// --- User hide / reorder controls ----------------------------------------------

export function PodUserControls({
  visiblePods,
  userState,
  onChange,
}: {
  visiblePods: { id: PodId; title: string; rearrangeable: boolean }[];
  userState: PodUserState;
  onChange(next: PodUserState): void;
}): ReactNode {
  const [open, setOpen] = useState(false);

  function hide(id: PodId): void {
    onChange({ ...userState, hidden: [...userState.hidden.filter((h) => h !== id), id] });
  }
  function restore(id: PodId): void {
    onChange({ ...userState, hidden: userState.hidden.filter((h) => h !== id) });
  }
  function move(id: PodId, dir: -1 | 1): void {
    const order = userState.order.length > 0
      ? [...userState.order]
      : visiblePods.map((p) => p.id);
    const idx = order.indexOf(id);
    const target = idx + dir;
    if (idx < 0 || target < 0 || target >= order.length) return;
    const a = order[idx] as PodId;
    const b = order[target] as PodId;
    order[idx] = b;
    order[target] = a;
    onChange({ ...userState, order });
  }

  const rearrangeable = visiblePods.filter((p) => p.rearrangeable);

  if (!open) {
    return (
      <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
        Arrange pods
      </Button>
    );
  }
  return (
    <div
      className="grid gap-2 rounded-[2px] border p-3"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="pod-user-controls"
    >
      <div className="flex items-center justify-between">
        <p className="text-xs font-semibold">Arrange your pods</p>
        <Button size="sm" onClick={() => setOpen(false)}>
          Done
        </Button>
      </div>
      <ul className="grid gap-1">
        {rearrangeable.map((pod, idx) => (
          <li key={pod.id} className="flex items-center gap-2 text-xs">
            <span className="min-w-0 flex-1 truncate">{pod.title}</span>
            <button
              type="button"
              className="rounded-[2px] border px-1.5 py-0.5"
              style={{ borderColor: "var(--m-border)" }}
              aria-label={`Move ${pod.title} up`}
              disabled={idx === 0}
              onClick={() => move(pod.id, -1)}
            >
              ↑
            </button>
            <button
              type="button"
              className="rounded-[2px] border px-1.5 py-0.5"
              style={{ borderColor: "var(--m-border)" }}
              aria-label={`Move ${pod.title} down`}
              disabled={idx === rearrangeable.length - 1}
              onClick={() => move(pod.id, 1)}
            >
              ↓
            </button>
            <button
              type="button"
              className="rounded-[2px] border px-1.5 py-0.5"
              style={{ borderColor: "var(--m-border)" }}
              aria-label={`Hide ${pod.title}`}
              onClick={() => hide(pod.id)}
            >
              Hide
            </button>
          </li>
        ))}
      </ul>
      {userState.hidden.length > 0 && (
        <div className="grid gap-1 border-t pt-2" style={{ borderColor: "var(--m-border)" }}>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>Hidden</p>
          <div className="flex flex-wrap gap-1">
            {userState.hidden.map((id) => (
              <button
                key={id}
                type="button"
                className="rounded-[2px] border px-1.5 py-0.5 text-xs"
                style={{ borderColor: "var(--m-border)" }}
                onClick={() => restore(id)}
              >
                {visiblePods.find((p) => p.id === id)?.title ?? id} + show
              </button>
            ))}
          </div>
        </div>
      )}
      <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        Saved for your account on this device
      </p>
    </div>
  );
}
