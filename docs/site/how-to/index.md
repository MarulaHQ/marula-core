# Admin how-to guides

Each how-to guide gives you a direct checklist for one specific job in Marula. If you are brand new and want a guided walkthrough from start to finish, begin with [Tutorials](../tutorials/index.md). If you want the reasoning behind a product rule, read [Explanation](../explanation/index.md). For field-level schemas and endpoints, see [Reference](../reference/index.md).

Every guide lists its target **Audience** (`admin`, `staff`, or `both`) right under the title so you know whether it requires administrator settings access or is part of everyday service desk work.

## Daily guides for technicians and service desk staff

- [Work a ticket](staff-work-a-ticket.md) — Open a ticket from your morning queue, run the timer, post internal or customer-visible notes, and send your hours to approvals.
- [Dispatch from the board](staff-dispatch.md) — See technician capacity across the day, drag unassigned tickets onto open calendar slots, and resolve scheduling conflicts.
- [Run a service day from the phone](technician-service-day.md) — Accept scheduled site visits, clock in and out on tickets, log mileage between stops, and work offline when you lose signal in a server room.
- [Work a day pack](staff-day-pack.md) — Review the tools and parts you need for today's scheduled stops before your van leaves the lot.
- [Check out tools and use parts on site](technician-tools-and-parts.md) — Check out tool kits, record parts used on a ticket from your phone, and return equipment when the job wraps up.
- [Move stock and check out tools in the field](technician-field-ops.md) — Scan barcodes and QR tags with your phone camera, transfer parts between your van and the stockroom, and confirm incoming shipments.
- [Use the Marula mobile app](technician-mobile-app.md) — Install the iOS or Android build, sign in with Face ID or Touch ID, and see how offline changes sync back to the office.
- [Run stock and use parts](staff-stock-and-parts.md) — Receive inventory, consume or ship parts directly from a ticket, transfer stock between locations, and set low-stock reorder alerts.
- [Check tools out, and run client loans](staff-tools-and-loans.md) — Track who has which serialized tool right now, check kits back in, and manage hardware loaned or leased to clients.
- [Scan barcodes and QR codes](staff-scanning.md) — Use a USB barcode scanner or keyboard on tickets, purchase orders, and stock screens without losing focus in your open form.
- [Log billable time with categories](staff-time-categories.md) — Pick the right work category when logging hours and record a required reason whenever work is marked non-billable.
- [Use reply snippets](admin-reply-snippets.md) — Insert canned responses into ticket replies with customer and ticket details filled in automatically.
- [Sign your own email](staff-email-signature.md) — Set up your personal greeting and email signature for ticket replies.
- [Use multiple windows](staff-multi-window.md) — Pop the dispatch board, stock lookup, or active tickets into separate windows across multiple monitors.
- [Use the keyboard shortcuts](staff-keyboard-shortcuts.md) — Hold `Alt` to see available shortcuts on any screen, press `?` for the full cheat sheet, or turn shortcuts off.
- [Use Marula with a keyboard, a screen reader, and themes](staff-accessibility.md) — Switch between light and dark or high-contrast themes, and navigate with a screen reader or keyboard alone.
- [Get help inside Marula](get-help.md) — Open the contextual `?` help drawer on any page and manage interactive feature tours.

## Guides for client portal users

- [Portal client-side tasks](client-portal-tasks.md) — Sign in to the client portal to submit support requests, view invoices, read help articles, and book appointment times.
- [Approve or decline a quote](client-quote-approval.md) — Review line items and expiration dates on a sent quote and record your approval or decline reason.

## Billing and purchasing guides (office managers and bookkeepers)

- [Price a service contract](admin-contract-types.md) — Set up recurring retainers, time-and-materials rate cards, prepaid hour blocks, not-to-exceed caps, minimums, annual escalators, change orders, and service credits.
- [Bill project work](project-billing.md) — Link a project to a client contract, track phase budget burn, release completed milestones into invoice runs, and log project hours without creating a dummy ticket.
- [Verify a billing period](admin-billing-verification.md) — Review every logged hour in a billing window, categorize or write down entries with a recorded reason, and sign off before issuing invoices.
- [Buy: vendors, purchase orders, receiving, bills](admin-purchasing.md) — Create vendor records, issue purchase orders, scan arriving goods into stock, and post vendor bills against project budgets.
- [Record FX rates & read reconciliation](admin-fx-rates.md) — Add exchange rates to the append-only rate book and review multi-currency totals by line type.

## Workspace setup and administration guides (admins and service desk leads)

- [Bootstrap your first organization](admin-org-bootstrap.md) — Create your organization on the sign-in screen and set your workspace name, ticket prefix, default timezone, and base currency.
- [Manage users and invites](admin-users-invites.md) — Invite teammates by email, set up two-factor authentication (TOTP and passkeys), and configure notification quiet hours.
- [Connect an IdP (SSO)](admin-sso.md) — Connect Google Workspace, Microsoft Entra ID, Okta, Keycloak, or any standard OIDC identity provider.
- [Set up the portal](admin-portal-setup.md) — Choose which ticket boards each client can see and create portal accounts for their contacts.
- [Set workflow categories](admin-workflow-categories.md) — Customize board columns and ticket categories while keeping SLA timers and billing tied to stable underlying stages.
- [Configure SLA calendars & escalation ladders](admin-sla-calendars.md) — Define business hours and holidays alongside priority escalation rules.
- [Author automation rules](admin-automation-rules.md) — Create event-driven rules, test them with a dry run against sample tickets, and route incoming alerts.
- [Configure email](admin-email.md) — Route inbound support emails to boards, set up SPF/DKIM/DMARC records, configure Postmark or SMTP sending, and manage organization-wide signature policies.
- [Set up CSAT surveys on resolution](admin-csat-surveys.md) — Send one-click customer satisfaction surveys when tickets close, respecting quiet hours and minimum intervals between surveys.
- [Set up labels and label templates](admin-labels.md) — Design QR and Code128 barcode labels for Zebra and DYMO printers.
- [Set up toolboxes and dispatch defaults](admin-toolboxes.md) — Group serialized tools into physical kits and auto-recommend parts on the day pack by ticket type or board.
- [Turn on travel time & mileage reconciliation](admin-routing-travel.md) — Configure drive-time estimates or live routing providers, set technician morning start points, and reimburse logged mileage.
- [Manage dashboards & report schedules](admin-dashboards-reports.md) — Build custom reports and dashboard widgets over the read model and schedule recurring email digests.
- [Run a dry-run import](admin-imports.md) — Preview CSV or JSON imports before writing a single row and save column mappings for repeat runs.
- [Customize without code](customize.md) — Add typed custom fields, reshape boards, and keep your changes safe across version upgrades.

## AI, plugins, API, and self-hosting guides (admins and developers)

- [Manage AI access](admin-ai-access.md) — Control the master AI switch, grant specific AI helpers to individual roles, and set usage caps.
- [Run transcription and embeddings on your own hardware](admin-local-ai-models.md) — Run Whisper voice transcription and 768-dimension vector search locally with Ollama so no data leaves your network.
- [Configure the plugin consent flow](admin-plugin-consent.md) — Install digest-verified plugins and review capability permissions before enabling them.
- [Extend Marula with plugin surfaces](admin-plugin-surfaces.md) — Register declarative dashboard widgets, automation triggers, read nodes, action steps, and custom field validators.
- [Manage API keys](admin-api-keys.md) — Create narrowly scoped bearer tokens for external scripts and rotate keys without downtime.
- [Call the API](call-the-api.md) — Authenticate, read paged collections, handle optimistic write envelopes, and use `@marula-core/sdk`.
- [Report against the read model](read-model.md) — Connect your BI tool or SQL client to the stable `ops.v1_*` reporting views.
- [Watch your self-hosted services](admin-self-host-services.md) — Check container health probes, inspect ClamAV email quarantines, and link out to Grafana or Uptime Kuma.
- [Run the local demo stack](local-demo-stack.md) — Boot a seeded demo on your own machine with one command, including the BYOK checklist and the desktop alternative that needs no Docker.
- [Upgrade to the latest release](upgrade.md) — Run `marula upgrade --check`, back up PostgreSQL, and upgrade safely within the N-2 support window.
- Also see the full [Reference section](../reference/index.md) for self-hosting runbooks and schema catalogs.
