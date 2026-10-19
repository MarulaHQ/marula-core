"use client";
/**
 * SurfaceHeader — the page frame every staff surface opens with.
 *
 * Purpose: the pack draws each page the same way (sheet S1's data-surface
 * template, and the title row on every `web/<area>/states` sheet): a
 * breadcrumb or
 * eyebrow, the title, a one-line description, a run of attention/meta pills
 * beside the title, and one primary action hard right. Until this existed,
 * 65 pages under `apps/web/app` each re-decided that frame — the title size
 * alone is currently spelled four different ways (`text-lg`, `text-xl`,
 * `text-2xl`, `text-[26px]`). This is the one place it is decided.
 *
 * Three title sizes, because the pack draws three for a page heading:
 *
 * - `hero` — the home/greeting heading the pack draws at 28px: `/today`'s
 *   "Good morning, Tumi" (`source/today.html`, `.tdh h1`) and the Knowledge
 *   home hero (`source/knowledge-vault.html`, K01 and K15). A surface that
 *   opens a whole area, rather than one list inside it, wears this step.
 * - `surface` (default) — the data-surface template. A large title at 26px
 *   with the meta pills on its baseline. This is the ledger shape
 *   (DESIGN-NOTES-tickets.md §3, "Title 26 px semibold"), and it is the one
 *   the pack's own references use.
 * - `compact` — the settings/utility shape: a smaller title over a
 *   description. `SettingsHeader` in the settings feature folder is exactly
 *   this and is now a thin alias for it, so its 33 call sites are unchanged.
 *
 * `eyebrow` sits ABOVE the title, which is where the pack puts the
 * breadcrumb on a record and where GitHub's engineering blog puts its
 * `Home / Engineering / Infrastructure` crumb — the page's position before
 * the page's name.
 *
 * Non-purpose: no data, no tabs, no fetching, no actions of its own. The
 * header renders the frame and the slots; what goes in them belongs to the
 * surface. It is also not the app's top bar — the rail, the top bar and the
 * timer bar are shell chrome and live in `apps/web/components/shell`. This is
 * the frame *inside* the page, and the two must not be confused.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export type SurfaceHeaderSize = "hero" | "surface" | "compact";

export interface SurfaceHeaderProps {
  /** The page's name. The only `h1` a staff page should contain. */
  title: ReactNode;
  /** One line under the title, `compact` only. A surface title carries pills instead. */
  description?: ReactNode;
  /**
   * Attention shortcuts and status pills, rendered on the title's baseline to
   * its right — "Changed since you looked 4", "Warranty ending 0". A pill
   * that needs colour alone is a bug: the pack requires a word too.
   */
  meta?: ReactNode;
  /** Right-aligned actions. The pack allows exactly one primary in here. */
  actions?: ReactNode;
  /** Breadcrumb or section label, above the title. */
  eyebrow?: ReactNode;
  size?: SurfaceHeaderSize;
  className?: string;
  children?: ReactNode;
}

export function SurfaceHeader({
  title,
  description,
  meta,
  actions,
  eyebrow,
  size = "surface",
  className,
  children,
}: SurfaceHeaderProps): React.JSX.Element {
  return (
    <header
      data-testid="surface-header"
      data-size={size}
      className={cn("m-surface-header", className)}
    >
      {eyebrow ? (
        <div className="m-surface-header-eyebrow" data-testid="surface-header-eyebrow">
          {eyebrow}
        </div>
      ) : null}

      <div className="m-surface-header-row">
        <div className="m-surface-header-id">
          <h1 className={cn("m-surface-header-title", `m-surface-header-title-${size}`)}>
            {title}
          </h1>
          {meta ? (
            <div className="m-surface-header-meta" data-testid="surface-header-meta">
              {meta}
            </div>
          ) : null}
        </div>
        {actions ? (
          <div className="m-surface-header-actions" data-testid="surface-header-actions">
            {actions}
          </div>
        ) : null}
      </div>

      {description ? (
        <p className="m-surface-header-description" data-testid="surface-header-description">
          {description}
        </p>
      ) : null}

      {children}
    </header>
  );
}
