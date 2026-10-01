// /portal — the CLIENT PORTAL shell, deliberately distinct from the staff app.
//
// Purpose: a separate layout family under one URL prefix so client users never
// see staff chrome (no icon rail, no top bar, no timer bar, no workbench).
// The shell is PortalShell (components/portal/portal-shell.tsx): a 56px header
// carrying the SERVICE DESK's name, a centred 768px column, and a footer that
// credits the service desk.
//
// THE THEME IS LOCAL. The root carries `data-portal-theme`, and portal.css
// scopes both columns to it, so the client picks Sand or Dark for the portal
// without touching the staff app's Neutral.
//
// Non-purpose: no staff routes, no notification dock, no settings entry, no
// internal status anywhere below (the client-facing words live in
// lib/portal-copy.ts).
import "./portal.css";
import { PortalShell } from "../../components/portal/portal-shell";
import { PortalAuthProvider } from "../../lib/portal-api";

export default function PortalLayout({ children }: { children: React.ReactNode }) {
  return (
    <PortalAuthProvider>
      <PortalShell>{children}</PortalShell>
    </PortalAuthProvider>
  );
}
