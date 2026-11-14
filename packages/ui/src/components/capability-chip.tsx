"use client";
/**
 * CapabilityChip — one capability's providers on a record, drawn once.
 *
 * Purpose: implement §6.2/§6.3 of `docs/reference/connector-surface.md`. A
 * record shows one chip per *capability*, never one per brand: "RMM:
 * NinjaOne" with the other RMM in its menu, not a second chip. The chip body
 * runs the primary provider's action; the caret lists every provider with its
 * own status. With exactly one provider there is no caret and no menu, so the
 * common case is one press and no decision.
 *
 * The decisions this component owns, rather than leaving to a caller:
 *
 * 1. **No silent choice.** With several providers and no `isPrimary`, the body
 *    opens the menu and says "Choose a system". Guessing would send a
 *    technician into the wrong remote tool.
 * 2. **A degraded provider stays visible and offers a way back.** A provider
 *    whose status is not `connected` or `degraded` is not a disabled row: it
 *    reads `Fix · <name>` and calls `onFix(connectionId)`, which the surface
 *    points at that connection's detail. A missing tool reads as "we don't
 *    have that" and sends someone looking elsewhere.
 * 3. **`open` is a link, `launch` is a button.** Opening a vendor page is a
 *    navigation the browser can own (`target="_blank" rel="noreferrer"`);
 *    minting a session spends a secret, writes an audit row and needs a
 *    permission check, so it is a press the caller handles through
 *    `onLaunch`. A static href never stands in for a launch.
 * 4. **Nothing renders when there is no provider.** An empty chip is what an
 *    unlinked tool looks like; a greyed chip with an empty menu is what a demo
 *    looks like.
 *
 * The one-provider and multi-provider shapes share their height, padding and
 * radius, so adding a caret changes the chip's width and nothing else. That is
 * the "no layout shift" rule in practice: the row does not reflow its
 * neighbours when a second provider is connected.
 *
 * Non-purpose: no fetching, no URL building and no audit. The component is
 * handed resolved providers; `onLaunch` and `onFix` are the caller's domain
 * actions. A caller with no `onLaunch` gets no launch control at all rather
 * than a button that silently does nothing.
 *
 * Design research (Tettra, LaunchNotes): one visible action per item, with the
 * alternatives one press away in a picker, and the status stated beside the
 * action rather than in a separate legend. The share-dialog references
 * (ChatGPT, Relume) say the same thing from the other direction: never make
 * the user choose before they can act. So the body runs the default provider
 * and the caret is the chooser; a chip with one provider has no chooser at
 * all.
 */
import { useId, useState, type CSSProperties, type ReactNode } from "react";
import { Menu, type MenuItem } from "./menu";
import { cn } from "../lib/cn";

/** The two behaviours a chip can express (§6.4). Anything else has no chip. */
export type CapabilityActionKind = "open" | "launch";

export interface CapabilityAction {
  kind: CapabilityActionKind;
  label: string;
  /** Required for `open`; a launch is resolved by the adapter, not a URL. */
  url?: string;
}

export interface CapabilityProvider {
  connectionId: string;
  /** What the technician calls the tool: "NinjaOne", "ScreenConnect". */
  label: string;
  /** The connector slug, for a caller that needs to name it elsewhere. */
  connector: string;
  /**
   * The connection's raw status. Known values are `connected`, `degraded`,
   * `auth_failed`, `not_connected` and `disabled`; an unknown value is treated
   * as not actionable, which is the safe reading.
   */
  status: string;
  /** Exactly one provider per capability may carry this; none is allowed. */
  isPrimary?: boolean;
  actions: CapabilityAction[];
}

export interface CapabilityChipProps {
  /** Product kind: `rmm`, `remote_access`, `dms`, … (open vocabulary). */
  kind: string;
  providers: CapabilityProvider[];
  /**
   * Runs a launch. The caller owns the adapter call, the permission check and
   * the audit row; the chip never opens a session itself.
   */
  onLaunch?: (connectionId: string, actionKind: CapabilityActionKind) => void;
  /** Opens the failing connection's detail, where the fix lives. */
  onFix?: (connectionId: string) => void;
  className?: string;
}

/**
 * Short chip labels for the kinds the matrix names. A kind this map does not
 * know is humanised from its slug, because product kinds are an open
 * vocabulary (`docs/reference/connector-capabilities.md` §1).
 */
const CAPABILITY_LABEL: Record<string, string> = {
  rmm: "RMM",
  remote_access: "Remote",
  dms: "Documentation",
  documentation: "Documentation",
  network: "Network",
  identity: "Identity",
  telephony: "Phone",
  accounting: "Accounting",
  dns: "DNS",
  inventory: "Inventory",
  crm: "CRM",
  notifications: "Notifications",
};

const PROVIDER_STATUS_LABEL: Record<string, string> = {
  connected: "Connected",
  degraded: "Degraded",
  auth_failed: "Sign-in failed",
  not_connected: "Not connected",
  disabled: "Disabled",
};

export function capabilityLabel(kind: string): string {
  const known = CAPABILITY_LABEL[kind];
  if (known !== undefined) return known;
  const words = kind.replace(/[_-]+/g, " ").trim();
  if (words === "") return "Capability";
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function providerStatusLabel(status: string): string {
  return PROVIDER_STATUS_LABEL[status.trim().toLowerCase()] ?? status;
}

/** `connected` and `degraded` can act; everything else points at the fix. */
function isActionable(provider: CapabilityProvider): boolean {
  const status = provider.status.trim().toLowerCase();
  return status === "connected" || status === "degraded";
}

function firstAction(provider: CapabilityProvider): CapabilityAction | undefined {
  return provider.actions[0];
}

const GROUP_STYLE = {
  display: "inline-flex",
  alignItems: "stretch",
  height: "var(--m-density-control-sm)",
  minHeight: 32,
  maxWidth: "100%",
  border: "1px solid var(--m-border)",
  borderRadius: 999,
  background: "var(--m-surface)",
  color: "var(--m-text)",
  verticalAlign: "middle",
} as const satisfies CSSProperties;

const BODY_STYLE = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  height: "100%",
  minWidth: 0,
  padding: "0 11px",
  border: 0,
  borderRadius: 999,
  background: "transparent",
  color: "var(--m-text)",
  font: "inherit",
  fontSize: 13,
  fontWeight: 500,
  whiteSpace: "nowrap",
  textDecoration: "none",
  cursor: "pointer",
} as const satisfies CSSProperties;

export function CapabilityChip({
  kind,
  providers,
  onLaunch,
  onFix,
  className,
}: CapabilityChipProps): ReactNode {
  const [open, setOpen] = useState(false);
  const caretId = useId();

  // §6.2: a capability with no provider renders nothing at all.
  if (providers.length === 0) return null;

  const label = capabilityLabel(kind);
  const single = providers.length === 1 ? providers[0] : undefined;
  const primary = providers.find((provider) => provider.isPrimary === true);
  const multiple = providers.length > 1;

  /** The provider a one-provider chip acts on, or the resolved primary. */
  const chosen = single ?? primary;
  const active = chosen;

  const launch = (provider: CapabilityProvider, action: CapabilityAction): void => {
    onLaunch?.(provider.connectionId, action.kind);
  };

  // --- the body ------------------------------------------------------------
  let body: ReactNode;
  if (active === undefined) {
    // §6.3 rule 2: several providers, no default. The body is the way into the
    // list, and the words say so instead of naming a tool the technician did
    // not pick.
    body = (
      <button
        type="button"
        aria-label={`${label}, choose a system`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(true)}
        style={BODY_STYLE}
        data-testid="capability-chip-body"
      >
        Choose a system
      </button>
    );
  } else {
    const action = firstAction(active);
    const actionable = isActionable(active) && action !== undefined;
    const accessibleName = `${label}, ${active.label}`;

    if (
      actionable &&
      action !== undefined &&
      action.kind === "open" &&
      action.url !== undefined
    ) {
      body = (
        <a
          href={action.url}
          target="_blank"
          rel="noreferrer"
          aria-label={accessibleName}
          title={action.label}
          style={BODY_STYLE}
          data-testid="capability-chip-body"
        >
          {`${label}: ${active.label}`}
        </a>
      );
    } else if (
      actionable &&
      action !== undefined &&
      action.kind === "launch" &&
      onLaunch !== undefined
    ) {
      body = (
        <button
          type="button"
          aria-label={accessibleName}
          title={action.label}
          onClick={() => launch(active, action)}
          style={BODY_STYLE}
          data-testid="capability-chip-body"
        >
          {`${label}: ${active.label}`}
        </button>
      );
    } else if (!isActionable(active) && onFix !== undefined) {
      // A single failing provider is still the whole chip: the press opens the
      // fix, and the wording says that instead of pretending the tool works.
      body = (
        <button
          type="button"
          aria-label={`${label}, ${active.label}, needs attention`}
          onClick={() => onFix(active.connectionId)}
          style={{ ...BODY_STYLE, color: "var(--m-accent-text)" }}
          data-testid="capability-chip-body"
        >
          {`Fix · ${active.label}`}
        </button>
      );
    } else {
      // No declared action, or no handler for one. The chip still names what
      // is linked; it just does not offer a press that cannot act.
      body = (
        <span
          aria-label={accessibleName}
          title={action?.label ?? providerStatusLabel(active.status)}
          style={{ ...BODY_STYLE, cursor: "default", color: "var(--m-text-secondary)" }}
          data-testid="capability-chip-body"
        >
          {`${label}: ${active.label}`}
        </span>
      );
    }
  }

  // --- the provider menu ---------------------------------------------------
  const items: MenuItem[] = providers.map((provider) => {
    const action = firstAction(provider);
    const status = providerStatusLabel(provider.status);
    const actionable = isActionable(provider) && action !== undefined;
    const providerStatus = provider.status.trim().toLowerCase();
    const statusColour = !isActionable(provider)
      ? "var(--m-status-danger-fg)"
      : providerStatus === "degraded"
        ? "var(--m-status-warning-fg)"
        : "var(--m-text-muted)";
    const statusNode = (
      <span
        style={{
          marginLeft: 6,
          fontSize: 12,
          fontWeight: 400,
          color: statusColour,
        }}
      >
        {status}
      </span>
    );

    if (!actionable) {
      if (onFix !== undefined) {
        return {
          id: provider.connectionId,
          label: (
            <span>
              <span style={{ color: "var(--m-accent-text)", textDecoration: "underline" }}>
                {`Fix · ${provider.label}`}
              </span>
              {statusNode}
            </span>
          ),
          onSelect: () => onFix(provider.connectionId),
        };
      }
      // Nothing can act and no fix route was supplied: the row states the
      // situation, and the keyboard skips it rather than pressing air.
      return {
        id: provider.connectionId,
        label: (
          <span>
            {provider.label}
            {statusNode}
          </span>
        ),
        disabled: true,
        onSelect: () => {},
      };
    }

    if (action.kind === "open" && action.url !== undefined) {
      return {
        id: provider.connectionId,
        label: (
          <span>
            {provider.label}
            {statusNode}
          </span>
        ),
        href: action.url,
        onSelect: () => {},
      };
    }

    if (action.kind === "launch" && onLaunch !== undefined) {
      return {
        id: provider.connectionId,
        label: (
          <span>
            {provider.label}
            {statusNode}
          </span>
        ),
        onSelect: () => launch(provider, action),
      };
    }

    return {
      id: provider.connectionId,
      label: (
        <span>
          {provider.label}
          {statusNode}
        </span>
      ),
      disabled: true,
      onSelect: () => {},
    };
  });

  return (
    <span
      className={cn(className)}
      style={GROUP_STYLE}
      data-testid="capability-chip"
      data-kind={kind}
      data-providers={providers.length}
    >
      {body}
      {multiple ? (
        <Menu
          label={`${label} providers`}
          align="end"
          open={open}
          onOpenChange={setOpen}
          items={items}
          trigger={
            <button
              id={caretId}
              type="button"
              aria-label={`${label} providers`}
              style={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                width: 26,
                height: "100%",
                padding: 0,
                border: 0,
                borderLeft: "1px solid var(--m-border)",
                borderTopRightRadius: 999,
                borderBottomRightRadius: 999,
                background: "transparent",
                color: "var(--m-text-secondary)",
                cursor: "pointer",
              }}
              data-testid="capability-chip-caret"
            >
              <svg aria-hidden="true" width="10" height="10" viewBox="0 0 16 16" fill="none">
                <path
                  d="m3 6 5 5 5-5"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                />
              </svg>
            </button>
          }
        />
      ) : null}
    </span>
  );
}
