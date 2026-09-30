// The portal shell: the client's own header, column and footer.
//
// Purpose: the client portal is a different surface for a different audience
// (the client's staff, not the MSP's), so it does not borrow the staff app's
// chrome. It has a 56px header of its own carrying the SERVICE DESK's name —
// not Marula's — a centred 768px column, and a footer that credits the service
// desk with their hours and the after-hours number.
//
// THE THEME IS THE PORTAL'S. The header root carries `data-portal-theme`
// (see lib/portal-theme.ts), which portal.css scopes: choosing Dark here
// changes this page and nothing else. The account menu says so in a sentence,
// because a claim nobody can verify is not worth making.
//
// Non-purpose: no staff nav, no notification dock, no settings, no timer bar.
// A client sees none of those things and must not be shown their ghosts.
"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { usePortalAuth } from "../../lib/portal-api";
import { portalOrgName } from "../../lib/portal-branding";
import { PORTAL_THEMES, usePortalTheme, type PortalTheme } from "../../lib/portal-theme";
import { initials } from "../../lib/portal-copy";
import { PortalIcon } from "./portal-ui";

/** The service desk's name, from deployment config, with the honest default. */
export const DESK_NAME = portalOrgName() ?? "Marula";

/** The links in the client's own order: what she came for, then help. */
const PORTAL_LINKS: readonly { href: string; label: string }[] = [
  { href: "/portal/tickets", label: "Tickets" },
  { href: "/portal/invoices", label: "Invoices" },
  { href: "/portal/quotes", label: "Quotes" },
  { href: "/portal/schedule", label: "Schedule" },
  { href: "/portal/kb", label: "Help" },
];

function NavLink({ href, label }: { href: string; label: string }) {
  const pathname = usePathname();
  // A ticket detail and the CSAT form both belong under Tickets, so the
  // section a client is standing in stays lit.
  const current =
    pathname === href ||
    pathname.startsWith(`${href}/`) ||
    (href === "/portal/tickets" && pathname === "/portal/csat");
  return (
    <Link
      href={href}
      className="portal-nav__link"
      aria-current={current ? "page" : undefined}
    >
      {label}
    </Link>
  );
}

/** The account menu, and the only place the portal's theme is picked. */
function AccountMenu() {
  const { me, signOut } = usePortalAuth();
  const router = useRouter();
  const { theme, choose } = usePortalTheme();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent) {
      if (wrapRef.current && !wrapRef.current.contains(event.target as Node)) setOpen(false);
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const name = me?.display_name ?? "Your account";

  return (
    <div className="portal-account" ref={wrapRef}>
      <button
        type="button"
        className="portal-account__button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Your account"
        onClick={() => setOpen((v) => !v)}
      >
        <span className="portal-avatar" aria-hidden>
          {initials(name)}
        </span>
        <span>{me === null ? "Account" : name.split(" ")[0]}</span>
        <PortalIcon name="down" size={16} />
      </button>
      {open && (
        <div className="portal-menu" role="menu" aria-label="Your account">
          <div>
            <b>{name}</b>
            <div className="portal-hint">
              {me === null ? "Your portal" : "Your organization's portal"}
            </div>
          </div>
          <div>
            <div className="portal-menu__label">Appearance</div>
            <div role="radiogroup" aria-label="Appearance">
              {PORTAL_THEMES.map((option: PortalTheme) => (
                <button
                  key={option}
                  type="button"
                  role="radio"
                  aria-checked={theme === option}
                  className="portal-menu__option"
                  style={{ marginBottom: 8 }}
                  onClick={() => choose(option)}
                >
                  <span style={{ flex: 1 }}>
                    <b style={{ display: "block" }}>
                      {option === "sand" ? "Sand" : "Dark"}
                    </b>
                    <span className="portal-hint">
                      {option === "sand" ? "Warm and light" : "Easy on the eyes"}
                    </span>
                  </span>
                  {theme === option && <PortalIcon name="check" size={18} />}
                </button>
              ))}
            </div>
          </div>
          <small className="portal-hint">
            This changes the portal for you only. Your service desk's own screens stay as they
            are.
          </small>
          {me !== null && (
            <button
              type="button"
              className="portal-btn portal-btn--secondary portal-btn--small"
              onClick={() => {
                signOut();
                router.replace("/portal/login");
              }}
            >
              Sign out
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function PortalHeader() {
  return (
    <header className="portal-header">
      <div className="portal-header__in">
        <Link href="/portal" className="portal-desk" aria-label={`${DESK_NAME} portal home`}>
          <span className="portal-desk__mark" aria-hidden />
          {DESK_NAME}
        </Link>
        <nav className="portal-nav" aria-label="Portal">
          {PORTAL_LINKS.map((link) => (
            <NavLink key={link.href} href={link.href} label={link.label} />
          ))}
        </nav>
        <AccountMenu />
      </div>
    </header>
  );
}

/**
 * The footer credits the service desk: who they are, their hours, the
 * after-hours number for anything urgent, and a way to reach them. It is the
 * last thing on every page and it never names Marula, because the portal
 * wears the service desk's brand (DESIGN-NOTES-portal.md §9.1).
 */
function PortalFooter() {
  return (
    <footer className="portal-footer">
      <div className="portal-footer__in">
        <span>
          <b>{DESK_NAME} Services</b> is your service desk.
          <br />
          Monday to Friday, 08:00 to 17:00. After hours, call 555 0100 for anything urgent.
        </span>
        <span style={{ flex: 1 }} />
        <a className="portal-link" href="mailto:support@brightline.example">
          Contact us
        </a>
      </div>
    </footer>
  );
}

export function PortalShell({ children }: { children: React.ReactNode }) {
  const { rootRef } = usePortalTheme();
  return (
    <div className="portal-root" ref={rootRef} data-portal-theme="sand">
      <PortalHeader />
      <main className="portal-main">{children}</main>
      <PortalFooter />
    </div>
  );
}
