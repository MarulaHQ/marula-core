// Ticket-detail pod system — registry, per-RBAC visibility, per-user state.
//
// Purpose: compose the detail view from registered pods. Two state layers:
// (1) ADMIN config — which pods each RBAC level may see (org defaults,
// stored per org); (2) USER state — hide/show + order overrides (stored
// per user). Both persist through the packages/layout panel-state engine's
// LocalStoragePanelStateStore contract (per-user keyed opaque envelopes),
// so a server adapter can replace storage without touching components.
// The registry is designed so custom pods land later as extensions: a pod
// is a plain descriptor; anything registered renders.
// Non-purpose: no permission logic here — role derivation happens once in
// `rbacLevelFor` from /me permissions and is display + config gating only;
// the API remains authoritative for every read/write.

import {
  LocalStoragePanelStateStore,
  type PanelStateStore,
} from "@meridian/layout";

// --- RBAC levels -------------------------------------------------------------

/** The org's builtin RBAC levels (valid role strings per the org-users lane). */
export type RbacLevel = "owner" | "admin" | "dispatcher" | "technician" | "read_only";

export const RBAC_LEVELS: RbacLevel[] = [
  "owner",
  "admin",
  "dispatcher",
  "technician",
  "read_only",
];

export function rbacLevelLabel(level: RbacLevel): string {
  return level.charAt(0).toUpperCase() + level.slice(1).replace("_", " ");
}

/** The org-role string carried on directory rows / invite payloads. */
export function isRbacLevel(v: string): v is RbacLevel {
  return (RBAC_LEVELS as string[]).includes(v);
}

/**
 * Derives the viewer's RBAC level from /me permissions (highest-precedence
 * first: owner/admin > dispatcher > technician > read_only). Builtin-role
 * permission sets come from core-domain (`policy.rs`); owner and admin hold
 * the IDENTICAL matrix there and /me carries permissions only, so owner
 * resolves into the admin bucket — the config matrix keeps both rows for
 * future role differentiation. This mirror only picks a config bucket —
 * never gates a request.
 */
export function rbacLevelFor(permissions: readonly string[]): RbacLevel {
  if (permissions.includes("special:manage_users")) return "admin";
  if (permissions.includes("dispatch:write")) return "dispatcher";
  if (permissions.includes("tickets:edit")) return "technician";
  return "read_only";
}

// --- Pod registry -------------------------------------------------------------

/** Every renderable pod id. Register new pods here — nothing else to wire. */
export type PodId =
  | "client-contact"
  | "next-steps"
  | "task-lists"
  | "scheduling"
  | "taxonomy";

export interface PodDescriptor {
  id: PodId;
  title: string;
  /** Whether users may hide/reorder this pod (false = always visible, fixed). */
  rearrangeable: boolean;
  /** Pods a level sees by default (admin config starts from this). */
  defaultVisible: readonly RbacLevel[];
  /** One-line description shown in the admin config matrix. */
  description: string;
}

export const PODS: readonly PodDescriptor[] = [
  {
    id: "client-contact",
    title: "Client & contact",
    rearrangeable: true,
    defaultVisible: ["owner", "admin", "dispatcher", "technician", "read_only"],
    description: "Ticket's client and contact with click-through and change pickers",
  },
  {
    id: "next-steps",
    title: "Next steps",
    rearrangeable: true,
    defaultVisible: ["owner", "admin", "dispatcher", "technician"],
    description: "Dedicated next-steps field; merges into final notes at close",
  },
  {
    id: "task-lists",
    title: "Tasks",
    rearrangeable: true,
    defaultVisible: ["owner", "admin", "dispatcher", "technician"],
    description: "Per-ticket task items with check / uncheck / not-applicable",
  },
  {
    id: "scheduling",
    title: "Scheduling",
    rearrangeable: true,
    defaultVisible: ["owner", "admin", "dispatcher", "technician"],
    description: "Send the client a scheduling link or template auto-email",
  },
  {
    id: "taxonomy",
    title: "Classification",
    rearrangeable: true,
    defaultVisible: ["owner", "admin", "dispatcher"],
    description: "Ticket type / category / subcategory taxonomy",
  },
];

export const POD_IDS: readonly PodId[] = PODS.map((p) => p.id);

// --- Persisted state -----------------------------------------------------------

/**
 * Admin layer: pod → level → visible. Absent entries fall back to the
 * registry default. Org-scoped (any admin edit affects every member of the
 * same role); stored under the org key.
 */
export type PodVisibilityConfig = Partial<Record<PodId, Partial<Record<RbacLevel, boolean>>>>;

/**
 * User layer: hidden pods + display order (ids in render order; pods not
 * listed trail in registry order). Per-user.
 */
export interface PodUserState {
  hidden: PodId[];
  order: PodId[];
}

interface PodPersistEnvelope {
  version: 1;
  admin: PodVisibilityConfig;
  user: PodUserState;
}

const PODS_NAMESPACE = "meridian.ticket-pods";

/** Panel-state-engine store instance: per-user keyed, opaque envelopes. */
const podStore: PanelStateStore = new LocalStoragePanelStateStore({
  namespace: PODS_NAMESPACE,
});

function keyFor(orgId: string, userId: string, scope: "admin" | "user"): string {
  return scope === "admin" ? `${orgId}:admin` : `${orgId}:${userId}`;
}

function emptyEnvelope(): PodPersistEnvelope {
  return { version: 1, admin: {}, user: { hidden: [], order: [] } };
}

/** Loads one scope's envelope; every failure degrades to defaults. */
async function loadEnvelope(
  orgId: string,
  userId: string,
  scope: "admin" | "user",
): Promise<PodPersistEnvelope> {
  try {
    const raw = await podStore.load(keyFor(orgId, userId, scope));
    if (raw === null) return emptyEnvelope();
    const parsed = JSON.parse(raw) as Partial<PodPersistEnvelope> | null;
    if (
      parsed === null ||
      typeof parsed !== "object" ||
      parsed.version !== 1 ||
      typeof parsed.admin !== "object" ||
      parsed.admin === null ||
      typeof parsed.user !== "object" ||
      parsed.user === null
    ) {
      return emptyEnvelope();
    }
    const admin: PodVisibilityConfig = {};
    for (const [podId, levels] of Object.entries(parsed.admin)) {
      if (!POD_IDS.includes(podId as PodId) || levels === null || typeof levels !== "object") continue;
      const clean: Partial<Record<RbacLevel, boolean>> = {};
      for (const level of RBAC_LEVELS) {
        if (typeof levels[level] === "boolean") clean[level] = levels[level];
      }
      admin[podId as PodId] = clean;
    }
    const hidden = Array.isArray(parsed.user.hidden)
      ? parsed.user.hidden.filter((id): id is PodId => POD_IDS.includes(id))
      : [];
    const order = Array.isArray(parsed.user.order)
      ? parsed.user.order.filter((id): id is PodId => POD_IDS.includes(id))
      : [];
    return { version: 1, admin, user: { hidden, order } };
  } catch {
    return emptyEnvelope();
  }
}

async function saveEnvelope(
  orgId: string,
  userId: string,
  scope: "admin" | "user",
  envelope: PodPersistEnvelope,
): Promise<void> {
  await podStore.save(keyFor(orgId, userId, scope), JSON.stringify(envelope));
}

export async function loadPodState(
  orgId: string,
  userId: string,
): Promise<{ admin: PodVisibilityConfig; user: PodUserState }> {
  const [admin, user] = await Promise.all([
    loadEnvelope(orgId, userId, "admin"),
    loadEnvelope(orgId, userId, "user"),
  ]);
  return { admin: admin.admin, user: user.user };
}

export async function saveAdminConfig(
  orgId: string,
  userId: string,
  admin: PodVisibilityConfig,
): Promise<void> {
  const envelope = await loadEnvelope(orgId, userId, "admin");
  await saveEnvelope(orgId, userId, "admin", { ...envelope, admin });
}

export async function saveUserState(
  orgId: string,
  userId: string,
  user: PodUserState,
): Promise<void> {
  const envelope = await loadEnvelope(orgId, userId, "user");
  await saveEnvelope(orgId, userId, "user", { ...envelope, user });
}

/**
 * Effective visibility for one viewer: the admin layer decides
 * (level overrides win), falling back to the registry default. The user's
 * hidden list narrows — never widens — what the admin layer allows.
 */
export function podVisibleFor(
  pod: PodDescriptor,
  level: RbacLevel,
  admin: PodVisibilityConfig,
  userHidden: readonly PodId[],
): boolean {
  const override = admin[pod.id]?.[level];
  const base = override ?? pod.defaultVisible.includes(level);
  return base && !userHidden.includes(pod.id);
}

/** Renders pods in the user's order; unlisted pods trail in registry order. */
export function orderedPods(order: readonly PodId[]): readonly PodDescriptor[] {
  const listed = order
    .map((id) => PODS.find((p) => p.id === id))
    .filter((p): p is PodDescriptor => p !== undefined);
  const rest = PODS.filter((p) => !order.includes(p.id));
  return [...listed, ...rest];
}
