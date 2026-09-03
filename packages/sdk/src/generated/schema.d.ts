export interface paths {
    "/api/v1/agent-proposals": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/agent-proposals — cursor-paged listing, newest first. */
        get: operations["agent_proposals_list"];
        put?: never;
        /**
         * POST /api/agent-proposals — file a proposal (IF8). Accepts human-JWT
         *     callers for administrative simulation, but the API-key/machine path is
         *     the intended caller: the machine principal's key id becomes the
         *     agent_ref when none is supplied. The dial position is captured from
         *     the capability's grant row at creation time.
         */
        post: operations["agent_proposals_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/agent-proposals/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/agent-proposals/{id}. */
        get: operations["agent_proposals_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/agent-proposals/{id}/approve": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/agent-proposals/{id}/approve — decide FOR. Approved ticket
         *     update proposals apply immediately through the IF1 instant-first path
         *     in the SAME transaction (idempotency-keyed, version-gated, write-trail
         *     row — stays undoable); other entity/op combinations stay approved-
         *     pending until the write-execution lanes widen application.
         */
        post: operations["agent_proposals_approve"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/agent-proposals/{id}/decline": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/agent-proposals/{id}/decline — decide AGAINST. A declined
         *     proposal produces NO write at all; the row is the audit record.
         */
        post: operations["agent_proposals_decline"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/agent-proposals/{id}/withdraw": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/agent-proposals/{id}/withdraw — the proposing agent (or an
         *     admin) cancels its own open proposal. No write ever happened.
         */
        post: operations["agent_proposals_withdraw"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/agent-skills": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/agent-skills — latest version per slug (or one slug's history). */
        get: operations["agent_skills_list"];
        put?: never;
        /** POST /api/v1/agent-skills — publish a NEW version of a skill. */
        post: operations["agent_skills_publish"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/agent-skills/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/agent-skills/{id} — one immutable version. */
        get: operations["agent_skills_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/alert-sources": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/alert-sources — newest-first config list (no secrets). */
        get: operations["alert_sources_list"];
        put?: never;
        /**
         * POST /api/alert-sources — creates an ingest source, generates + seals
         *     its HMAC secret (returned ONCE), and writes the audit trail.
         *     Instant-first (IF1/IF2): idempotency-keyed, read-your-write response.
         */
        post: operations["alert_sources_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/alert-sources/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/alert-sources/{id} — re-point the board, rename, resize the
         *     storm window, or disable. Instant-first (IF1/IF2).
         */
        patch: operations["alert_sources_update"];
        trace?: never;
    };
    "/api/v1/alerts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/alerts — newest-first ledger browse, filterable by source and
         *     incident ticket (staff, ticket-view scoped).
         */
        get: operations["alerts_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/alerts/ingest/{source_ref}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/alerts/ingest/{source_ref} — HMAC-verified alert ingest.
         * @description Responses: 200 with a terminal `status` (incident_created / collapsed /
         *     duplicate); 401 missing/bad signature; 403 disabled source; 404 unknown
         *     source_ref; 422 malformed payload.
         */
        post: operations["alerts_ingest"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/api-keys": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/api-keys — listing without secrets. */
        get: operations["api_keys_list"];
        put?: never;
        /** POST /api/api-keys — creates a key and returns the raw secret ONCE. */
        post: operations["api_keys_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/api-keys/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/api-keys/{id} — immediate revocation (rotation story:
         *     create replacement → revoke old).
         */
        delete: operations["api_keys_revoke"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/appointments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/appointments — create a booking (409 on overlap). */
        post: operations["appointment_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/appointments/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/appointments/{id} — fetch one booking in the caller's org. */
        get: operations["appointment_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/appointments/{id} — edit fields/status (times unchanged;
         *     moves use the dedicated endpoint so clients get rescheduled events).
         */
        patch: operations["appointment_patch"];
        trace?: never;
    };
    "/api/v1/appointments/{id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/appointments/{id}/cancel — frees the slot (status flip, never
         *     deletion, so history survives).
         */
        post: operations["appointment_cancel"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/appointments/{id}/move": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/appointments/{id}/move — reschedule with re-validation
         *     (dry-run semantics inline: a conflicting move is rejected atomically
         *     and emits a `rescheduled` event only when it lands).
         */
        post: operations["appointment_move"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/attachments/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/attachments/{id} — author-gated (the uploader, or an org
         *     owner/admin). The stored object is reclaimed only when no sibling row
         *     still references the same content-addressed key.
         */
        delete: operations["attachments_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/attachments/{id}/download": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/attachments/{id}/download — streams the stored bytes through
         *     the API behind board-view authz (no public URLs in v1). Integrity is
         *     verifiable against the recorded sha256 (`x-meridian-sha256` header).
         */
        get: operations["attachments_download"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/audit-events": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/audit-events — append-only audit trail, newest first, filterable
         *     by actor / action prefix / time range with opaque cursor pagination.
         */
        get: operations["audit_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/login": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/login — resolves the org by email (SECURITY DEFINER
         *     bridge), verifies the argon2 hash under RLS scope. When any second
         *     factor is enrolled, returns a short-lived `mfa_pending` bridge instead
         *     of tokens; POST /api/auth/mfa/verify completes it.
         */
        post: operations["handlers_login"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/logout": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/auth/logout — revokes the refresh session and audits the event. */
        post: operations["handlers_logout"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/recovery-codes/regenerate": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/recovery-codes/regenerate — invalidates all previous
         *     codes, issues a new batch (revealed once).
         */
        post: operations["mfa_recovery_regenerate"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/totp/disable": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/totp/disable — requires password AND a live factor;
         *     wipes TOTP enrollment and all recovery codes.
         */
        post: operations["mfa_totp_disable"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/totp/enroll-begin": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/totp/enroll-begin — generates a secret (returned ONCE)
         *     and stores it sealed+unconfirmed. Re-calling restarts enrollment safely.
         */
        post: operations["mfa_totp_enroll_begin"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/totp/enroll-verify": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/totp/enroll-verify — confirms the enrollment with the
         *     first valid code and issues the one-time recovery-code batch.
         */
        post: operations["mfa_totp_enroll_verify"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/verify": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/verify — completes an MFA-gated login with a TOTP or
         *     recovery code. Tries TOTP first (advancing the replay watermark), then
         *     falls back to single-use recovery codes.
         */
        post: operations["mfa_verify"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/webauthn/begin": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/webauthn/begin — assertion request for the pending
         *     MFA login. Requires a valid mfa_pending token.
         */
        post: operations["mfa_webauthn_login_begin"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/webauthn/credentials": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/auth/mfa/webauthn/credentials — list registered passkeys. */
        get: operations["mfa_webauthn_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/webauthn/credentials/{id}/remove": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/webauthn/credentials/{id}/remove — password-gated
         *     removal of one passkey.
         */
        post: operations["mfa_webauthn_remove"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/webauthn/finish": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/webauthn/finish — verifies the assertion and issues
         *     full tokens. Ceremony state is single-use; replay of a finish call fails.
         */
        post: operations["mfa_webauthn_login_finish"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/webauthn/register/begin": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/webauthn/register/begin — starts a passkey
         *     registration for the authenticated user.
         */
        post: operations["mfa_webauthn_register_begin"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/mfa/webauthn/register/finish": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/mfa/webauthn/register/finish — verifies the attestation
         *     and stores the passkey. The public key is stored; no secret exists.
         */
        post: operations["mfa_webauthn_register_finish"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/oidc/providers": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/auth/oidc/providers — admin listing. Secrets excluded by type. */
        get: operations["oidc_providers_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/oidc/providers/public": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/auth/oidc/providers/public — drives config-driven SSO buttons
         *     on the login screen. No configuration detail beyond names.
         */
        get: operations["oidc_providers_public"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/oidc/providers/{name}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /**
         * PUT /api/auth/oidc/providers/{name} — create or replace a provider.
         *     The client secret is required on every write (rotate-friendly).
         */
        put: operations["oidc_provider_upsert"];
        post?: never;
        /** DELETE /api/auth/oidc/providers/{name}. */
        delete: operations["oidc_provider_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/oidc/{name}/callback": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/auth/oidc/{name}/callback — validates state signature/expiry/
         *     provider binding and nonce echo, runs the conservative decision, then
         *     hands tokens (or link confirmation) to the web app via a single-use code.
         */
        get: operations["oidc_callback"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/oidc/{name}/link": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/oidc/{name}/link — begins the EXPLICIT account-link flow
         *     for the current user. Returns the authorize URL to navigate to.
         */
        post: operations["oidc_link_begin"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/oidc/{name}/login": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/auth/oidc/{name}/login — 303 to the IdP with signed state,
         *     nonce, and PKCE S256 bound together.
         */
        get: operations["oidc_login_redirect"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/refresh": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/refresh — rotates the refresh secret as a NEW chain node.
         *     Presenting an already-consumed (rotated/revoked/expired) token proves
         *     replay: the WHOLE chain is revoked and the event is audited.
         */
        post: operations["handlers_refresh"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/register": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/register — creates the org, its default roles/board/SLA
         *     policy, and the first user as Owner. One transaction: a half-created org
         *     can never exist.
         */
        post: operations["handlers_register"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/sessions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/auth/sessions — the caller's live refresh sessions (security
         *     settings page).
         */
        get: operations["auth_sessions_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/sessions/{id}/revoke": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/sessions/{id}/revoke — kills one of the caller's own
         *     sessions (sign-out-everywhere-else primitive).
         */
        post: operations["auth_session_revoke"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/automation-rules": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/automation-rules */
        get: operations["automation_rules_list"];
        put?: never;
        /**
         * POST /api/automation-rules — documents are validated through core-domain
         *     BEFORE storage, so stored rules are always well-formed (malformed rules
         *     can only arrive out-of-band and are skipped at evaluation time).
         */
        post: operations["automation_rules_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/automation-rules/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** DELETE /api/automation-rules/{id} */
        delete: operations["automation_rules_delete"];
        options?: never;
        head?: never;
        /** PATCH /api/automation-rules/{id} */
        patch: operations["automation_rules_update"];
        trace?: never;
    };
    "/api/v1/availability": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/availability — list one user's weekly templates. */
        get: operations["availability_list"];
        put?: never;
        /** POST /api/availability — add a weekly template for a user. */
        post: operations["availability_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/availability/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** DELETE /api/availability/{id}?user_id= — remove one template. */
        delete: operations["availability_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/billing/invoice-runs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/billing/invoice-runs — most recent runs first. */
        get: operations["billing_invoice_runs_list"];
        put?: never;
        /**
         * POST /api/billing/invoice-runs — evaluate every billable contract over
         *     the window (optionally one client's) and persist one invoice per
         *     contract with at least one candidate line. Idempotency-keyed (IF1):
         *     the same key with the same body replays the stored run.
         */
        post: operations["billing_invoice_runs_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/billing/invoice-runs/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/billing/invoice-runs/{id} — one run's roll-up. */
        get: operations["billing_invoice_runs_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/billing/preview": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/billing/preview — evaluate EVERY billable contract in the
         *     org over `[from, to]` (cancelled contracts excluded; effective windows
         *     must intersect). READ-ONLY: the org-wide dry-run of a Wave-9 invoice
         *     run.
         */
        get: operations["billing_preview"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/board-groups": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/board-groups — the org's groups in nav order (any member may
         *     read; rendering input for the tree).
         */
        get: operations["board_groups_list"];
        put?: never;
        /**
         * POST /api/board-groups — creates a nav group (ManageBoards gated);
         *     emits `board.group_created`.
         */
        post: operations["board_groups_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/board-groups/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/board-groups/{id} — removes a group; member boards become
         *     ungrouped, never deleted. ManageBoards OR the group's owner; emits
         *     `board.group_removed`.
         */
        delete: operations["board_groups_delete"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/board-groups/{id} — renames/reorders a group. ManageBoards
         *     OR the group's owner may do this; emits `board.group_updated`.
         */
        patch: operations["board_groups_update"];
        trace?: never;
    };
    "/api/v1/boards": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/boards */
        get: operations["boards_list"];
        put?: never;
        /** POST /api/boards — creates a board seeded with the four default statuses. */
        post: operations["boards_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/favorite": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/boards/favorite — toggles a board favorite for the current
         *     user (user-scoped; any authenticated member may star boards they can
         *     see). Emits `board.favorite_toggled` in-tx.
         */
        post: operations["boards_toggle_favorite"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/nav": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/boards/nav — every org board annotated with the current
         *     user's favorite mark, board role, and group assignment.
         */
        get: operations["boards_nav"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/checklist-template": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/boards/{id}/checklist-template */
        get: operations["boards_get_checklist_template"];
        /**
         * PUT /api/boards/{id}/checklist-template — creates or replaces the
         *     board's template wholesale and sets the enforcement toggle; emits
         *     `board.checklist_template_updated` and audits the change.
         */
        put: operations["boards_put_checklist_template"];
        post?: never;
        /**
         * DELETE /api/boards/{id}/checklist-template — removes the template.
         *     Ticket instances demote to ad-hoc items; with no template left,
         *     close-time enforcement is off by construction regardless of the toggle.
         */
        delete: operations["boards_delete_checklist_template"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/fields": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/boards/{id}/fields — active ticket definitions governing this
         *     board (org-wide plus board-owned), sort-order stable.
         */
        get: operations["fields_list_for_board"];
        put?: never;
        /**
         * POST /api/boards/{id}/fields — creates a board-scoped definition
         *     (board-admin gated); emits `custom_field.def_created`.
         */
        post: operations["fields_create_def"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/fields/{field_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/boards/{id}/fields/{field_id} — removes the definition and
         *     cascades its stored values; emits `custom_field.def_removed`.
         */
        delete: operations["fields_delete_def"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/boards/{id}/fields/{field_id} — partial edit of a board-owned
         *     definition. Kind is immutable (it would orphan stored values); emit
         *     `custom_field.def_updated` on success.
         */
        patch: operations["fields_update_def"];
        trace?: never;
    };
    "/api/v1/boards/{id}/group": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /**
         * PUT /api/boards/{id}/group — assigns/unassigns the board's nav group.
         *     Board-admin gated (ConfigureBoard); emits `board.group_updated`.
         */
        put: operations["boards_assign_group"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/members": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/boards/{id}/members — the roster (ConfigureBoard gated). */
        get: operations["boards_list_members"];
        /**
         * PUT /api/boards/{id}/members — adds or re-roles one membership.
         *     ManageMembers gated (board admin); emits `board.membership_updated`.
         */
        put: operations["boards_upsert_member"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/members/{user_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/boards/{id}/members/{user_id} — removes one membership;
         *     the user falls back to org-governed access. Emits
         *     `board.membership_removed`.
         */
        delete: operations["boards_remove_member"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/statuses": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/boards/{id}/statuses */
        get: operations["boards_list_statuses"];
        put?: never;
        /**
         * POST /api/boards/{id}/statuses — adds a custom status bound to a
         *     semantic category; emits `board.status_added`.
         */
        post: operations["boards_add_status"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/boards/{id}/statuses/{status_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/boards/{id}/statuses/{status_id} — removes an UNUSED custom
         *     status. Tickets referencing it force a 409; the last entry status of a
         *     board is protected so ticket creation can never break.
         */
        delete: operations["boards_delete_status"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/boards/{id}/statuses/{status_id} — rebinds/reorders/renames a
         *     status; emits `board.status_updated`.
         */
        patch: operations["boards_update_status"];
        trace?: never;
    };
    "/api/v1/boards/{id}/transitions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/boards/{id}/transitions — the board's effective workflow. */
        get: operations["boards_list_transitions"];
        /**
         * PUT /api/boards/{id}/transitions — atomically replaces the explicit
         *     transition matrix; emits `board.transitions_updated`. Endpoints must be
         *     statuses of THIS board and self-transitions are rejected.
         */
        put: operations["boards_set_transitions"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/calendar-exclusions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/calendar-exclusions?user_id=&from=&to= */
        get: operations["calendar_exclusion_list"];
        put?: never;
        /** POST /api/calendar-exclusions — record time-off. */
        post: operations["calendar_exclusion_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/calendar-exclusions/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** DELETE /api/calendar-exclusions/{id}?user_id= */
        delete: operations["calendar_exclusion_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/calendar/day": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/calendar/day — one local day for one user. */
        get: operations["calendar_day"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/calendar/team": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/calendar/team — every org member over an absolute UTC range. */
        get: operations["calendar_team"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/calendar/week": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/calendar/week — Monday-start local week for one user. */
        get: operations["calendar_week"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/clients */
        get: operations["clients_list"];
        put?: never;
        /**
         * POST /api/clients — instant-first (IF1/IF2): idempotency-keyed and
         *     returns the read-your-write `mutation` metadata.
         */
        post: operations["clients_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/clients/{id} */
        get: operations["clients_get_one"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients/{id}/assets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/clients/{id}/assets */
        get: operations["clients_list_assets"];
        put?: never;
        /** POST /api/clients/{id}/assets */
        post: operations["clients_create_asset"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients/{id}/board-exposures": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/clients/{id}/board-exposures — every configured row for the
         *     client (exposed or not), so admin UI can render toggles.
         */
        get: operations["board_exposures_list"];
        /** PUT /api/clients/{id}/board-exposures — upsert one board×client row. */
        put: operations["board_exposures_put"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients/{id}/contacts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/clients/{id}/contacts */
        get: operations["clients_list_contacts"];
        put?: never;
        /** POST /api/clients/{id}/contacts */
        post: operations["clients_create_contact"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients/{id}/portal-users": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/clients/{id}/portal-users — directory for one client. */
        get: operations["portal_users_list"];
        put?: never;
        /** POST /api/clients/{id}/portal-users — provision (invite) one identity. */
        post: operations["portal_users_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/clients/{id}/sites": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/clients/{id}/sites */
        get: operations["clients_list_sites"];
        put?: never;
        /** POST /api/clients/{id}/sites */
        post: operations["clients_create_site"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/contracts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/contracts — list with optional client/status/effective-date
         *     filters, ordered by contract number.
         */
        get: operations["contracts_list"];
        put?: never;
        /**
         * POST /api/contracts — instant-first (IF1/IF2): idempotency-keyed and
         *     returns the read-your-write `mutation` metadata.
         */
        post: operations["contracts_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/contracts/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/contracts/{id} — the contract aggregate (agreement + rate
         *     card + fee schedule).
         */
        get: operations["contracts_get_one"];
        put?: never;
        post?: never;
        /**
         * DELETE /api/contracts/{id} — removes the contract aggregate. A typed
         *     404 (`rls_sparse`) means the target was never visible in scope.
         *     The Wave 9 invoicing lane's billing-lock guard refuses the delete
         *     once invoices reference the contract (typed 409).
         */
        delete: operations["contracts_delete"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/contracts/{id} — instant-first (IF1/IF2) partial update.
         *     Absent fields keep their values; `rates`/`fees`, when present,
         *     replace the whole card/schedule. The version gate rejects stale
         *     writes with a typed 409.
         */
        patch: operations["contracts_patch"];
        trace?: never;
    };
    "/api/v1/contracts/{id}/billing-preview": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/contracts/{id}/billing-preview — evaluate this contract's
         *     billing rules over `[from, to]` and return the draft invoice-line
         *     candidates. READ-ONLY: nothing persists, no billed markers exist.
         */
        get: operations["contracts_billing_preview"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/dispatch/assign": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/dispatch/assign — place an unassigned ticket on a
         *     technician's board: creates the booking AND flips the ticket
         *     assignee in one transaction.
         */
        post: operations["dispatch_assign"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/dispatch/bulk-move": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/dispatch/bulk-move — atomic batch reschedule.
         * @description Every move is re-validated against the TARGET user's live bookings
         *     in the union window (each moved booking vacates its old slot, so
         *     swaps are legal); the first violating move rejects the WHOLE batch
         *     with a typed 409 naming the move index and blocking booking.
         */
        post: operations["dispatch_bulk_move"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/dispatch/day": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/dispatch/day — the whole board for one day. */
        get: operations["dispatch_day"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/documents": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/documents */
        get: operations["documents_list"];
        put?: never;
        /**
         * POST /api/documents — instant-first (IF1/IF2); creates the document and
         *     its immutable version 1 in one commit.
         */
        post: operations["documents_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/documents/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/documents/{id} */
        get: operations["documents_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/documents/{id} — instant-first (IF1/IF2); an edit APPENDS
         *     the next immutable version and moves the document's head to it. No
         *     existing version row is touched.
         */
        patch: operations["documents_update"];
        trace?: never;
    };
    "/api/v1/documents/{id}/restore": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/documents/{id}/restore — instant-first (IF1/IF2); §6 rule:
         *     the restore APPENDS a new version copying the target's content. The
         *     target's title comes along, renames after it are not silently undone.
         */
        post: operations["documents_restore"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/documents/{id}/versions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/documents/{id}/versions — the full immutable history, newest
         *     first, complete bodies included so any pair is diffable.
         */
        get: operations["documents_list_versions"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/documents/{id}/versions/{version}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/documents/{id}/versions/{version} — one immutable snapshot. */
        get: operations["documents_get_version"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/batches": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/imports/batches */
        get: operations["imports_list_batches"];
        put?: never;
        /** POST /api/imports/batches */
        post: operations["imports_create_batch"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/batches/{batch_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/imports/batches/{id} */
        get: operations["imports_get_batch"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/batches/{batch_id}/chunks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/imports/batches/{id}/chunks — plans then executes ONE chunk
         *     atomically. A fail-policy chunk with any conflict persists nothing and
         *     returns 409 with the full per-row report.
         */
        post: operations["imports_post_chunk"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/batches/{batch_id}/finish": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/imports/batches/{id}/finish — closes the span, emits
         *     `import.batch_finished`, and records cumulative counters in audit.
         */
        post: operations["imports_finish_batch"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/batches/{batch_id}/run": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/imports/batches/{id}/run — hands the staged batch to the
         *     worker queue exactly once (generalized 2.7 job queue). Re-running a
         *     finished or queued batch is a typed 409; the worker replays chunks
         *     idempotently through the SAME commit path as synchronous pushes.
         */
        post: operations["imports_run_batch"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/batches/{batch_id}/stage": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/imports/batches/{id}/stage — uploads ONE chunk of rows to the
         *     ObjectStore under a deterministic key WITHOUT executing anything. An
         *     optional saved template is applied at stage time so the stored chunk is
         *     already canonical; the worker later replays the staged sequence.
         */
        post: operations["imports_stage_chunk"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/dry-run": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/imports/dry-run */
        post: operations["imports_dry_run"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/templates": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/imports/templates */
        get: operations["imports_list_templates"];
        put?: never;
        /**
         * POST /api/imports/templates — saves one reusable source→canonical column
         *     mapping. Targets are validated against the canonical field catalog for
         *     the entity kind; duplicates per (org, source system, kind, name) are a
         *     typed 409.
         */
        post: operations["imports_create_template"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/imports/templates/{template_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/imports/templates/{id} */
        get: operations["imports_get_template"];
        /**
         * PUT /api/imports/templates/{id} — content update; uniqueness conflicts
         *     (renaming onto an existing sibling) surface as typed 409s.
         */
        put: operations["imports_update_template"];
        post?: never;
        /** DELETE /api/imports/templates/{id} */
        delete: operations["imports_delete_template"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/internal/effects": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/internal/effects — applies automation-rule effects to one
         *     ticket through the SAME write paths users go through (ticket patch,
         *     comment insert, notification fan-out), emitting every corresponding
         *     domain event tagged `via_automation` so evaluators never loop.
         */
        post: operations["internal_effects"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/internal/sla/verdict": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/internal/sla/verdict — fire-once application of one ladder
         *     step: ledger insert (PK-guarded), breach flip, notification fan-out with
         *     preference gates, events + audit, all in ONE transaction.
         */
        post: operations["internal_sla_verdict"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/invoices": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/invoices — list with client/contract/status/dunning filters. */
        get: operations["invoices_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/invoices/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/invoices/{id} — the invoice aggregate (document, lines,
         *     payment receipts).
         */
        get: operations["invoices_get_one"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/invoices/{id}/document": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/invoices/{id}/document — the print-ready HTML invoice
         *     document (print-to-PDF from any browser; v1 ships no PDF binary
         *     output — see TASKLIST).
         */
        get: operations["invoices_document"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/invoices/{id}/dunning": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/invoices/{id}/dunning — move an ISSUED invoice's dunning
         *     state forward (none → reminder_sent → overdue → final_notice →
         *     collections; skipping rungs is allowed, backwards is refused).
         *     States are data: the write emits `invoice.dunning_changed` and lands
         *     in the write trail. Instant-first gated (IF1/IF2).
         */
        post: operations["invoices_dunning_transition"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/invoices/{id}/payments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/invoices/{id}/payments — the payment receipts. */
        get: operations["invoices_payments_list"];
        put?: never;
        /**
         * POST /api/invoices/{id}/payments — record a payment through the
         *     `PaymentProvider` seam. The charge runs INSIDE the mutation
         *     transaction so receipt, payment row, and invoice status commit (or
         *     roll back) atomically. Instant-first gated (IF1/IF2).
         */
        post: operations["invoices_payments_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/invoices/{id}/void": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/invoices/{id}/void — withdraw an ISSUED invoice. Terminal:
         *     consumption markers stay, so re-running the window can never
         *     double-bill. Instant-first gated (IF1/IF2).
         */
        post: operations["invoices_void"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/kb/articles": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/kb/articles */
        get: operations["kb_list_articles"];
        put?: never;
        /** POST /api/kb/articles — instant-first (IF1/IF2). */
        post: operations["kb_create_article"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/kb/articles/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/kb/articles/{id} */
        get: operations["kb_get_article"];
        put?: never;
        post?: never;
        /**
         * DELETE /api/kb/articles/{id} — emits `kb_article.deleted` so the
         *     unified-search indexer drops the article's index entry.
         */
        delete: operations["kb_delete_article"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/kb/articles/{id} — instant-first (IF1/IF2). Archived
         *     articles refuse title/body edits until unarchived (typed conflict);
         *     status flips stay legal.
         */
        patch: operations["kb_update_article"];
        trace?: never;
    };
    "/api/v1/kb/categories": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/kb/categories */
        get: operations["kb_list_categories"];
        put?: never;
        /** POST /api/kb/categories — instant-first (IF1/IF2). */
        post: operations["kb_create_category"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/kb/categories/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/kb/categories/{id} — articles keep existing (category_id
         *     clears; ON DELETE SET NULL), so no content is lost behind a removal.
         */
        delete: operations["kb_delete_category"];
        options?: never;
        head?: never;
        /** PATCH /api/kb/categories/{id} — instant-first (IF1/IF2). */
        patch: operations["kb_update_category"];
        trace?: never;
    };
    "/api/v1/mcp/capabilities": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/mcp/capabilities — every granted capability of the org. */
        get: operations["mcp_capabilities_list"];
        /** PUT /api/v1/mcp/capabilities — grant or re-dial one capability. */
        put: operations["mcp_capability_put"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/mcp/capabilities/{capability}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** DELETE /api/v1/mcp/capabilities/{capability} — revoke (deny again). */
        delete: operations["mcp_capability_revoke"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/me": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/me — the authenticated principal with resolved permissions. */
        get: operations["handlers_me"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notification-preferences": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/notification-preferences — the caller's effective preference
         *     for every kind (defaults-first resolution applied).
         */
        get: operations["notification_prefs_get"];
        /**
         * PUT /api/notification-preferences — upserts the caller's override row
         *     for one kind and emits `notification_prefs.updated`. Quiet hours
         *     validate as a paired `HH:MM` window in a real IANA timezone;
         *     validation errors return 422.
         */
        put: operations["notification_prefs_update"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/notifications — the caller's notifications, newest first,
         *     keyset-paginated.
         */
        get: operations["notifications_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications/mark-read": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/notifications/mark-read — bulk variant; unknown and
         *     already-read ids are skipped (idempotent).
         */
        post: operations["notifications_mark_read_bulk"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications/read-all": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/notifications/read-all — empties the unread badge. */
        post: operations["notifications_mark_all_read"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications/unread-count": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/notifications/unread-count */
        get: operations["notifications_unread_count"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications/{id}/read": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/notifications/{id}/read — marks one notification read and
         *     emits `notification.read` for cross-session badge sync.
         */
        post: operations["notifications_mark_read"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/on-call-blocks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/on-call-blocks?user_id=&from=&to= */
        get: operations["on_call_list"];
        put?: never;
        /** POST /api/on-call-blocks — add a rotation segment (409 when stacking). */
        post: operations["on_call_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/on-call-blocks/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** DELETE /api/on-call-blocks/{id}?user_id= */
        delete: operations["on_call_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/org": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/org — profile fields shown on General settings / the wizard. */
        get: operations["org_profile_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/org — updates name and/or ticket prefix (Admin:edit).
         *     Idempotent; emits `org.updated` + audit row.
         */
        patch: operations["org_profile_patch"];
        trace?: never;
    };
    "/api/v1/org/exports": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/org/exports — builds the versioned bundle, streams it into
         *     the ObjectStore, and returns the signed download link.
         */
        post: operations["org_export_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/org/exports/{org_id}/{export_id}/download": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/org/exports/{org_id}/{export_id}/download — streams the stored
         *     bundle. Authenticated by the signed link alone (no JWT): an expired or
         *     forged token is an unauthorized request, indistinguishable between the
         *     two. The org id is IN the path (and inside the signature) so a token
         *     authorizes exactly one (org, export, expiry) triple.
         */
        get: operations["org_export_download"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/org/manifest": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/org/manifest — the org's customization manifest.
         * @description Owner/admin gated (policy §5: the manifest is an org-level
         *     administrative artifact; technicians and dispatchers have no business
         *     reading the full customization inventory, e.g. webhook URLs).
         */
        get: operations["org_manifest"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal-users/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/portal-users/{id} — hard removal (sessions cascade). Prefer
         *     PATCH status=disabled to preserve history.
         */
        delete: operations["portal_users_delete"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/portal-users/{id} — rename / re-tier / enable / disable /
         *     reset password.
         */
        patch: operations["portal_users_patch"];
        trace?: never;
    };
    "/api/v1/portal/login": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/portal/login — argon2 verify against `portal_users`. No MFA in
         *     portal v1 (documented deferral). Disabled accounts resolve to nothing →
         *     uniform 401, never revealing which half failed.
         */
        post: operations["portal_login"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/me": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/portal/me — the signed-in portal identity. */
        get: operations["portal_me"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/refresh": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/portal/refresh — rotation within the portal session family.
         *     A presented token resolves ONLY via its live session row; rotation
         *     revokes that row, so replaying an old refresh token fails.
         */
        post: operations["portal_refresh"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/tickets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/portal/tickets — the caller's client's tickets across EXPOSED
         *     boards only. Unexposed boards are invisible, not empty-filtered.
         */
        get: operations["portal_list_tickets"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/tickets/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/portal/tickets/{id} — detail plus comments through the
         *     notes-taxonomy whitelist for the caller's tier. A ticket from another
         *     client (or an unexposed board) is a 404, indistinguishable from absent.
         */
        get: operations["portal_get_ticket"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/tickets/{id}/attachments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/portal/tickets/{id}/attachments — attachments the portal
         *     caller may see: ticket-level files always; comment files only when the
         *     owning note kind is visible to this tier.
         */
        get: operations["portal_list_attachments"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/tickets/{id}/attachments/{attachment_id}/download": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/portal/tickets/{id}/attachments/{attachment_id}/download —
         *     streams one visible attachment through the API. Hidden or foreign
         *     attachments are plain 404s (no existence leak).
         */
        get: operations["portal_download_attachment"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/portal/tickets/{id}/reply": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/portal/tickets/{id}/reply — ALWAYS creates a `client_visible`
         *     comment regardless of tier, authored under portal audit provenance
         *     (`actor_kind = portal_contact | portal_approver | portal_it_manager`;
         *     `author_id` stays NULL because portal identities are not staff rows).
         *     Emits the SAME `ticket.comment_added` event as staff replies so every
         *     downstream notification path works unchanged.
         */
        post: operations["portal_reply"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/project-templates": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/project-templates */
        get: operations["project_templates_list"];
        put?: never;
        /**
         * POST /api/project-templates — stores a blueprint (spec validated);
         *     emits `project.template_updated`.
         */
        post: operations["project_templates_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/project-templates/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/project-templates/{id} — blueprints are data, not live
         *     references: projects already instantiated are untouched; emits
         *     `project.template_removed`.
         */
        delete: operations["project_templates_delete"];
        options?: never;
        head?: never;
        /** PATCH /api/project-templates/{id} — emits `project.template_updated`. */
        patch: operations["project_templates_patch"];
        trace?: never;
    };
    "/api/v1/project-templates/{id}/instantiate": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/project-templates/{id}/instantiate — expands the template
         *     into a REAL project: phases, tasks, dependencies, and every lifecycle
         *     event, all in ONE transaction (claim → expand → insert → events →
         *     record). Rollback on any failure leaves zero partial rows.
         */
        post: operations["project_templates_instantiate"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/project-views": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/project-views — one project's views, or all org views when
         *     `project_id` is omitted.
         */
        get: operations["project_views_list"];
        put?: never;
        /** POST /api/project-views — saves a view; emits `project.view_updated`. */
        post: operations["project_views_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/project-views/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** DELETE /api/project-views/{id} — emits `project.view_removed`. */
        delete: operations["project_views_delete"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/project-views/{id} — renames / respecs a view; emits
         *     `project.view_updated`.
         */
        patch: operations["project_views_patch"];
        trace?: never;
    };
    "/api/v1/projects": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/projects — filtered, newest-first list. */
        get: operations["projects_list"];
        put?: never;
        /**
         * POST /api/projects — creates the project and emits `project.created` in
         *     one transaction. Instant-first (IF1/IF2): idempotency-keyed,
         *     version-gated, read-your-write response.
         */
        post: operations["projects_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/projects/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/projects/{id} */
        get: operations["projects_get_one"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/projects/{id} — tri-state partial update. Instant-first
         *     (IF1/IF2): idempotency-keyed, version-gated, returns prior + new values.
         */
        patch: operations["projects_patch"];
        trace?: never;
    };
    "/api/v1/projects/{id}/phases": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/projects/{id}/phases — ordered by sort_order. */
        get: operations["projects_list_phases"];
        put?: never;
        /**
         * POST /api/projects/{id}/phases — adds a phase; emits
         *     `project.phase_created`.
         */
        post: operations["projects_create_phase"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/projects/{id}/phases/{phase_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /** PATCH /api/projects/{id}/phases/{phase_id} — tri-state partial update. */
        patch: operations["projects_patch_phase"];
        trace?: never;
    };
    "/api/v1/projects/{id}/phases/{phase_id}/complete": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/projects/{id}/phases/{phase_id}/complete — stamps the phase
         *     completed and, when it is a billing milestone, emits
         *     `project.milestone_completed` in the same transaction (the core-billing
         *     seam). Instant-first: the idempotency key guarantees a retried
         *     completion can never double-emit the billing event.
         */
        post: operations["projects_complete_phase"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/projects/{id}/tasks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/projects/{id}/tasks — filtered by phase/status/assignee and
         *     the milestone flag, sort_order-ordered.
         */
        get: operations["projects_list_tasks"];
        put?: never;
        /**
         * POST /api/projects/{id}/tasks — creates the task (optionally with its
         *     dependency set) and emits `project.task_created` in one transaction.
         */
        post: operations["projects_create_task"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/projects/{id}/tasks/{task_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/projects/{id}/tasks/{task_id} — tri-state partial update.
         *     `status: "completed"` is rejected: completion runs through the complete
         *     endpoint (dependency gating + the billing seam).
         */
        patch: operations["projects_patch_task"];
        trace?: never;
    };
    "/api/v1/projects/{id}/tasks/{task_id}/complete": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/projects/{id}/tasks/{task_id}/complete — the guarded
         *     completion: every finish-to-start dependency must already be completed
         *     (typed 409 `task_dependencies_unsatisfied` naming the blockers), then
         *     `project.task_completed` fires — plus `project.milestone_completed`
         *     when the task is a billing milestone. Instant-first: the idempotency
         *     key makes retried completions replay instead of double-billing.
         */
        post: operations["projects_complete_task"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/projects/{id}/tasks/{task_id}/dependencies": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/projects/{id}/tasks/{task_id}/dependencies */
        get: operations["projects_get_dependencies"];
        /**
         * PUT /api/projects/{id}/tasks/{task_id}/dependencies — replaces the
         *     whole dependency set, cycle-checked over the project graph BEFORE any
         *     write; emits `project.task_updated`.
         */
        put: operations["projects_set_dependencies"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/report-definitions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/report-definitions — visible definitions, deterministically
         *     ordered `(name, id)`.
         */
        get: operations["report_definitions_list"];
        put?: never;
        /**
         * POST /api/report-definitions — create a stored definition through the
         *     instant-first write path (idempotency key honored; response carries
         *     `mutation` metadata).
         */
        post: operations["report_definitions_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/report-definitions/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/report-definitions/{id} — one visible definition; anything not
         *     visible (deleted, other org, private of another user) is a bare 404.
         */
        get: operations["report_definitions_get"];
        /**
         * PUT /api/report-definitions/{id} — full-replace update through the
         *     instant-first write path (version gate enforced; a miss inside a
         *     mutation is the typed `rls_sparse` reason, not `not_found`).
         */
        put: operations["report_definitions_update"];
        post?: never;
        /**
         * DELETE /api/report-definitions/{id} — removal through the same gated
         *     path; the write trail keeps the deleted snapshot (undo substrate).
         */
        delete: operations["report_definitions_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/report-schedules": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/report-schedules — cursor-paged listing (admin), ordered
         *     `(name, id)`, each row carrying its next-run preview.
         */
        get: operations["report_schedules_list"];
        put?: never;
        /**
         * POST /api/report-schedules — create a schedule through the
         *     instant-first write path; the next-run preview computes from now.
         */
        post: operations["report_schedules_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/report-schedules/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/report-schedules/{id} — one schedule; anything not in this
         *     org is a bare 404.
         */
        get: operations["report_schedules_get"];
        /**
         * PUT /api/report-schedules/{id} — full-replace update; the next-run
         *     preview recomputes from now (cadence changes cannot strand a slot in
         *     the past; the worker's claim sweep catches up enabled schedules
         *     anyway, so this only affects the preview's freshness).
         */
        put: operations["report_schedules_update"];
        post?: never;
        /**
         * DELETE /api/report-schedules/{id} — removal through the same gated
         *     path; ledger rows cascade and the write trail keeps the snapshot.
         */
        delete: operations["report_schedules_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/reports/{kind}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/reports/{kind} — prebuilt operational report over the read-model
         *     views. Kinds: time_by_user | time_by_client | time_by_ticket |
         *     sla_attainment | sla_breaches | ticket_throughput | ticket_aging |
         *     first_response.
         */
        get: operations["report_get"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/scheduling/conflict-check": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/scheduling/conflict-check — dry-run overlap probe that writes
         *     nothing: returns every blocking booking for the candidate slot.
         */
        post: operations["scheduling_conflict_check"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/search": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/search — unified staff search across tickets, comments and the
         *     client hierarchy.
         */
        get: operations["search_query"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/ai/byok": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /**
         * PUT /api/settings/ai/byok — seals and stores an API key at rest
         *     (Admin:edit + owner/admin standing). Rides the instant-first write
         *     path; replays are byte-identical and the trail stores presence flags
         *     only.
         */
        put: operations["settings_ai_byok_put"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/ai/byok/{provider}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/settings/ai/byok/{provider} — removes a sealed BYOK key
         *     through the same gated path; the write trail keeps the presence
         *     snapshot.
         */
        delete: operations["settings_ai_byok_delete"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/ai/status": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/settings/ai/status — overview of AI completion provider & configured keys. */
        get: operations["settings_ai_status"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/ai/test": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/settings/ai/test — test-connection probe (Admin:edit gated).
         *     None mode returns the typed unavailable outcome; a configured provider
         *     verifies its sealed key still decrypts (tamper/rotation check). No
         *     vendor HTTP call happens in v1 — the live probe rides the edge adapters
         *     when a lane wires them.
         */
        post: operations["settings_ai_test"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/org": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/settings/org — effective org settings (defaults overlaid by
         *     stored overrides). Visible to every member.
         */
        get: operations["settings_org_get"];
        /**
         * PUT /api/settings/org — upserts one org-scope override after registry
         *     validation (Admin:edit required). Emits `setting.updated` + audit row.
         */
        put: operations["settings_org_put"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/registry": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/settings/registry — every declared setting with kind, default,
         *     scope and validation shape. Any authenticated member may read it.
         */
        get: operations["settings_registry"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/settings/user": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/settings/user — the caller's effective personal settings. */
        get: operations["settings_user_get"];
        /**
         * PUT /api/settings/user — upserts one of the CALLER's own user-scope
         *     overrides. No role requirement: everyone may tune their own preferences.
         */
        put: operations["settings_user_put"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/sla/calendars": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/sla/calendars */
        get: operations["sla_calendars_list"];
        put?: never;
        /** POST /api/sla/calendars */
        post: operations["sla_calendars_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/sla/calendars/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/sla/calendars/{id} — detaches policies (FK SET NULL); their
         *     embedded calendars keep governing.
         */
        delete: operations["sla_calendars_delete"];
        options?: never;
        head?: never;
        /**
         * PATCH /api/sla/calendars/{id} — partial update; validates the merged
         *     result by re-reading and running domain validation.
         */
        patch: operations["sla_calendars_update"];
        trace?: never;
    };
    "/api/v1/sla/policies": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/sla/policies */
        get: operations["sla_policies_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/sla/policies/{id}/ladder": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/sla/policies/{id}/ladder — enabled + disabled steps, ordered. */
        get: operations["sla_ladder_get"];
        /** PUT /api/sla/policies/{id}/ladder — replaces the whole ladder in one tx. */
        put: operations["sla_ladder_replace"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/sso/exchange": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/auth/sso/exchange — swaps the single-use callback code for
         *     tokens (or a link confirmation). Codes die on first use or after 60s.
         */
        post: operations["sso_exchange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets — filtered, priority-ordered list; each row carries its
         *     custom-field values (TASKLIST 2.17) fetched in one batched query.
         */
        get: operations["tickets_list"];
        put?: never;
        /**
         * POST /api/tickets — allocates the human ticket number, initializes SLA
         *     clocks, validates + stores any initial custom-field values (REQUIRED
         *     definitions enforced), and emits `ticket.created` — all in one transaction.
         *     Instant-first (IF1/IF2): routed through the shared idempotency gate and
         *     returns the read-your-write `mutation` metadata.
         */
        post: operations["tickets_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/my-work": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets/my-work — everything assigned to me, not closed,
         *     across all boards, priority-then-SLA-due ordered (the unified
         *     cross-board view from §1; TASKLIST 2.11 adds client/board annotations).
         */
        get: operations["tickets_my_work"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/tickets/{id} */
        get: operations["tickets_get_one"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /**
         * PATCH /api/tickets/{id} — partial update; validates + upserts any
         *     provided custom-field values; emits `ticket.updated`,
         *     `ticket.field_set` (per changed field), and `ticket.assigned` when the
         *     assignee changes. Instant-first (IF1/IF2): idempotency-keyed, version-
         *     gated, and returns prior + new values for undo.
         */
        patch: operations["tickets_patch"];
        trace?: never;
    };
    "/api/v1/tickets/{id}/attachments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/tickets/{id}/attachments — every attachment on one ticket. */
        get: operations["attachments_list_for_ticket"];
        put?: never;
        /**
         * POST /api/tickets/{id}/attachments — stores one file against a ticket
         *     (linked to a comment later by the composer submit).
         * @description Headers: `x-filename` (required), `content-type` (declared; verified
         *     against sniffed magic bytes). Body: raw bytes, capped by
         *     `MERIDIAN_MAX_UPLOAD_MB` (default 25 MB).
         */
        post: operations["attachments_upload"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/checklist": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets/{id}/checklist — lazily instantiates any template items
         *     the ticket has not picked up yet, then returns the full view.
         */
        get: operations["tickets_get_checklist"];
        put?: never;
        /**
         * POST /api/tickets/{id}/checklist — appends an ad-hoc checklist item.
         *     Ad-hoc items never block closure; emits `ticket.checklist_updated`.
         *     Instant-first (IF1/IF2): idempotency-keyed and returns the
         *     read-your-write mutation metadata for the created item.
         */
        post: operations["tickets_add_checklist_item"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/checklist/{item_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/tickets/{id}/checklist/{item_id} — toggles one checklist item;
         *     emits `ticket.checklist_updated` in the same transaction. Instant-first
         *     (IF1/IF2): idempotency-keyed and returns the read-your-write mutation
         *     metadata for the toggled item.
         */
        post: operations["tickets_set_checklist_item"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/comments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets/{id}/comments — internal notes require ticket view;
         *     the portal surface filters kinds separately (portal v1).
         */
        get: operations["tickets_list_comments"];
        put?: never;
        /**
         * POST /api/tickets/{id}/comments — adds a structured note; @mentions
         *     fan out into the notification center. Instant-first (IF1/IF2):
         *     idempotency-keyed and returns the read-your-write mutation metadata.
         */
        post: operations["tickets_add_comment"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/comments/{comment_id}/attachments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets/{id}/comments/{comment_id}/attachments — one comment's
         *     attachments.
         */
        get: operations["attachments_list_for_comment"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/fields/{field_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /**
         * PUT /api/tickets/{id}/fields/{field_id} — validates the incoming JSON
         *     against the definition (kind coercion, option membership) and stores the
         *     canonical encoding; emits `ticket.field_set`.
         */
        put: operations["fields_set_ticket_value"];
        post?: never;
        /**
         * DELETE /api/tickets/{id}/fields/{field_id} — clears one value; refuses
         *     (422) to empty a REQUIRED definition; emits `ticket.field_cleared`.
         */
        delete: operations["fields_clear_ticket_value"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/revert": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/tickets/{id}/revert — undo one server-accepted edit by
         *     restoring its prior values as a NORMAL authorized idempotent write
         *     (IF1 envelope end to end). Appends a new trail row (op `revert`),
         *     emits `ticket.reverted` on the event bus, and writes the audited
         *     `ticket.reverted` trail row — the append-only history is never
         *     rewritten.
         */
        post: operations["tickets_revert"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/sla": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets/{id}/comments — optional kind filter; internal notes
         *     require ticket view. The portal surface must consume the dedicated
         *     client-visible read (`core-db::tickets::list_client_visible_comments`),
         *     never this staff endpoint.
         *     GET /api/tickets/{id}/sla — clock state plus a live business-time
         *     remaining estimate for the resolution budget (drives the UI badge).
         */
        get: operations["tickets_sla_status"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/status": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/tickets/{id}/status — the guarded lifecycle transition:
         *     domain transition rules, pre-close checklist enforcement (typed 409
         *     listing the incomplete required items when the board enforces), SLA
         *     pause bookkeeping, and the `ticket.status_changed` event, all in one tx.
         *     Instant-first (IF1/IF2): idempotency-keyed, version-gated, read-your-write.
         */
        post: operations["tickets_change_status"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/time-entries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/tickets/{id}/time-entries */
        get: operations["tickets_ticket_time_entries"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/tickets/{id}/undo-stack": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/tickets/{id}/undo-stack — the per-field / per-record
         *     applied-state history read model for one record.
         */
        get: operations["tickets_undo_stack"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/time-entries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/time-entries */
        get: operations["timers_list_entries"];
        put?: never;
        /** POST /api/time-entries — manual entry (no timer). */
        post: operations["timers_create_entry"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/time-gaps": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/time-gaps?user_id= — manager-scoped report for one member.
         *     Owner/admin only; everyone else uses `/api/time-gaps/me`.
         */
        get: operations["time_gaps_user"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/time-gaps/me": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/time-gaps/me — the caller's day-level gap report (defaults to
         *     yesterday in the caller's timezone).
         */
        get: operations["time_gaps_me"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/time-settings": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/time-settings — org time-discipline knobs (admin). */
        get: operations["time_settings_get"];
        /** PUT /api/time-settings — update the org's time-discipline knobs (admin). */
        put: operations["time_settings_put"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/timers/active": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/timers/active — the caller's running session (at most one). */
        get: operations["timers_active"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/timers/pause": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/timers/pause — pauses the active (or given) session, recording
         *     the reason on the segment.
         */
        post: operations["timers_pause"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/timers/resume": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/timers/resume — resumes a paused session with a new segment. */
        post: operations["timers_resume"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/timers/start": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/timers/start — starts (or switches to) a timer. If another
         *     session is running it is auto-paused with reason `switched` and stays in
         *     the working set; the DB's partial unique index is the backstop.
         *     Instant-first (IF1/IF2): idempotency-keyed and read-your-write.
         */
        post: operations["timers_start"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/timers/stop": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/timers/stop — stops the session and materializes a time entry
         *     (rounded up to the minute) in the same transaction.
         */
        post: operations["timers_stop"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/timers/working-set": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/timers/working-set — recent sessions (running first): the
         *     docked working set the UI renders.
         */
        get: operations["timers_working_set"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/webhooks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/webhooks — the org's subscriptions (never includes secrets). */
        get: operations["webhooks_list"];
        put?: never;
        /**
         * POST /api/webhooks — create a subscription; responds with the signing
         *     secret once. Emits an audit record in the same transaction.
         */
        post: operations["webhooks_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/webhooks/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /**
         * DELETE /api/webhooks/{id} — removes the subscription; deliveries
         *     cascade (history is intentionally scoped to the subscription's life).
         */
        delete: operations["webhooks_delete"];
        options?: never;
        head?: never;
        /** PATCH /api/webhooks/{id} — mutate URL / filter / enabled flag. */
        patch: operations["webhooks_update"];
        trace?: never;
    };
    "/api/v1/webhooks/{id}/deliveries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * GET /api/webhooks/{id}/deliveries — the in-app delivery log, newest
         *     first, keyset-cursor paged.
         */
        get: operations["webhooks_deliveries_list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/webhooks/{id}/test": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * POST /api/webhooks/{id}/test — fires one `webhook.test` envelope into
         *     the real pipeline (outbox → relay → stream → signed delivery), then
         *     returns 202; poll the deliveries endpoint to watch attempts land.
         */
        post: operations["webhooks_test_fire"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        /**
         * @description Who performed the audited action. Append-only provenance record.
         *     Portal variants carry the client-side tier so audit trails distinguish
         *     staff actions from client-portal actions at the storage layer
         *     (actor_kind portal_contact / portal_approver / portal_it_manager).
         */
        Actor: {
            /** Format: uuid */
            id: string;
            /** @enum {string} */
            kind: "human";
        } | {
            id: string;
            /** @enum {string} */
            kind: "agent";
        } | {
            id: string;
            /** @enum {string} */
            kind: "plugin";
        } | {
            id: string;
            /** @enum {string} */
            kind: "connector";
        } | {
            /** @enum {string} */
            kind: "system";
        } | {
            /** Format: uuid */
            id: string;
            /** @enum {string} */
            kind: "portal_contact";
        } | {
            /** Format: uuid */
            id: string;
            /** @enum {string} */
            kind: "portal_approver";
        } | {
            /** Format: uuid */
            id: string;
            /** @enum {string} */
            kind: "portal_it_manager";
        } | {
            /**
             * @description Bulk-import runs (TASKLIST D2): the descriptor names the driving
             *     identity without pretending it row-walked — `"user:<uuid>"` for
             *     admin JWT runs, `"api-key:<key-id>""` for machine pushes. Row-level
             *     mutations still emit their ordinary domain events; the import actor
             *     owns batch-lifecycle provenance (kind "import" in audit storage).
             */
            id: string;
            /** @enum {string} */
            kind: "import";
        };
        AddChecklistItemRequest: components["schemas"]["MutationParams"] & {
            text: string;
        };
        AddCommentRequest: components["schemas"]["MutationParams"] & {
            /**
             * @description Attachments uploaded earlier against this ticket (composer flow:
             *     upload first, link when the comment is created). Absent/empty in
             *     older clients — purely additive.
             */
            attachment_ids?: string[];
            body: string;
            kind: components["schemas"]["NoteKind"];
        };
        /** @description GET /api/agent-proposals envelope. */
        AgentProposalPage: {
            next_cursor?: string | null;
            proposals: components["schemas"]["AgentProposalView"][];
        };
        /** @description One IF8 proposal with the full snapshot the approval decision needs. */
        AgentProposalView: {
            agent_kind: string;
            agent_ref: string;
            applied_at?: string | null;
            /**
             * Format: int64
             * @description The entity write-seq the proposal was created against.
             */
            base_version: number;
            /** @description The value the agent saw (`null` when the record did not exist). */
            before_value: Record<string, never> | null;
            capability?: string | null;
            classification: components["schemas"]["ProposalClassification"];
            created_at: string;
            decided_at?: string | null;
            /** Format: uuid */
            decided_by?: string | null;
            decision_note?: string | null;
            /** @description Involvement-dial snapshot at proposal time. */
            dial_mode: components["schemas"]["CapabilityMode"];
            /**
             * @description Record coordinates (v1 applies `ticket`/`update` only; the store is
             *     generic).
             */
            entity: string;
            /** Format: uuid */
            entity_id?: string | null;
            /** Format: uuid */
            for_user?: string | null;
            /** Format: uuid */
            id: string;
            op: string;
            /** @description The proposed after value. */
            proposed_value: Record<string, never>;
            rationale?: string | null;
            rule_set_hash?: string | null;
            session_id?: string | null;
            skill_slug?: string | null;
            /** Format: int32 */
            skill_version?: number | null;
            status: components["schemas"]["ProposalStatus"];
            tool?: string | null;
            updated_at: string;
        };
        AgentSkillList: {
            skills: components["schemas"]["AgentSkillView"][];
        };
        /** @description One immutable skill version. */
        AgentSkillView: {
            created_at: string;
            /** Format: uuid */
            created_by: string;
            /** Format: uuid */
            id: string;
            /** @description `{capabilities, prompts, rules}` — declarative manifest (§4). */
            manifest: Record<string, never>;
            slug: string;
            /** Format: int32 */
            version: number;
        };
        /**
         * @description Aggregation function of a measure. `Count` may omit the column
         *     (`COUNT(*)`); every other aggregate requires one, and the catalog
         *     decides which column types each accepts.
         * @enum {string}
         */
        Aggregate: "count" | "sum" | "avg" | "min" | "max";
        AlertListQuery: {
            /** Format: int64 */
            limit?: number;
            /** Format: int64 */
            offset?: number;
            /** Format: uuid */
            source_id?: string | null;
            /** Format: uuid */
            ticket_id?: string | null;
        };
        AlertListResponse: {
            alerts: components["schemas"]["AlertView"][];
        };
        /**
         * @description Alert severity vocabulary. Maps onto ticket priorities when an alert
         *     anchors an incident ticket: critical→P1, warning→P2, info→P3.
         * @enum {string}
         */
        AlertSeverity: "info" | "warning" | "critical";
        AlertSourceList: {
            sources: components["schemas"]["AlertSourceView"][];
        };
        /**
         * @description Instant-first mutation response for a source (IF2). `secret` is the
         *     raw HMAC signing secret — populated ONLY by the create response and
         *     never again (same show-once contract as webhooks/api-keys).
         */
        AlertSourceMutationResponse: components["schemas"]["AlertSourceView"] & {
            mutation: components["schemas"]["MutationMeta"];
            secret?: string | null;
        };
        /**
         * @description An ingest source as the admin surface sees it. The HMAC secret is
         *     NEVER here — it is returned exactly once, in the create response.
         */
        AlertSourceView: {
            /** Format: uuid */
            board_id: string;
            /** Format: uuid */
            client_id?: string | null;
            /** Format: int32 */
            correlation_window_secs: number;
            created_at: string;
            enabled: boolean;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** @description Path segment of the ingest URL (public routing id). */
            source_ref: string;
            updated_at: string;
        };
        /** @description One normalized alert (canonical ingest document + ledger row view). */
        AlertView: {
            attributes: Record<string, never>;
            /** @description Storm grouping key (defaults to the dedup key server-side). */
            correlation_key: string;
            /** @description Provider's per-event identity; the replay gate per source. */
            dedup_key: string;
            description?: string | null;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            incident_ticket_id?: string | null;
            occurred_at: string;
            /** Format: uuid */
            org_id: string;
            received_at: string;
            severity: string;
            /** Format: uuid */
            source_id: string;
            title: string;
        };
        /**
         * @description The ONLY response carrying raw key material. Lost keys are rotated by
         *     creating a new key and revoking this one — never recovered.
         */
        ApiKeyCreatedResponse: {
            key: components["schemas"]["ApiKeyView"];
            /** @description Raw bearer material: `mrd_<32 hex>`. */
            raw_key: string;
        };
        ApiKeyList: {
            keys: components["schemas"]["ApiKeyView"][];
        };
        ApiKeyView: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            /** Format: date-time */
            last_used_at?: string | null;
            name: string;
            /** @description Non-secret display prefix, e.g. "mrd_ab12cd34". */
            prefix: string;
            /** Format: date-time */
            revoked_at?: string | null;
            scopes: string[];
        };
        /**
         * @description A booked slot on one user's calendar, optionally tied to a ticket and
         *     client/site. `travel_minutes` is a dispatch-board placeholder — never
         *     interpreted by scheduling rules in this stage.
         */
        Appointment: {
            /** Format: uuid */
            client_id?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            ends_at: string;
            /** Format: uuid */
            id: string;
            kind: components["schemas"]["AppointmentKind"];
            notes?: string | null;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            site_id?: string | null;
            /** Format: date-time */
            starts_at: string;
            status: components["schemas"]["AppointmentStatus"];
            /** Format: uuid */
            ticket_id?: string | null;
            title: string;
            /** Format: int32 */
            travel_minutes: number;
            /** Format: date-time */
            updated_at: string;
            /** Format: uuid */
            user_id: string;
        };
        /**
         * @description What an appointment is physically for. Drives rendering color-coding on
         *     calendars and future travel-time logic (on_site only).
         * @enum {string}
         */
        AppointmentKind: "on_site" | "remote" | "internal" | "personal";
        /**
         * @description Lifecycle of a booking. `cancelled` frees the slot (the DB exclusion
         *     constraint's partial predicate); `done` still occupies it.
         * @enum {string}
         */
        AppointmentStatus: "scheduled" | "in_progress" | "done" | "cancelled";
        ArticleListQuery: {
            /** Format: uuid */
            category_id?: string | null;
            /** @description Only articles passing the effective client-facing gate. */
            client_visible?: boolean;
            /** @description Title substring narrowing; ranked full-text search rides /api/search. */
            q?: string | null;
            status?: null | components["schemas"]["KbArticleStatus"];
        };
        /** @description Instant-first mutation response (IF2). */
        ArticleMutationResponse: components["schemas"]["KbArticle"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        AssignGroupRequest: {
            /**
             * Format: uuid
             * @description `Some(group)` moves the board into that group; `None` ungroups it.
             */
            group_id?: string | null;
        };
        AssignRequest: {
            /** Format: date-time */
            ends_at: string;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            ticket_id: string;
            /** Format: int32 */
            travel_minutes?: number;
            /** Format: uuid */
            user_id: string;
        };
        /** @description One uploaded file's metadata row; the payload lives behind `storage_key`. */
        Attachment: {
            /** @description Declared + magic-byte-verified content type. */
            content_type: string;
            /** Format: date-time */
            created_at: string;
            /**
             * Format: uuid
             * @description Set only when kind is email_message.
             */
            email_message_id?: string | null;
            /** @description Original filename, sanitized (no path components/control chars). */
            filename: string;
            /** Format: uuid */
            id: string;
            kind: components["schemas"]["AttachmentKind"];
            /** Format: uuid */
            org_id: string;
            /** @description Lowercase hex sha256 of the exact bytes; dedup + integrity anchor. */
            sha256: string;
            /** Format: int64 */
            size_bytes: number;
            /** @description Object-store key (`org/<id>/attachments/<sha256>`). */
            storage_key: string;
            /**
             * Format: uuid
             * @description Set only when kind is ticket_comment.
             */
            ticket_comment_id?: string | null;
            /**
             * Format: uuid
             * @description Set when kind is ticket_comment or ticket.
             */
            ticket_id?: string | null;
            /**
             * Format: uuid
             * @description Portal actor when uploaded through a portal surface (v1: none —
             *     reserved so portal uploads need no schema change).
             */
            uploaded_by_portal_user_id?: string | null;
            /**
             * Format: uuid
             * @description Staff uploader when uploaded through the API.
             */
            uploaded_by_user_id?: string | null;
        };
        /**
         * @description attachment.deleted — the metadata row went away; `object_removed`
         *     reports whether the backing bytes were reclaimed too (refcount hit 0).
         */
        AttachmentDeleted: {
            /** Format: uuid */
            attachment_id: string;
            object_removed: boolean;
            /** Format: uuid */
            ticket_id?: string | null;
        };
        /**
         * @description What an attachment belongs to. The enum value is persisted verbatim.
         * @enum {string}
         */
        AttachmentKind: "ticket_comment" | "email_message" | "ticket";
        AuditEvent: {
            /** @description Dot-path of the action, e.g. "auth.login", "ticket.status_change". */
            action: string;
            actor: components["schemas"]["Actor"];
            /** @description Structured detail — never contains secrets. */
            detail: unknown;
            entity_id?: string | null;
            /** @description Entity type + id the action targeted, when applicable. */
            entity_type?: string | null;
            /** Format: uuid */
            id: string;
            ip?: string | null;
            /** Format: date-time */
            occurred_at: string;
            /** Format: uuid */
            org_id: string;
        };
        AuditPage: {
            events: components["schemas"]["AuditEvent"][];
            /** @description Opaque cursor for the next page; absent when exhausted. */
            next_cursor?: string | null;
        };
        /**
         * @description Effects a rule may request. Every variant maps onto an existing core-api
         *     write path — automations never gain private write routes.
         */
        AutomationAction: {
            /** @enum {string} */
            type: "assign_user";
            /** Format: uuid */
            user_id: string;
        } | {
            priority: components["schemas"]["Priority"];
            /** @enum {string} */
            type: "set_priority";
        } | {
            body: string;
            /** @enum {string} */
            type: "add_internal_comment";
        } | {
            kind: components["schemas"]["NotificationKind"];
            title: string;
            /** @enum {string} */
            type: "notify_users";
            user_ids: string[];
        };
        /** @description One predicate; a rule matches only when EVERY condition holds. */
        AutomationCondition: {
            priority: components["schemas"]["Priority"];
            /** @enum {string} */
            type: "priority_is";
        } | {
            category: components["schemas"]["TicketStatusCategory"];
            /** @enum {string} */
            type: "category_is";
        } | {
            needle: string;
            /** @enum {string} */
            type: "title_contains";
        } | {
            /** @enum {string} */
            type: "unassigned";
        };
        AvailabilityListResponse: {
            /** Format: uuid */
            user_id: string;
            windows: components["schemas"]["AvailabilityWindow"][];
        };
        AvailabilityQuery: {
            /**
             * Format: uuid
             * @description Whose windows. Defaults to the caller; another user requires owner/admin.
             */
            user_id?: string | null;
        };
        /**
         * @description One recurring weekly availability template: `weekday` (0 = Monday …
         *     6 = Sunday), local wall-clock `[start_local, end_local)` in `timezone`,
         *     optionally bounded by an effective date range. This is a TEMPLATE —
         *     occurrences are computed, never stored.
         */
        AvailabilityWindow: {
            /** Format: date-time */
            created_at: string;
            effective_from?: string | null;
            effective_until?: string | null;
            end_local: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            start_local: string;
            /** @description IANA timezone the wall-clock bounds live in. */
            timezone: string;
            /** Format: uuid */
            user_id: string;
            /** Format: int32 */
            weekday: number;
        };
        BatchCreatedResponse: {
            /** Format: uuid */
            batch_id: string;
            label: string;
        };
        BatchViewResponse: {
            conflict_policy: components["schemas"]["ConflictPolicy"];
            dry_run: boolean;
            entity_kind: components["schemas"]["EntityKind"];
            /** Format: date-time */
            finished_at?: string | null;
            /** Format: uuid */
            id: string;
            label: string;
            /** Format: uuid */
            org_id: string;
            /** Format: date-time */
            queued_at?: string | null;
            /** Format: int32 */
            rows_created: number;
            /** Format: int32 */
            rows_failed: number;
            /** Format: int32 */
            rows_skipped: number;
            /** Format: int32 */
            rows_total: number;
            /** Format: int32 */
            rows_updated: number;
            source_note?: string | null;
            /**
             * Format: int32
             * @description Async giant-batch bookkeeping (D2 remainder): staged chunk count and
             *     the once-only worker-enqueue stamp.
             */
            staged_chunks: number;
            /** Format: date-time */
            started_at: string;
        };
        /**
         * @description One draft invoice-line candidate. This is a PREVIEW shape: nothing
         *     persists until Wave 9's invoice run.
         */
        BillingCandidateLine: {
            /**
             * Format: int64
             * @description Line total in minor units (currency on the parent evaluation).
             */
            amount_minor: number;
            /** Format: uuid */
            client_id: string;
            /** Format: uuid */
            contract_id: string;
            contract_number: string;
            description: string;
            kind: components["schemas"]["BillingLineKind"];
            /** Format: date */
            period_end: string;
            /**
             * Format: date
             * @description The billing period (recurring/retainer) or the effective slice of
             *     the evaluation window (T&M) or the fee's due date mirrored into
             *     both ends (fixed fee). Inclusive start, exclusive end for
             *     period-derived lines.
             */
            period_start: string;
            /**
             * Format: int64
             * @description Labor minutes on the line (T&M/overage); 0 for amount lines.
             */
            quantity_minutes: number;
            /**
             * Format: int64
             * @description Unit price in minor units: the hourly rate for labor lines, the
             *     full amount otherwise.
             */
            unit_amount_minor: number;
        };
        /** @description The result of evaluating one contract's billing rules over a window. */
        BillingEvaluation: {
            /** Format: uuid */
            client_id: string;
            /** Format: uuid */
            contract_id: string;
            contract_number: string;
            contract_type: components["schemas"]["ContractType"];
            currency: string;
            /**
             * Format: date
             * @description The evaluation window (inclusive bounds).
             */
            from: string;
            lines: components["schemas"]["BillingCandidateLine"][];
            /** Format: date */
            to: string;
            /**
             * Format: int64
             * @description Sum of line amounts, minor units.
             */
            total_minor: number;
        };
        /**
         * @description Billing period length. Period boundaries derive from the contract's
         *     `billing_anchor`: the anchor is the first period's start date and each
         *     subsequent period starts one frequency step later (calendar-aware
         *     clamping lives in core-billing's period arithmetic).
         * @enum {string}
         */
        BillingFrequency: "weekly" | "monthly" | "quarterly" | "semiannual" | "annual";
        /**
         * @description Which billing rule produced a candidate line.
         * @enum {string}
         */
        BillingLineKind: "recurring_base" | "retainer_base" | "retainer_overage" | "time_materials" | "fixed_fee";
        /**
         * @description How the project turns delivered work into money. The value rides the
         *     milestone event payload so core-billing can route without re-reading the
         *     project row (event payloads are additive-only, policy §2).
         * @enum {string}
         */
        BillingMethod: "milestone" | "time_and_materials" | "fixed_fee" | "non_billable";
        Board: {
            /**
             * Format: date-time
             * @description Users whose favorite this board is (board groups/favorites, Stage 2).
             */
            created_at: string;
            description?: string | null;
            /** Format: uuid */
            id: string;
            is_default: boolean;
            name: string;
            /** Format: uuid */
            org_id: string;
            /**
             * @description Pre-close checklist enforcement toggle (defaults OFF; closing is
             *     gated only when this AND a checklist template are both present —
             *     defaults-first doctrine, migration 0016).
             */
            require_checklist_close: boolean;
        };
        BoardExposureView: {
            /** Format: uuid */
            board_id: string;
            /** Format: uuid */
            client_id: string;
            exposed: boolean;
            /** Format: uuid */
            id: string;
            visible_status_ids: string[];
        };
        /**
         * @description A named, ordered nav grouping of boards within one org (TASKLIST 2.11).
         *     Groups are organization-wide; `owner_id` records who controls the group.
         */
        BoardGroup: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            owner_id: string;
            /** Format: int32 */
            position: number;
        };
        /**
         * @description An explicit per-user membership on one board carrying a board-level
         *     role tier (§6 board-level RBAC). Rows NARROW org grants; absence of a
         *     row leaves org-level governance untouched.
         */
        BoardMember: {
            /** Format: uuid */
            board_id: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            org_id: string;
            role: components["schemas"]["BoardRole"];
            /** Format: uuid */
            user_id: string;
        };
        BoardMemberView: {
            /** Format: date-time */
            created_at: string;
            display_name: string;
            role: components["schemas"]["BoardRole"];
            /** Format: uuid */
            user_id: string;
        };
        /** @description One left-nav entry: the board plus the CALLER's annotations. */
        BoardNavEntry: {
            description?: string | null;
            /** @description Whether THIS user favorited the board (star). */
            favorite: boolean;
            /**
             * Format: uuid
             * @description Group assignment for the collapsible tree (`None` = ungrouped).
             */
            group_id?: string | null;
            group_name?: string | null;
            /** Format: uuid */
            id: string;
            is_default: boolean;
            my_role?: null | components["schemas"]["BoardRole"];
            name: string;
        };
        /**
         * @description Per-board role tier stored on `board_members` rows (migration 0019,
         *     TASKLIST 2.11). Board roles sit BESIDE the org matrix and can only
         *     narrow it — see `authorize_board_scoped`.
         * @enum {string}
         */
        BoardRole: "viewer" | "member" | "admin";
        /**
         * @description A per-board status. `category` is the binding contract — labels are
         *     presentation, category is behavior.
         */
        BoardStatus: {
            /** Format: uuid */
            board_id: string;
            category: components["schemas"]["TicketStatusCategory"];
            color?: string | null;
            /** Format: uuid */
            id: string;
            /** @description Whether tickets may enter this status directly from New. */
            is_entry: boolean;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: int32 */
            sort_order: number;
        };
        /**
         * @description GET response for a board's workflow: either its explicit matrix or the
         *     marker that the semantic-category floor governs.
         */
        BoardTransitionsResponse: {
            edges: components["schemas"]["TransitionEdge"][];
            /**
             * @description `"explicit"` when the listed edges are authoritative; `"floor"` when
             *     the semantic-category default governs (any→any except closed→open).
             */
            mode: string;
        };
        BulkMoveRequest: {
            moves: components["schemas"]["MoveSpec"][];
        };
        BulkMoveResponse: {
            moved: components["schemas"]["DispatchAppointment"][];
        };
        /**
         * @description Weekly business hours in a named IANA timezone, plus holiday dates.
         *     `None` window = non-business day.
         */
        BusinessCalendar: {
            /** @description Local dates (YYYY-MM-DD) with no business hours. */
            holidays: string[];
            /**
             * @description Downtime stretches subtracted from every measurement. Absent in
             *     legacy documents (`default`) so stored policies keep deserializing.
             */
            maintenance?: components["schemas"]["MaintenanceWindow"][];
            name: string;
            timezone: string;
            /** @description [Mon..Sun] open/close in local wall-clock "HH:MM". */
            windows: ([
                string,
                string
            ] | null)[];
        };
        /** @description Summary of all configured AI providers for the org. */
        ByokOverviewResponse: {
            active_model: string;
            active_provider: string;
            configured_providers: string[];
        };
        /**
         * @description Mutation response: status fields plus the instant-first mutation
         *     metadata (`mutation.version` echoes into the next write).
         */
        ByokSecretMutationResponse: components["schemas"]["ByokSecretStatus"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description Non-secret status of a stored BYOK key (presence flags only — never
         *     key material). Flattened into mutation responses so a stored replay
         *     deserializes byte-identically.
         */
        ByokSecretStatus: {
            has_key: boolean;
            provider: string;
            updated_at: string | null;
        };
        /** @description Absolute UTC time-off punching holes in availability. */
        CalendarExclusion: {
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            ends_at: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            reason?: string | null;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            user_id: string;
        };
        /**
         * @description Flat merged entry so calendars render one ordered list without caring
         *     which table produced it.
         */
        CalendarItem: {
            /** @description On-call-only field. */
            category?: string | null;
            /** Format: date-time */
            ends_at: string;
            /** Format: uuid */
            id: string;
            /** @description "appointment" | "on_call" */
            item_type: string;
            kind?: null | components["schemas"]["AppointmentKind"];
            /** Format: date-time */
            starts_at: string;
            status?: null | components["schemas"]["AppointmentStatus"];
            /** Format: uuid */
            ticket_id?: string | null;
            title?: string | null;
            /** Format: uuid */
            user_id: string;
        };
        CalendarViewResponse: {
            /** @description Present on personal views only (templates are per-user). */
            availability_windows: components["schemas"]["AvailabilityWindow"][];
            exclusions: components["schemas"]["CalendarExclusion"][];
            /** Format: date-time */
            from: string;
            /** @description Merged appointments + on-call, ordered by starts_at then id. */
            items: components["schemas"]["CalendarItem"][];
            timezone: string;
            /** Format: date-time */
            to: string;
            /**
             * Format: uuid
             * @description None on team-wide views.
             */
            user_id?: string | null;
        };
        CapabilityGrantList: {
            grants: components["schemas"]["CapabilityGrantView"][];
        };
        /** @description One granted capability with its dial position. */
        CapabilityGrantView: {
            capability: string;
            granted_at: string;
            /** Format: uuid */
            granted_by: string;
            mode: components["schemas"]["CapabilityMode"];
            updated_at: string;
        };
        /**
         * @description Wire involvement-dial position of a capability grant.
         * @enum {string}
         */
        CapabilityMode: "proposed" | "autonomous";
        /**
         * @description Instant-first mutation response (IF2): category fields at the top level
         *     plus the additive `mutation` metadata.
         */
        CategoryMutationResponse: components["schemas"]["KbCategory"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        ChangeStatusRequest: components["schemas"]["MutationParams"] & {
            /** Format: uuid */
            status_id: string;
        };
        /**
         * @description One line of a ticket's pre-close checklist. Items instantiated from the
         *     board's template carry `template_item_id`; those are REQUIRED when the
         *     board enforces pre-close checklists (see `crate::notes`). Ad-hoc items
         *     (`template_item_id == None`) never block closure.
         */
        ChecklistItem: {
            checked: boolean;
            /** Format: date-time */
            checked_at?: string | null;
            /** Format: uuid */
            checked_by?: string | null;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            /** Format: int32 */
            sort_order: number;
            /**
             * Format: uuid
             * @description Template provenance: `Some` ⇒ required under enforcement.
             */
            template_item_id?: string | null;
            text: string;
            /** Format: uuid */
            ticket_id: string;
        };
        /**
         * @description Derived-at-read-time checklist progress (never materialized — see the
         *     0016 migration header for the justification). `required_open` counts
         *     unchecked template-derived instances: the close-blocking set when the
         *     board enforces.
         */
        ChecklistProgress: {
            /**
             * Format: int64
             * @description Checked rows across both classes.
             */
            checked: number;
            /**
             * Format: int64
             * @description Unchecked REQUIRED (template-derived) rows.
             */
            required_open: number;
            /**
             * Format: int64
             * @description All instance rows (template-derived + ad-hoc).
             */
            total: number;
        };
        /**
         * @description A board's pre-close checklist template. Exactly one per board
         *     (UNIQUE in storage): the deterministic gate consulted at close time.
         */
        ChecklistTemplate: {
            /** Format: uuid */
            board_id: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description One required step in a template. Instances on tickets link back to this
         *     id (`checklist_items.template_item_id`), so enforcement follows identity,
         *     never fragile text matching.
         */
        ChecklistTemplateItem: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            /** Format: int32 */
            sort_order: number;
            /** Format: uuid */
            template_id: string;
            text: string;
        };
        /**
         * @description GET /api/boards/{id}/checklist-template response: the template (if any),
         *     its items, and the enforcement posture actually applied at close time.
         */
        ChecklistTemplateView: {
            items: components["schemas"]["ChecklistTemplateItem"][];
            /** @description Mirrors `boards.require_checklist_close`. */
            require_checklist_close: boolean;
            template?: null | components["schemas"]["ChecklistTemplate"];
        };
        /** @description POST /imports/batches/{id}/chunks request body. */
        ChunkRequest: components["schemas"]["RowsPayload"] & {
            /**
             * Format: uuid
             * @description Optional saved field-mapping template applied before validation.
             */
            template_id?: string | null;
        };
        /**
         * @description u32-friendly mirror of [`core_domain::import_core::ChunkSummary`] so the
         *     generated SDK gets plain integers without overflow caveats.
         */
        ChunkSummaryWire: {
            /** Format: int32 */
            failed: number;
            /** Format: int32 */
            ok: number;
            /** Format: int32 */
            total: number;
            /** Format: int32 */
            would_skip: number;
            /** Format: int32 */
            would_update: number;
        };
        Client: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            name: string;
            notes?: string | null;
            /** Format: uuid */
            org_id: string;
            /**
             * @description Optional per-client visibility: when set, only these users (plus
             *     admins) can see the client. Empty = all org members.
             */
            restricted_to: string[];
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description Instant-first mutation response (IF2): client fields at the top level
         *     (unchanged shape) plus the additive `mutation` metadata.
         */
        ClientMutationResponse: components["schemas"]["Client"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        CommentListQuery: {
            kind?: null | components["schemas"]["NoteKind"];
        };
        ConflictCheckRequest: {
            /** Format: date-time */
            ends_at: string;
            /**
             * Format: uuid
             * @description Appointment excluded from the check (its own id when rescheduling).
             */
            exclude_appointment_id?: string | null;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            user_id: string;
        };
        ConflictCheckResponse: {
            conflicts: components["schemas"]["ConflictView"][];
            has_conflict: boolean;
        };
        /**
         * @description Per-entity conflict semantics when an external key already maps to a
         *     live row whose content differs from the incoming spec.
         * @enum {string}
         */
        ConflictPolicy: "skip" | "update" | "fail";
        /** @description One blocking booking named by the dry-run. */
        ConflictView: {
            /** Format: uuid */
            appointment_id: string;
            /** Format: date-time */
            ends_at: string;
            /** Format: date-time */
            starts_at: string;
            title: string;
        };
        Contact: {
            /** Format: uuid */
            client_id: string;
            /** Format: date-time */
            created_at: string;
            email?: string | null;
            /** Format: uuid */
            id: string;
            is_primary: boolean;
            name: string;
            /** Format: uuid */
            org_id: string;
            phone?: string | null;
            /** Format: uuid */
            site_id?: string | null;
            title?: string | null;
        };
        /**
         * @description A commercial agreement with one client. Money fields are integer minor
         *     units (cents for USD) — floats are forbidden near amounts (§5).
         */
        Contract: {
            /**
             * Format: int64
             * @description Per-period price (recurring/retainer), minor units.
             */
            amount_minor?: number | null;
            /**
             * Format: date
             * @description Required for recurring/retainer; the first billing period's start.
             */
            billing_anchor?: string | null;
            billing_frequency?: null | components["schemas"]["BillingFrequency"];
            /** Format: uuid */
            client_id: string;
            contract_type: components["schemas"]["ContractType"];
            /** Format: date-time */
            created_at: string;
            /** @description ISO 4217 code; every amount on the contract uses this currency. */
            currency: string;
            /**
             * Format: date
             * @description First day the agreement can generate billable candidates.
             */
            effective_from: string;
            /**
             * Format: date
             * @description Last day it can; `None` = open-ended.
             */
            effective_until?: string | null;
            /**
             * Format: int64
             * @description Default hourly rate for T&M when no rate card row matches the
             *     entry's `work_type`, minor units per hour.
             */
            hourly_rate_minor?: number | null;
            /** Format: uuid */
            id: string;
            /**
             * Format: int32
             * @description Retainer: labor minutes included in each period's base price.
             */
            included_minutes?: number | null;
            name: string;
            notes?: string | null;
            /** @description Human contract number, unique per org (e.g. "C-2026-014"). */
            number: string;
            /** Format: uuid */
            org_id: string;
            /**
             * Format: int64
             * @description Retainer: hourly rate for minutes beyond the included block.
             */
            overage_rate_minor?: number | null;
            status: components["schemas"]["ContractStatus"];
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description The contract aggregate as one view: the agreement plus its rate card
         *     and fee schedule.
         */
        ContractDetail: components["schemas"]["Contract"] & {
            fees: components["schemas"]["ContractFixedFee"][];
            rates: components["schemas"]["ContractRate"][];
        };
        /**
         * @description Fixed-fee schedule entry: becomes a candidate line when `due_on` falls
         *     inside the evaluation window (and the effective window).
         */
        ContractFixedFee: {
            /** Format: int64 */
            amount_minor: number;
            /** Format: uuid */
            contract_id: string;
            /** Format: date-time */
            created_at: string;
            /** Format: date */
            due_on: string;
            /** Format: uuid */
            id: string;
            label: string;
            notes?: string | null;
            /** Format: uuid */
            org_id: string;
        };
        ContractListQuery: {
            /** Format: uuid */
            client_id?: string | null;
            /**
             * Format: date
             * @description Only contracts effective on this date.
             */
            effective_on?: string | null;
            /** @description Exact status: `draft|active|ended|cancelled`. */
            status?: string | null;
        };
        /**
         * @description Instant-first mutation response (IF2): contract aggregate at the top
         *     level plus the additive `mutation` metadata.
         */
        ContractMutationResponse: components["schemas"]["ContractDetail"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description T&M rate card row: the hourly rate for time entries whose `work_type`
         *     matches `work_type`. Entries with no matching row bill at the
         *     contract's default `hourly_rate_minor`.
         */
        ContractRate: {
            /** Format: uuid */
            contract_id: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            /**
             * Format: int64
             * @description Hourly rate in minor units.
             */
            rate_minor: number;
            work_type: string;
        };
        /**
         * @description Lifecycle. `draft` contracts evaluate in previews (plan before
         *     activation) but Wave 9's invoice runs will refuse them; `cancelled`
         *     contracts never evaluate. `ended` keeps evaluating candidates whose
         *     dates fall inside the (closed) effective window.
         * @enum {string}
         */
        ContractStatus: "draft" | "active" | "ended" | "cancelled";
        /**
         * @description The four supported commercial agreement shapes.
         * @enum {string}
         */
        ContractType: "recurring" | "time_and_materials" | "retainer" | "fixed_fee";
        CreateAlertSourceRequest: components["schemas"]["MutationParams"] & {
            /**
             * Format: uuid
             * @description Board incident tickets land on.
             */
            board_id: string;
            /**
             * Format: uuid
             * @description Optional client context stamped onto created incident tickets.
             */
            client_id?: string | null;
            /**
             * Format: int32
             * @description Storm window in seconds (10..=86400; default 900).
             */
            correlation_window_secs?: number | null;
            name: string;
            /**
             * @description Optional custom routing id (3–64 chars of `[a-z0-9-]`); generated
             *     when absent. Globally unique — collisions are a typed 409.
             */
            source_ref?: string | null;
        };
        CreateApiKeyRequest: {
            name: string;
            /** @description Capability scopes from [`KNOWN_SCOPES`]. */
            scopes: string[];
        };
        CreateAppointmentRequest: {
            /** Format: uuid */
            client_id?: string | null;
            /** Format: date-time */
            ends_at: string;
            kind?: null | components["schemas"]["AppointmentKind"];
            notes?: string | null;
            /** Format: uuid */
            site_id?: string | null;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            ticket_id?: string | null;
            title: string;
            /**
             * Format: int32
             * @description Dispatch placeholder, stored verbatim (RoutingProvider comes later).
             */
            travel_minutes?: number;
            /**
             * Format: uuid
             * @description Assignee whose calendar holds the slot. Defaults to the caller;
             *     booking for someone else requires owner/admin.
             */
            user_id?: string | null;
        };
        CreateArticleRequest: components["schemas"]["MutationParams"] & {
            body?: string | null;
            /** Format: uuid */
            category_id?: string | null;
            /**
             * @description Portal v2 gate: effective client visibility additionally requires a
             *     published status and a client-visible category (core_kb rule).
             */
            client_visible?: boolean;
            /** @description Stable per-org handle; derived from the title when omitted. */
            slug?: string | null;
            status?: null | components["schemas"]["KbArticleStatus"];
            title: string;
        };
        CreateAssetRequest: {
            kind?: string | null;
            name: string;
            tag: string;
        };
        CreateAvailabilityRequest: {
            effective_from?: string | null;
            effective_until?: string | null;
            end_local: string;
            /** @description Local wall-clock "HH:MM" (window is `[start, end)`). */
            start_local: string;
            /** @description IANA timezone the wall-clock bounds live in. */
            timezone: string;
            /**
             * Format: uuid
             * @description Whose calendar. Defaults to the caller; another user requires
             *     owner/admin.
             */
            user_id?: string | null;
            /**
             * Format: int32
             * @description 0 = Monday … 6 = Sunday.
             */
            weekday: number;
        };
        /**
         * @description POST /api/imports/batches — opens a committed-run span. Chunks then land
         *     through `/chunks` (synchronous) or `/stage` + `/run` (async, D2
         *     remainder) until the batch finishes.
         */
        CreateBatchRequest: {
            conflict_policy: components["schemas"]["ConflictPolicy"];
            entity_kind: components["schemas"]["EntityKind"];
            label?: string | null;
            source_note?: string | null;
        };
        CreateBoardRequest: {
            description?: string | null;
            is_default: boolean;
            name: string;
        };
        CreateCalendarRequest: {
            /** @description Timezone/windows/holidays/maintenance document. */
            calendar: components["schemas"]["BusinessCalendar"];
            name: string;
        };
        CreateCategoryRequest: components["schemas"]["MutationParams"] & {
            /**
             * @description Portal v2 gate: only categories with this flag can expose articles
             *     to client users.
             */
            client_visible?: boolean;
            description?: string | null;
            name: string;
            /** Format: int32 */
            position?: number | null;
            /** @description Stable per-org handle; derived from the name when omitted. */
            slug?: string | null;
        };
        CreateClientRequest: components["schemas"]["MutationParams"] & {
            name: string;
            notes?: string | null;
        };
        CreateContactRequest: {
            email?: string | null;
            name: string;
            phone?: string | null;
            title?: string | null;
        };
        CreateContractRequest: components["schemas"]["MutationParams"] & {
            /**
             * Format: int64
             * @description Per-period price (recurring/retainer), minor units.
             */
            amount_minor?: number | null;
            /**
             * Format: date
             * @description Required for recurring/retainer; first billing period's start.
             */
            billing_anchor?: string | null;
            billing_frequency?: null | components["schemas"]["BillingFrequency"];
            /** Format: uuid */
            client_id: string;
            contract_type: components["schemas"]["ContractType"];
            /** @description ISO 4217 code (e.g. "USD"); every amount uses this currency. */
            currency: string;
            /** Format: date */
            effective_from: string;
            /** Format: date */
            effective_until?: string | null;
            /** @description Fee schedule (only valid on fixed_fee contracts). */
            fees?: components["schemas"]["FeeSpec"][];
            /**
             * Format: int64
             * @description T&M default hourly rate, minor units.
             */
            hourly_rate_minor?: number | null;
            /**
             * Format: int32
             * @description Retainer included minutes per period.
             */
            included_minutes?: number | null;
            name: string;
            notes?: string | null;
            number: string;
            /**
             * Format: int64
             * @description Retainer hourly overage rate, minor units.
             */
            overage_rate_minor?: number | null;
            /** @description T&M rate card (only valid on time_and_materials contracts). */
            rates?: components["schemas"]["RateSpec"][];
            status?: null | components["schemas"]["ContractStatus"];
        };
        CreateDocumentRequest: components["schemas"]["MutationParams"] & {
            body?: string | null;
            change_summary?: string | null;
            title: string;
        };
        CreateExclusionRequest: {
            /** Format: date-time */
            ends_at: string;
            reason?: string | null;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            user_id?: string | null;
        };
        CreateFieldDefRequest: {
            kind: components["schemas"]["FieldKindWire"];
            name: string;
            options?: string[];
            required?: boolean;
            /** Format: int32 */
            sort_order?: number;
        };
        CreateGroupRequest: {
            name: string;
            /** Format: int32 */
            position?: number | null;
        };
        CreateInvoiceRunRequest: components["schemas"]["MutationParams"] & {
            /**
             * Format: uuid
             * @description Restrict the run to one client's billable contracts.
             */
            client_id?: string | null;
            /**
             * Format: int32
             * @description Payment terms for the issued invoices, in days (default 30).
             */
            due_in_days?: number | null;
            /**
             * Format: date
             * @description Window start (inclusive), ISO date.
             */
            from: string;
            /**
             * Format: date
             * @description Window end (inclusive), ISO date.
             */
            to: string;
        };
        CreateOnCallRequest: {
            /** @description Rotation label ('primary', 'secondary', 'escalation', …). */
            category: string;
            /** Format: date-time */
            ends_at: string;
            notes?: string | null;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            user_id?: string | null;
        };
        CreatePhaseRequest: components["schemas"]["MutationParams"] & {
            description?: string | null;
            is_milestone?: boolean;
            /** Format: int64 */
            milestone_amount_minor?: number | null;
            milestone_currency?: string | null;
            name: string;
            /** Format: int32 */
            sort_order?: number;
        };
        CreatePortalUserRequest: {
            display_name: string;
            email: string;
            /**
             * @description Initial credential, hashed with argon2id before storage. The admin
             *     delivers it to the contact out-of-band (v1 has no invite email).
             */
            password: string;
            role: components["schemas"]["PortalRole"];
        };
        CreateProjectRequest: components["schemas"]["MutationParams"] & {
            billing_method?: components["schemas"]["BillingMethod"];
            /** Format: int64 */
            budget_minor?: number | null;
            /** Format: uuid */
            client_id?: string | null;
            currency?: string | null;
            description?: string | null;
            /** Format: date */
            due_date?: string | null;
            name: string;
            /** Format: date */
            start_date?: string | null;
        };
        /** @description POST /api/agent-proposals body. */
        CreateProposalRequest: {
            /**
             * @description Agent provenance (§5). Optional: the machine identity defaults to
             *     the caller's (`api-key:<id>` / `user:<id>`).
             */
            agent_ref?: string | null;
            /**
             * @description The value the agent saw when it drafted the change (the IF8
             *     before-snapshot). `null` for creates. Application re-reads the
             *     live entity, so this is decision context, not a blind write.
             */
            before_value?: Record<string, never> | null;
            /** @description Write capability this change belongs to (must be granted). */
            capability?: string | null;
            classification?: components["schemas"]["ProposalClassification"];
            entity: string;
            /** Format: uuid */
            entity_id?: string | null;
            /**
             * Format: uuid
             * @description The user the agent acted on behalf of.
             */
            for_user?: string | null;
            op: string;
            proposed_value: Record<string, never>;
            rationale?: string | null;
            /** @description Hash of the evaluated rule set at proposal time. */
            rule_set_hash?: string | null;
            session_id?: string | null;
            skill_slug?: string | null;
            /** Format: int32 */
            skill_version?: number | null;
            tool?: string | null;
        };
        /** @description POST /api/agent-proposals response: the stored proposal. */
        CreateProposalResponse: {
            proposal: components["schemas"]["AgentProposalView"];
        };
        /** @description POST /api/report-definitions body. */
        CreateReportDefinitionRequest: components["schemas"]["MutationParams"] & {
            description?: string | null;
            /** @description Org-unique display name (1–200 chars after trimming). */
            name: string;
            scope?: null | components["schemas"]["DefinitionScope"];
            /** @description The registry-validated report spec. */
            spec: components["schemas"]["ReportSpec"];
        };
        /** @description POST /api/report-schedules body. */
        CreateReportScheduleRequest: components["schemas"]["MutationParams"] & {
            cadence: components["schemas"]["ScheduleCadence"];
            /**
             * Format: int32
             * @description Monthly only: 1..=28.
             */
            day_of_month?: number | null;
            /**
             * Format: int32
             * @description Weekly only: 0 = Monday .. 6 = Sunday.
             */
            day_of_week?: number | null;
            enabled?: boolean | null;
            /** @description Org-unique display name (1–200 chars after trimming). */
            name: string;
            /** @description Recipient staff users of this org (1–50). */
            recipients: string[];
            /**
             * Format: uuid
             * @description The stored R3 definition this schedule delivers.
             */
            report_definition_id: string;
            /** @description `HH:MM` or `HH:MM:SS`, read in `timezone`. */
            time_of_day: string;
            /** @description IANA zone the wall clock reads in. */
            timezone: string;
            /**
             * Format: int32
             * @description Trailing-day window per run (1..=366); default 30.
             */
            window_days?: number | null;
        };
        CreateRuleRequest: {
            /** @description Action documents (see AutomationAction schema). */
            actions: unknown;
            /** Format: uuid */
            board_id?: string | null;
            /** @description Condition documents (see AutomationCondition schema). */
            conditions: unknown;
            name: string;
            /** @description Domain event type, e.g. "ticket.created". */
            trigger_event: string;
        };
        CreateSiteRequest: {
            address?: string | null;
            city?: string | null;
            country?: string | null;
            /** Format: double */
            latitude?: number | null;
            /** Format: double */
            longitude?: number | null;
            name: string;
            postal_code?: string | null;
            region?: string | null;
            timezone?: string | null;
        };
        CreateStatusRequest: {
            /**
             * @description The semantic category this status binds to — behavior (SLA pauses,
             *     billing, automations) reads the category, never the label (§5).
             */
            category: components["schemas"]["TicketStatusCategory"];
            is_entry: boolean;
            name: string;
            /** Format: int32 */
            sort_order: number;
        };
        CreateTaskRequest: components["schemas"]["MutationParams"] & {
            /** Format: uuid */
            assignee_id?: string | null;
            /**
             * @description Finish-to-start dependencies to create with the task; every id must
             *     be a task of the SAME project. Cycles are impossible at creation
             *     (nothing can depend on the not-yet-existing task) but membership is
             *     validated.
             */
            depends_on?: string[];
            description?: string | null;
            /** Format: date-time */
            due_at?: string | null;
            is_milestone?: boolean;
            /** Format: int64 */
            milestone_amount_minor?: number | null;
            milestone_currency?: string | null;
            /** Format: uuid */
            phase_id?: string | null;
            /** Format: int32 */
            sort_order?: number;
            title: string;
        };
        CreateTemplateRequest: {
            description?: string | null;
            name: string;
            /**
             * @description Versioned template spec (core-projects `template`, spec_version 1).
             *     Validated eagerly so broken blueprints never persist.
             */
            spec: unknown;
        };
        CreateTicketRequest: components["schemas"]["MutationParams"] & {
            /** Format: uuid */
            assignee_id?: string | null;
            /** Format: uuid */
            board_id: string;
            body?: string | null;
            /** Format: uuid */
            client_id?: string | null;
            /** Format: uuid */
            contact_id?: string | null;
            /**
             * @description Initial custom-field values; each is validated against its
             *     definition. REQUIRED definitions must all be present (TASKLIST 2.17).
             */
            custom_fields?: components["schemas"]["TicketFieldInput"][];
            /** Format: date-time */
            due_at?: string | null;
            priority: components["schemas"]["Priority"];
            /** Format: uuid */
            site_id?: string | null;
            title: string;
        };
        CreateTimeEntryRequest: {
            billable: boolean;
            /** Format: date-time */
            ended_at: string;
            notes?: string | null;
            /** Format: date-time */
            started_at: string;
            /** Format: uuid */
            ticket_id: string;
            work_type?: string | null;
        };
        CreateViewRequest: {
            is_default?: boolean;
            name: string;
            /**
             * Format: uuid
             * @description `None` = org-wide view (spans every project).
             */
            project_id?: string | null;
            /**
             * @description Versioned, additive-only view spec (filter/group/sort). Consumers
             *     tolerate unknown fields.
             */
            spec?: unknown;
        };
        CreateWebhookRequest: {
            /** @description Exact event types to subscribe; empty subscribes to everything. */
            event_types?: string[];
            /** @description HTTPS endpoint receiving `POST`s. (http:// allowed for local dev.) */
            url: string;
        };
        /** @description The customization manifest envelope, `manifest_version` 1. */
        CustomizationManifest: {
            /**
             * Format: date-time
             * @description Dump instant (excluded from determinism comparisons).
             */
            generated_at: string;
            /**
             * Format: int32
             * @description Envelope schema version. Bump ONLY for breaking section reshapes.
             */
            manifest_version: number;
            /**
             * Format: uuid
             * @description The org this manifest documents.
             */
            org_id: string;
            /** @description The customization sections. Always fully serialized. */
            sections: components["schemas"]["ManifestSections"];
        };
        DayWeekQuery: {
            /**
             * @description Local date in the TARGET user's timezone. Day view: the day itself.
             *     Week view: any date inside the week (Monday-start). Defaults to
             *     today in that timezone.
             */
            date?: string | null;
            /**
             * Format: uuid
             * @description Whose calendar. Defaults to the caller; another user requires
             *     owner/admin.
             */
            user_id?: string | null;
        };
        DecideProposalRequest: {
            note?: string | null;
        };
        /**
         * @description Wire shape of one declaration. Kinds/options are flattened so generated
         *     clients can render editors directly from this payload.
         */
        DeclarationView: {
            default: components["schemas"]["SettingValue"];
            description: string;
            key: string;
            kind: components["schemas"]["SettingKind"];
            label: string;
            /** @description Closed option set for enumerations; empty otherwise. */
            options: string[];
            /** @description "org" | "user" */
            scope: string;
            /** @description Settings-hub section this declaration renders under. */
            section: string;
            /** @description "ui" renders an editable row; "hidden" is state readable via values. */
            visibility: string;
        };
        /**
         * @description Visibility scope of a stored definition: `org` (published org-wide,
         *     admins only) or `self` (private to the creator).
         * @enum {string}
         */
        DefinitionScope: "org" | "self";
        DeliveryAttemptView: {
            /** Format: int32 */
            attempt: number;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            delivered_at?: string | null;
            /** Format: uuid */
            event_id: string;
            event_type: string;
            /** Format: uuid */
            id: string;
            last_error?: string | null;
            /** Format: date-time */
            next_retry_at?: string | null;
            /**
             * Format: int32
             * @description HTTP status when a response arrived; null for transport failures.
             */
            response_status?: number | null;
        };
        DeliveryPage: {
            deliveries: components["schemas"]["DeliveryAttemptView"][];
            /** @description Opaque keyset cursor for the next page; absent when exhausted. */
            next_cursor?: string | null;
        };
        DeliveryQuery: {
            cursor?: string | null;
            /** Format: int64 */
            limit?: number;
        };
        /**
         * @description A task's direct finish-to-start dependencies (full rows, so the client
         *     renders "blocked by" without extra fetches).
         */
        DependencyListResponse: {
            depends_on: components["schemas"]["ProjectTask"][];
            /** Format: uuid */
            project_id: string;
            /** Format: uuid */
            task_id: string;
        };
        DeviceAsset: {
            /** Format: uuid */
            client_id: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            kind: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            site_id?: string | null;
            status: string;
            /** @description Human asset tag, e.g. "EXCH-01". */
            tag: string;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description One booking block on the dispatch board: the appointment plus the
         *     ticket number and client name the block face renders.
         */
        DispatchAppointment: {
            /** Format: uuid */
            client_id?: string | null;
            client_name?: string | null;
            /** Format: date-time */
            ends_at: string;
            /** Format: uuid */
            id: string;
            kind: components["schemas"]["AppointmentKind"];
            /** Format: date-time */
            starts_at: string;
            status: components["schemas"]["AppointmentStatus"];
            /** Format: uuid */
            ticket_id?: string | null;
            ticket_number?: string | null;
            title: string;
            /** Format: int32 */
            travel_minutes: number;
            /** Format: uuid */
            user_id: string;
        };
        /**
         * @description One dispatch-path query: `date` is a LOCAL date interpreted as the
         *     UTC day window (documented UTC scope).
         */
        DispatchDayQuery: {
            /**
             * Format: date
             * @description The day to assemble; defaults to today (UTC).
             */
            date?: string | null;
        };
        /** @description The assembled day-view payload: one request, one render. */
        DispatchDayView: {
            /** Format: date */
            date: string;
            /** Format: date-time */
            from: string;
            technicians: components["schemas"]["TechnicianView"][];
            /** Format: date-time */
            to: string;
            /**
             * @description Unassigned, non-closed tickets in dispatch order (priority rank,
             *     then effective SLA/due deadline).
             */
            unassigned: components["schemas"]["MyWorkItem"][];
        };
        /**
         * @description A document's living handle. No content lives here — content is only
         *     ever a [`DocumentVersion`]; `current_version` names the newest one.
         */
        Document: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by?: string | null;
            /**
             * Format: int32
             * @description The newest version number in the document's history.
             */
            current_version: number;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            title: string;
            /** Format: date-time */
            updated_at: string;
            /** Format: uuid */
            updated_by?: string | null;
        };
        DocumentListQuery: {
            /** @description Title substring narrowing; ranked full-text search rides /api/search. */
            q?: string | null;
        };
        /**
         * @description Instant-first mutation response (IF2): document fields at the top
         *     level plus the additive `mutation` metadata.
         */
        DocumentMutationResponse: components["schemas"]["Document"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description One immutable snapshot in a document's append-only history. Every
         *     version stores its complete title + body, so any pair is diffable
         *     without reconstructing intermediate state.
         */
        DocumentVersion: {
            /** Format: uuid */
            author_id?: string | null;
            body?: string;
            /** @description Optional human explanation of what changed in this version. */
            change_summary?: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            document_id: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            /**
             * Format: int32
             * @description Set when this version was created by a RESTORE: the historical
             *     version whose content was copied. Restore-creates-new-version is
             *     the only way old content comes back — history is never rewritten.
             */
            restored_from_version?: number | null;
            title: string;
            /**
             * Format: int32
             * @description 1-based, gap-free per document.
             */
            version: number;
        };
        /**
         * @description GET /api/documents/{id} response: the living handle plus the CURRENT
         *     version's content (read-your-write on the body the last write stored).
         */
        DocumentView: components["schemas"]["Document"] & {
            body: string;
            change_summary: string;
            /** Format: date-time */
            content_updated_at: string;
            /** Format: int32 */
            restored_from_version?: number | null;
        };
        /**
         * @description POST /api/imports/dry-run request: validate a full payload and report
         *     what each row WOULD do before any data write. The probe is recorded as a
         *     finished dry-run batch for operator history.
         */
        DryRunRequest: components["schemas"]["RowsPayload"] & {
            conflict_policy: components["schemas"]["ConflictPolicy"];
            entity_kind: components["schemas"]["EntityKind"];
            /** @description Free-form run label recorded with the dry-run history entry. */
            label?: string | null;
            /**
             * Format: uuid
             * @description Optional saved field-mapping template: source headers are renamed to
             *     the template's canonical columns before validation (D2 remainder).
             */
            template_id?: string | null;
        };
        /**
         * @description Collections posture of an issued invoice. Forward-only: states are
         *     data moved by explicit API writes (events + write trail), never a
         *     background side effect.
         * @enum {string}
         */
        DunningState: "none" | "reminder_sent" | "overdue" | "final_notice" | "collections";
        DunningTransitionRequest: components["schemas"]["MutationParams"] & {
            note?: string | null;
            /**
             * @description The target state; must be strictly forward on the ladder
             *     (skipping rungs is allowed).
             */
            to_state: components["schemas"]["DunningState"];
        };
        EffectsRequest: {
            /** @description AutomationAction documents, parsed and validated via core-domain. */
            actions: unknown;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            rule_id?: string | null;
            /** Format: uuid */
            ticket_id: string;
        };
        /**
         * @description Which entity family a batch targets. One batch = exactly one kind; the
         *     parent-before-child ordering across batches keeps foreign references
         *     resolvable without cross-batch transactions. D2 remainder extends the
         *     original client/site/contact family with boards, tickets, and time
         *     entries: [`Self::import_order`] is the documented run-script order.
         * @enum {string}
         */
        EntityKind: "client" | "site" | "contact" | "board" | "ticket" | "time_entry" | "kb_category" | "kb_article";
        /** @description One escalation ladder step as stored (`sla_escalation_steps`). */
        EscalationStep: {
            /** Format: int64 */
            at_pct: number;
            enabled: boolean;
            /** Format: uuid */
            id: string;
            /** @description Extra recipients beyond the ticket assignee. */
            notify_user_ids: string[];
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            policy_id: string;
            /** Format: int32 */
            step_index: number;
            /** @description 'risk' | 'breach' | 'escalate' — bound to existing notification kinds. */
            trigger_kind: string;
        };
        /** @description The org-data export envelope, `bundle_version` 1. */
        ExportBundle: {
            /**
             * Format: int32
             * @description Envelope schema version. Bump ONLY for breaking section reshapes.
             */
            bundle_version: number;
            /**
             * Format: date-time
             * @description Dump instant (excluded from determinism comparisons).
             */
            generated_at: string;
            /**
             * Format: uuid
             * @description The org this bundle documents.
             */
            org_id: string;
            /** @description The export sections. Always fully serialized. */
            sections: components["schemas"]["ExportSections"];
        };
        /** @description One client company row. */
        ExportClient: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            name: string;
            notes?: string | null;
            /**
             * @description Optional visibility restriction: when non-empty, only these users
             *     (plus admins) can see the client. Opaque user-id references.
             */
            restricted_to: string[];
            /** Format: date-time */
            updated_at: string;
        };
        /** @description One ticket comment. */
        ExportComment: {
            /** Format: uuid */
            author_id: string | null;
            body: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            /** @description Structured note taxonomy: internal | client_visible | in_house_update. */
            kind: components["schemas"]["NoteKind"];
        };
        /** @description One contact row; `client_id`/`site_id` reference this bundle. */
        ExportContact: {
            /** Format: uuid */
            client_id: string;
            /** Format: date-time */
            created_at: string;
            email?: string | null;
            /** Format: uuid */
            id: string;
            is_primary: boolean;
            name: string;
            phone?: string | null;
            /** Format: uuid */
            site_id: string | null;
            title?: string | null;
        };
        ExportCreatedResponse: {
            /**
             * Format: int32
             * @description Envelope version of the produced bundle (mirrors the body).
             */
            bundle_version: number;
            /**
             * @description Self-contained relative URL carrying the signature + expiry; the
             *     client prefixes its API origin. Works without a bearer token.
             */
            download_url: string;
            /** Format: date-time */
            expires_at: string;
            /** Format: uuid */
            export_id: string;
            /** @description sha256 of the stored bytes (download integrity check). */
            sha256: string;
            /**
             * Format: int64
             * @description Exact bundle byte size as stored.
             */
            size_bytes: number;
        };
        /** @description One typed custom-field value attached to any entity row. */
        ExportFieldValue: {
            /**
             * @description Field-definition family the value belongs to (ticket | client |
             *     contact | asset) — resolved from the definition at load time.
             */
            entity: string;
            /** Format: uuid */
            entity_id: string;
            /** Format: uuid */
            field_id: string;
            /** @description The typed value exactly as stored (JSONB passthrough). */
            value: Record<string, never>;
        };
        /** @description Org profile identity. */
        ExportOrg: {
            /** Format: date-time */
            created_at: string;
            name: string;
            slug: string;
            /** @description Human ticket-number prefix (e.g. "MER"). */
            ticket_prefix: string;
        };
        /**
         * @description Section container — all fields always present; empty org state is an
         *     empty array, never an absent key.
         */
        ExportSections: {
            clients: components["schemas"]["ExportClient"][];
            contacts: components["schemas"]["ExportContact"][];
            /**
             * @description Every custom-field VALUE in the org, flat across entity families —
             *     the data plane the U5 manifest deliberately excludes. `entity` names
             *     the family the field definition attaches to (ticket | client |
             *     contact | asset); `entity_id` is the owning row.
             */
            custom_field_values: components["schemas"]["ExportFieldValue"][];
            /**
             * @description The U5 customization-manifest sections, embedded verbatim — D1 is
             *     the manifest's generalization, not its replacement.
             */
            customization: components["schemas"]["ManifestSections"];
            /**
             * @description Org profile identity (volatile counters excluded — a restore
             *     reseeds the ticket-number sequence from the imported tickets).
             */
            org: components["schemas"]["ExportOrg"];
            sites: components["schemas"]["ExportSite"][];
            /**
             * @description Tickets with their comments nested. Parents (client/site/contact)
             *     are referenced by id into the sections above.
             */
            tickets: components["schemas"]["ExportTicket"][];
            time_entries: components["schemas"]["ExportTimeEntry"][];
        };
        /** @description One site row; `client_id` references `clients` in this bundle. */
        ExportSite: {
            address?: string | null;
            city?: string | null;
            /** Format: uuid */
            client_id: string;
            country?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            /** Format: double */
            latitude?: number | null;
            /** Format: double */
            longitude?: number | null;
            name: string;
            postal_code?: string | null;
            region?: string | null;
            timezone?: string | null;
        };
        /**
         * @description One ticket with its comment thread nested. Authorship is an opaque
         *     user-id reference; the bundle never carries user records.
         */
        ExportTicket: {
            /** Format: uuid */
            assignee_id: string | null;
            /** Format: uuid */
            board_id: string;
            body?: string | null;
            /**
             * @description Semantic category of the captured status (behavior binds to this,
             *     never to the status label).
             */
            category: components["schemas"]["TicketStatusCategory"];
            /** Format: uuid */
            client_id: string | null;
            comments: components["schemas"]["ExportComment"][];
            /** Format: uuid */
            contact_id: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            due_at?: string | null;
            /** Format: uuid */
            id: string;
            /** @description Human number (e.g. "MER-12") — the bundle's stable ticket key. */
            number: string;
            priority: components["schemas"]["Priority"];
            /** Format: uuid */
            requester_id: string | null;
            /** Format: date-time */
            resolved_at?: string | null;
            /** Format: uuid */
            site_id: string | null;
            /** Format: uuid */
            status_id: string;
            title: string;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description One time entry; `ticket_id` and `user_id` reference this bundle's
         *     tickets and the org's live users respectively.
         */
        ExportTimeEntry: {
            billable: boolean;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            ended_at: string;
            /** Format: uuid */
            id: string;
            /** Format: int64 */
            minutes: number;
            notes?: string | null;
            /** Format: date-time */
            started_at: string;
            /** Format: uuid */
            ticket_id: string;
            /** Format: uuid */
            user_id: string;
            work_type?: string | null;
        };
        FavoriteRequest: {
            /** Format: uuid */
            board_id: string;
            favorite: boolean;
        };
        /** @description Fixed-fee schedule entry spec. */
        FeeSpec: {
            /** Format: int64 */
            amount_minor: number;
            /** Format: date */
            due_on: string;
            label: string;
            notes?: string | null;
        };
        /**
         * @description Wire view of one field definition; choice kinds carry their closed
         *     option set so generated clients can render editors directly.
         */
        FieldDefView: {
            /**
             * Format: uuid
             * @description Owning board (`None` = org-wide; v1 always creates board-scoped).
             */
            board_id?: string | null;
            entity: string;
            /** Format: uuid */
            id: string;
            kind: string;
            name: string;
            /** @description Closed option set for select/multiselect; empty otherwise. */
            options: string[];
            /** Format: uuid */
            org_id: string;
            required: boolean;
            /** Format: int32 */
            sort_order: number;
        };
        /**
         * @description The definition kinds (mirror of migration 0009's CHECK set). Kept as a
         *     string-newtype wire enum so unknown kinds fail validation, not deserialization.
         * @enum {string}
         */
        FieldKindWire: "text" | "number" | "date" | "select" | "multiselect" | "bool" | "url" | "user";
        /**
         * @description Filter operators supported by the v1 compiled reports. Operand typing is
         *     enforced through [`ValueType`] matches in the builder, so e.g. a bool
         *     operand can never reach a timestamp column. The serde encoding is the
         *     `snake_case` slug (`lt_eq`, `is_null`, …) stored specs carry.
         * @enum {string}
         */
        FilterOp: "eq" | "not_eq" | "lt" | "lt_eq" | "gt" | "gt_eq" | "is_null" | "is_not_null";
        /**
         * @description A typed filter operand. The variant must match the catalog
         *     [`ValueType`] of the filtered column (`Int` covers both Int and BigInt
         *     columns) — enforced by [`ReportSpec::validate`], so a mismatched
         *     operand can never reach a bind. The serde encoding is the externally
         *     tagged slug (`{"text": "p1"}`, `{"bool": true}`).
         */
        FilterValue: {
            /** Format: uuid */
            uuid: string;
        } | {
            text: string;
        } | {
            /**
             * Format: date-time
             * @description RFC 3339 instant (chrono serde encoding).
             */
            timestamp: string;
        } | {
            /**
             * Format: date
             * @description `YYYY-MM-DD`.
             */
            date: string;
        } | {
            /** Format: int64 */
            int: number;
        } | {
            bool: boolean;
        };
        GapDayView: {
            activity: boolean;
            /** @description Local calendar day (YYYY-MM-DD). */
            day: string;
            /** Format: int64 */
            expected_minutes: number;
            has_gap: boolean;
            /** Format: int64 */
            logged_minutes: number;
            /**
             * Format: date-time
             * @description When the EOD run nudged for this day, if it did.
             */
            nudge_sent_at?: string | null;
            /** Format: int64 */
            unaccounted_minutes: number;
        };
        GapReportQuery: {
            /**
             * Format: date
             * @description Inclusive local-day start (YYYY-MM-DD). Defaults to yesterday.
             */
            from?: string | null;
            /**
             * Format: date
             * @description Inclusive local-day end (YYYY-MM-DD). Defaults to `from`.
             */
            to?: string | null;
            /**
             * Format: uuid
             * @description Manager report only: whose days. Requires owner/admin.
             */
            user_id?: string | null;
        };
        GapReportResponse: {
            days: components["schemas"]["GapDayView"][];
            /** Format: int64 */
            expected_minutes: number;
            timezone: string;
            /** Format: uuid */
            user_id: string;
        };
        HealthResponse: {
            status: string;
        };
        /** @description The storm group behind an incident ticket. */
        IncidentView: {
            /**
             * Format: int32
             * @description How many alerts have collapsed into this ticket so far.
             */
            alert_count: number;
            first_alert_at: string;
            last_alert_at: string;
            /** Format: uuid */
            ticket_id: string;
            ticket_number: string;
        };
        IngestAlertRequest: {
            /** @description Provider-specific structured context (must be a JSON object). */
            attributes?: unknown;
            /**
             * @description Storm grouping key; defaults to `dedup_key` when absent so
             *     uncorrelated alerts each anchor their own incident.
             */
            correlation_key?: string | null;
            /** @description Provider's per-event identity. REQUIRED: this is the replay gate. */
            dedup_key: string;
            description?: string | null;
            /**
             * Format: date-time
             * @description When the condition started (defaults to receipt time).
             */
            occurred_at?: string | null;
            severity: components["schemas"]["AlertSeverity"];
            title: string;
        };
        /** @description POST /api/alerts/ingest — outcome of one signed delivery. */
        IngestResponse: {
            alert: components["schemas"]["AlertView"];
            incident?: null | components["schemas"]["IncidentView"];
            /**
             * @description `incident_created` (this alert anchored a NEW incident ticket),
             *     `collapsed` (folded into the still-open storm incident), or
             *     `duplicate` (replayed dedup_key — the stored alert is returned,
             *     nothing was written).
             */
            status: string;
        };
        InstantiateTemplateRequest: components["schemas"]["MutationParams"] & {
            /**
             * @description Resolves template `assignee_key`s to user ids at instantiation
             *     time; unmapped keys simply leave those tasks unassigned.
             */
            assignee_map?: {
                [key: string]: string;
            };
            /** Format: uuid */
            client_id?: string | null;
            /** Format: date */
            due_date?: string | null;
            name: string;
            /** Format: date */
            start_date?: string | null;
        };
        /**
         * @description The billing document. Derives from exactly one contract; `currency`
         *     and `total_minor` come from the contract's evaluation (line sum).
         */
        Invoice: {
            /** Format: uuid */
            client_id: string;
            /** Format: uuid */
            contract_id: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by?: string | null;
            /** @description ISO 4217 code copied from the contract; every amount uses it. */
            currency: string;
            /** Format: date */
            due_on: string;
            dunning_note?: string | null;
            dunning_state: components["schemas"]["DunningState"];
            /** Format: date-time */
            dunning_updated_at?: string | null;
            /** Format: uuid */
            id: string;
            /** Format: date */
            issued_at: string;
            notes?: string | null;
            /** @description Human invoice number, unique per org (e.g. "INV-000042"). */
            number: string;
            /** Format: uuid */
            org_id: string;
            /**
             * Format: date
             * @description The billed period, when the contract type has one (recurring /
             *     retainer / T&M window slice); `None` for fixed-fee schedules.
             */
            period_from?: string | null;
            /** Format: date */
            period_to?: string | null;
            /**
             * Format: uuid
             * @description The run that created this invoice (`None` for future manual invoices).
             */
            run_id?: string | null;
            status: components["schemas"]["InvoiceStatus"];
            /**
             * Format: int64
             * @description Sum of line amounts, minor units.
             */
            total_minor: number;
            /**
             * Format: int64
             * @description Recorded payment total, minor units.
             */
            total_paid_minor: number;
            /** Format: date-time */
            updated_at: string;
        };
        /** @description The invoice aggregate: document, lines, and payment receipts. */
        InvoiceDetail: components["schemas"]["Invoice"] & {
            lines: components["schemas"]["InvoiceLine"][];
            payments: components["schemas"]["InvoicePayment"][];
        };
        /**
         * @description One persisted billing line; mirrors the engine's `BillingCandidateLine`
         *     (kind reuse via [`crate::entities::BillingLineKind`]) minus the
         *     denormalized contract/client names.
         */
        InvoiceLine: {
            /** Format: int64 */
            amount_minor: number;
            /** Format: uuid */
            contract_id: string;
            description: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            invoice_id: string;
            kind: components["schemas"]["BillingLineKind"];
            /** Format: uuid */
            org_id: string;
            /** Format: date */
            period_end: string;
            /** Format: date */
            period_start: string;
            /** Format: int32 */
            position: number;
            /**
             * Format: int64
             * @description Labor minutes on the line (T&M / overage); 0 for amount lines.
             */
            quantity_minutes: number;
            /** Format: int64 */
            unit_amount_minor: number;
        };
        InvoiceListQuery: {
            /** Format: uuid */
            client_id?: string | null;
            /** Format: uuid */
            contract_id?: string | null;
            /**
             * @description Exact dunning state:
             *     `none|reminder_sent|overdue|final_notice|collections`.
             */
            dunning_state?: string | null;
            /** @description Exact status: `draft|issued|paid|void`. */
            status?: string | null;
        };
        /** @description Instant-first mutation response (IF2) for invoice mutations. */
        InvoiceMutationResponse: components["schemas"]["Invoice"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description A payment receipt recorded through the `PaymentProvider` seam. The
         *     provider stays a string (`none` in v1 — the built-in provider approves
         *     locally without external PSP calls).
         */
        InvoicePayment: {
            /** Format: int64 */
            amount_minor: number;
            /** Format: uuid */
            created_by?: string | null;
            currency: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            invoice_id: string;
            notes?: string | null;
            /** Format: uuid */
            org_id: string;
            provider: string;
            provider_ref?: string | null;
            /** Format: date-time */
            received_at: string;
        };
        /**
         * @description The invoice-run batch record: one evaluation sweep over a window and
         *     the roll-up of the invoices it created.
         */
        InvoiceRun: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by?: string | null;
            /**
             * @description Currency of the summed total; `None` when the run created no
             *     invoices (mixed currencies are rejected before any write).
             */
            currency?: string | null;
            /** Format: int32 */
            due_in_days: number;
            /** Format: uuid */
            id: string;
            /** Format: int32 */
            invoice_count: number;
            /** @description Human run number, unique per org (e.g. "RUN-000007"). */
            number: string;
            /** Format: uuid */
            org_id: string;
            /** Format: date */
            period_from: string;
            /** Format: date */
            period_to: string;
            /** Format: int64 */
            total_minor: number;
        };
        /** @description Instant-first mutation response (IF2) for an invoice run. */
        InvoiceRunResponse: {
            invoices: components["schemas"]["IssuedInvoice"][];
            mutation: components["schemas"]["MutationMeta"];
            run: components["schemas"]["InvoiceRun"];
        };
        /**
         * @description Document lifecycle of an invoice.
         * @enum {string}
         */
        InvoiceStatus: "draft" | "issued" | "paid" | "void";
        /** @description One issued invoice: the document plus its lines. */
        IssuedInvoice: {
            invoice: components["schemas"]["Invoice"];
            lines: components["schemas"]["InvoiceLine"][];
        };
        /** @description A KB article. */
        KbArticle: {
            body?: string;
            /**
             * Format: uuid
             * @description Survives category deletion (the FK is ON DELETE SET NULL).
             */
            category_id?: string | null;
            /** @description The article half of the client-facing gate (portal v2 honors it). */
            client_visible: boolean;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by?: string | null;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            /** @description Stable per-org handle (unique). */
            slug: string;
            status: components["schemas"]["KbArticleStatus"];
            title: string;
            /** Format: date-time */
            updated_at: string;
            /** Format: uuid */
            updated_by?: string | null;
        };
        /**
         * @description Article lifecycle. Closed vocabulary so filtering and the client-facing
         *     rule stay deterministic: `draft` (staff authoring), `published`
         *     (readable, searchable), `archived` (retired, out of the active tree —
         *     content edits are refused while archived; unarchive to edit).
         * @enum {string}
         */
        KbArticleStatus: "draft" | "published" | "archived";
        /** @description A KB category: grouping + ordering for the article tree. */
        KbCategory: {
            /**
             * @description The category half of the client-facing gate: an article is visible
             *     to portal clients only when this AND the article flag are set (and
             *     the article is published).
             */
            client_visible: boolean;
            /** Format: date-time */
            created_at: string;
            description?: string;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: int32 */
            position: number;
            /**
             * @description Stable per-org handle (unique); portals and imports reference this,
             *     never the row id.
             */
            slug: string;
            /** Format: date-time */
            updated_at: string;
        };
        LadderStepSpec: {
            /**
             * Format: int64
             * @description Fires when consumed budget reaches this percent (1..=500).
             */
            at_pct: number;
            enabled?: boolean;
            notify_user_ids: string[];
            /** Format: int32 */
            step_index: number;
            /** @description 'risk' | 'breach' | 'escalate' (notification-center kinds). */
            trigger_kind: string;
        };
        LinkBeginResponse: {
            authorize_url: string;
        };
        ListEntriesQuery: {
            /** Format: int64 */
            limit?: number;
            /** Format: date-time */
            since?: string | null;
            /** Format: uuid */
            ticket_id?: string | null;
            /** Format: uuid */
            user_id?: string | null;
        };
        LoginRequest: {
            email: string;
            password: string;
        };
        /**
         * @description Login outcome: tokens directly, or a short-lived MFA bridge token when
         *     any second factor is enrolled. Untagged so legacy clients that ignore
         *     `mfa_required` keep parsing the plain token pair they always got.
         */
        LoginResponse: components["schemas"]["TokenPair"] | {
            /** @description Second factors able to satisfy this login. */
            methods: string[];
            mfa_required: boolean;
            mfa_token: string;
        };
        /**
         * @description Planned downtime: an absolute UTC interval during which SLA clocks do
         *     not consume business-time budget (vendor maintenance, change freezes).
         */
        MaintenanceWindow: {
            /** Format: date-time */
            ends_at: string;
            name: string;
            /** Format: date-time */
            starts_at: string;
        };
        /**
         * @description API-key metadata. `key_hash` (the credential verifier) and the raw key
         *     material are structurally absent — only the display prefix survives.
         */
        ManifestApiKey: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by: string | null;
            /** Format: uuid */
            id: string;
            /** Format: date-time */
            last_used_at?: string | null;
            name: string;
            /** @description Non-secret display identifier (e.g. `mrd_ab12cd34`). */
            prefix: string;
            /** Format: date-time */
            revoked_at?: string | null;
            /** @description Capability scope strings ("tickets:read", …). */
            scopes: string[];
        };
        /** @description One event-triggered automation rule. */
        ManifestAutomationRule: {
            actions: Record<string, never>;
            /**
             * Format: uuid
             * @description Board the rule is scoped to; null = every board in the org.
             */
            board_id: string | null;
            conditions: Record<string, never>;
            enabled: boolean;
            /** Format: uuid */
            id: string;
            name: string;
            /** @description Domain event type from the envelope, e.g. `ticket.created`. */
            trigger_event: string;
        };
        /** @description One board's workflow customization. */
        ManifestBoard: {
            /** Format: uuid */
            id: string;
            /** @description The org's default intake board. */
            is_default: boolean;
            name: string;
            /** @description Pre-close checklist enforcement toggle (workflow discipline flag). */
            require_checklist_close: boolean;
            /**
             * @description Custom statuses; ALWAYS one per semantic category (defaults-first
             *     seeding guarantees category coverage, never removed by edits).
             */
            statuses: components["schemas"]["ManifestBoardStatus"][];
            /**
             * @description Explicit transition adjacency between this board's statuses. Empty
             *     means the semantic-category floor applies (any→any except identity
             *     and closed→open), not "no transitions".
             */
            transitions: components["schemas"]["ManifestTransition"][];
        };
        /**
         * @description One custom status, bound to its semantic category. Renames and colors
         *     never change behavior — consumers must branch on `category`, not `name`.
         */
        ManifestBoardStatus: {
            /**
             * @description Semantic category: open | in_progress | waiting | closed. THE
             *     binding every SLA/automation/reporting behavior keys on.
             */
            category: components["schemas"]["TicketStatusCategory"];
            color?: string | null;
            /** Format: uuid */
            id: string;
            /** @description Tickets may enter this status directly at creation. */
            is_entry: boolean;
            name: string;
            /** Format: int32 */
            sort_order: number;
        };
        /**
         * @description One custom field definition. `config` is the kind-specific document
         *     (select options, validation bounds, …) exactly as stored.
         */
        ManifestCustomField: {
            /**
             * Format: date-time
             * @description Present when the definition was archived (still a customization the
             *     preflight must see; the values it shaped may still exist).
             */
            archived_at?: string | null;
            config: Record<string, never>;
            /** @description Entity the field attaches to: ticket | client | contact | asset. */
            entity: string;
            /** Format: uuid */
            id: string;
            /** @description Field kind: text | number | date | select | multiselect | bool | url | user. */
            kind: string;
            name: string;
            required: boolean;
            /** Format: int32 */
            sort_order: number;
        };
        /** @description One escalation ladder step. */
        ManifestEscalationStep: {
            /**
             * Format: int32
             * @description Percent of the target's business-time budget consumed at firing.
             */
            at_pct: number;
            enabled: boolean;
            /** @description JSON array of user ids to notify. */
            notify_user_ids: Record<string, never>;
            /** Format: int32 */
            step_index: number;
            /** @description risk | breach | escalate. */
            trigger_kind: string;
        };
        /**
         * @description One stored report definition (TASKLIST R3). The spec body rides as its
         *     stored jsonb document (`{filters, group_by, measures}`) exactly like
         *     custom-field `config` — the registry-validated view/viz slugs travel as
         *     their own fields so a preflight can flag deprecated constructs without
         *     re-deriving validation. Nothing here is secret-bearing: definitions
         *     reference the read-model catalog only, never credentials or data.
         */
        ManifestReportDefinition: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by: string;
            description?: string | null;
            /** Format: uuid */
            id: string;
            /** @description Registry-backed view slug (`tickets`, `time_entries`, …). */
            kind: string;
            name: string;
            /**
             * @description Visibility scope: `org` (published org-wide) or `self` (private to
             *     the creator; admins see all). Preflight-relevant because org-scope
             *     definitions are shared surface, self-scope ones are personal.
             */
            scope: string;
            /** @description Spec body document: `{filters, group_by, measures}` as stored. */
            spec: Record<string, never>;
            /** Format: date-time */
            updated_at: string;
            /** @description Visualization hint slug (`bar` | `line` | `pie` | `table`). */
            viz: string;
        };
        /**
         * @description Section container. All fields are always present; empty org state is an
         *     empty array, never an absent key. New sections append here.
         */
        ManifestSections: {
            /** @description API-key metadata minus secret material (no hashes, no raw keys). */
            api_keys: components["schemas"]["ManifestApiKey"][];
            /** @description Event-triggered automation rules. */
            automation_rules: components["schemas"]["ManifestAutomationRule"][];
            /**
             * @description Per-board workflow: the board identity plus its statuses (each bound
             *     to a semantic category — the defaults-first guarantee) and its
             *     explicit transition adjacency. An EMPTY transitions array means the
             *     semantic-category floor applies, not "no transitions".
             */
            boards: components["schemas"]["ManifestBoard"][];
            /**
             * @description Custom field definitions across all entities (tickets, clients,
             *     contacts, assets), including archived ones.
             */
            custom_fields: components["schemas"]["ManifestCustomField"][];
            /**
             * @description Stored report definitions (TASKLIST R3): registry-validated specs
             *     over the versioned read-model views. Covered so the upgrade
             *     preflight (U6) can flag any definition whose `kind`/spec columns
             *     reference deprecated view or column constructs (policy §5).
             */
            report_definitions: components["schemas"]["ManifestReportDefinition"][];
            /**
             * @description Org-scope settings-registry overrides (user-scope rows are per-user
             *     preferences, not org customization).
             */
            settings: components["schemas"]["ManifestSetting"][];
            /** @description SLA configuration: calendars and policies with escalation ladders. */
            sla: components["schemas"]["ManifestSla"];
            /** @description Webhook subscriptions with event-type filters, never secrets. */
            webhooks: components["schemas"]["ManifestWebhook"][];
        };
        /** @description One org-scope settings-registry override. */
        ManifestSetting: {
            key: string;
            value: Record<string, never>;
        };
        /** @description SLA configuration sections. */
        ManifestSla: {
            calendars: components["schemas"]["ManifestSlaCalendar"][];
            policies: components["schemas"]["ManifestSlaPolicy"][];
        };
        /** @description One named SLA calendar row with its flattened business calendar body. */
        ManifestSlaCalendar: {
            /** @description Timezone / weekday windows / holidays / maintenance document. */
            calendar: Record<string, never>;
            /** Format: uuid */
            id: string;
            name: string;
        };
        /**
         * @description One SLA policy with its targets document, embedded fallback calendar,
         *     calendar attachment, and escalation ladder.
         */
        ManifestSlaPolicy: {
            /** @description Embedded fallback calendar; governs while `calendar_id` is null. */
            calendar: Record<string, never>;
            /**
             * Format: uuid
             * @description Attached named calendar; null keeps the embedded one authoritative.
             */
            calendar_id: string | null;
            /** @description Escalation ladder steps, ordered by `step_index`. */
            escalation_steps: components["schemas"]["ManifestEscalationStep"][];
            /** Format: uuid */
            id: string;
            is_default: boolean;
            name: string;
            /** @description Targets document keyed by priority string. */
            targets: Record<string, never>;
        };
        /** @description One allowed status change, `from` → `to`, both ids from the SAME board. */
        ManifestTransition: {
            /** Format: uuid */
            from_status_id: string;
            /** Format: uuid */
            to_status_id: string;
        };
        /**
         * @description One webhook subscription. The HMAC signing secret is structurally
         *     absent: it exists only at creation time and in the sealed column.
         */
        ManifestWebhook: {
            /** Format: uuid */
            created_by: string | null;
            enabled: boolean;
            /** @description Event-type filter; empty array = wildcard (all event types). */
            event_types: string[];
            /** Format: uuid */
            id: string;
            url: string;
        };
        MarkReadBulkRequest: {
            ids: string[];
        };
        MarkReadResult: {
            /**
             * Format: int64
             * @description Rows flipped from unread to read (idempotent under retry).
             */
            marked: number;
        };
        MeResponse: {
            display_name: string;
            email: string;
            /** Format: uuid */
            org_id: string;
            permissions: string[];
            timezone: string;
            /** Format: uuid */
            user_id: string;
        };
        /** @description One aggregate measure. */
        MeasureSpec: {
            agg: components["schemas"]["Aggregate"];
            /** @description `None` only with [`Aggregate::Count`] (`COUNT(*)`). */
            column?: string | null;
        };
        MfaTokenRequest: {
            mfa_token: string;
        };
        MfaVerifyRequest: {
            /** @description TOTP code or recovery code. */
            code: string;
            /** @description The `mfa_token` from POST /api/auth/login when mfa_required was set. */
            mfa_token: string;
        };
        MoveAppointmentRequest: {
            /** Format: date-time */
            ends_at: string;
            /** Format: date-time */
            starts_at: string;
        };
        /** @description One move in a batch: the booking's new user and slot. */
        MoveSpec: {
            /** Format: uuid */
            appointment_id: string;
            /** Format: date-time */
            ends_at: string;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            user_id: string;
        };
        /**
         * @description Read-your-write metadata every mutation response carries (IF2), merged
         *     as the additive `mutation` object alongside the entity fields.
         */
        MutationMeta: {
            /** @description Applied entity snapshot. */
            after: Record<string, never>;
            /**
             * @description Prior entity snapshot (the undo substrate). `null` on creates. The
             *     OpenAPI schema stays a plain object; generated clients get the
             *     runtime-typed view through the SDK's `snapshotOf` accessor.
             */
            before: Record<string, never> | null;
            /**
             * @description The idempotency key that claimed this result, when the caller
             *     supplied one.
             */
            idempotency_key?: string | null;
            /**
             * Format: int64
             * @description Monotonic per-entity sequence: strictly increases by one per
             *     applied write, so clients order reconciliations against the event
             *     stream.
             */
            seq: number;
            /** @description The applied entity's `updated_at`. */
            updated_at: string;
            /**
             * Format: int64
             * @description Version token for If-Match-style writes; mirrors `seq`. Echo it in
             *     the next write's `version` field.
             */
            version: number;
        };
        /**
         * @description The autosave mutation parameters every gated create/update body carries
         *     (flattened into the request struct — purely additive fields).
         */
        MutationParams: {
            /**
             * @description Idempotency key: the same key with the same payload replays the
             *     SAME stored result (never double-applies); the same key with a
             *     different payload is a typed 409. Generate one per user edit
             *     debounce/retry episode (UUID works).
             */
            idempotency_key?: string | null;
            /**
             * Format: int64
             * @description If-Match-style version: the entity `version` the caller last saw
             *     (the `mutation.version` of the previous response). A mismatch
             *     rejects the write with a typed 409 `version_conflict` carrying
             *     `expected`/`provided`; omit for creates and for legacy callers.
             */
            version?: number | null;
        };
        /**
         * @description One cross-board my-work queue row (TASKLIST 2.11): the ticket plus the
         *     annotations the Today screen renders per the mockup.
         */
        MyWorkItem: {
            /**
             * Format: uuid
             * @description The source board of this cross-board row.
             */
            board_id: string;
            board_name: string;
            category: components["schemas"]["TicketStatusCategory"];
            /** @description Client short label for the squared tag (`None` = internal ticket). */
            client_name?: string | null;
            /**
             * Format: date-time
             * @description Manual deadline set on the ticket itself.
             */
            due_at?: string | null;
            /** Format: uuid */
            id: string;
            number: string;
            priority: components["schemas"]["Priority"];
            /**
             * Format: date-time
             * @description SLA resolution deadline when state exists; the UI prefers this over
             *     `due_at` for the right-aligned countdown column.
             */
            sla_resolution_due?: string | null;
            status_name: string;
            title: string;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description Taxonomy of structured ticket notes (§1).
         * @enum {string}
         */
        NoteKind: "internal" | "client_visible" | "in_house_update";
        Notification: {
            body?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            kind: components["schemas"]["NotificationKind"];
            /** Format: uuid */
            org_id: string;
            /** Format: date-time */
            read_at?: string | null;
            /** Format: uuid */
            ticket_id?: string | null;
            title: string;
            /** Format: uuid */
            user_id: string;
        };
        /** @enum {string} */
        NotificationKind: "mention" | "assignment" | "sla_risk" | "sla_breach" | "escalation" | "missing_time" | "ticket_update" | "report_delivery";
        /** @description One page of the inbox plus the cursor to continue with. */
        NotificationPage: {
            items: components["schemas"]["Notification"][];
            /** @description Pass as `cursor` on the next request; `None` means exhausted. */
            next_cursor?: string | null;
        };
        OidcProviderInfo: {
            client_id: string;
            discovery_url: string;
            display_name: string;
            enabled: boolean;
            /** Format: uuid */
            id: string;
            issuer: string;
            jit_provisioning: boolean;
            /** @description URL path segment used by login/link routes. */
            name: string;
        };
        /** @description An absolute UTC on-call rotation segment for one user. */
        OnCallBlock: {
            /** @description Free-form rotation label ('primary', 'secondary', 'escalation', …). */
            category: string;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            ends_at: string;
            /** Format: uuid */
            id: string;
            notes?: string | null;
            /** Format: uuid */
            org_id: string;
            /** Format: date-time */
            starts_at: string;
            /** Format: uuid */
            user_id: string;
        };
        /**
         * @description Org-wide preview: every billable contract's evaluation for the window
         *     plus the grand total — the shape a Wave-9 invoice run will consume.
         */
        OrgBillingPreview: {
            evaluations: components["schemas"]["BillingEvaluation"][];
            /** Format: date */
            from: string;
            /** Format: date */
            to: string;
            /**
             * Format: int64
             * @description Sum over evaluations, minor units (mixed currencies reject).
             */
            total_minor: number;
        };
        OrgProfileResponse: {
            /** Format: uuid */
            id: string;
            name: string;
            slug: string;
            ticket_prefix: string;
        };
        OwnedDeleteQuery: {
            /**
             * Format: uuid
             * @description Owner of the row being deleted (must match self-or-admin rule).
             */
            user_id: string;
        };
        PatchAlertSourceRequest: components["schemas"]["MutationParams"] & {
            /** Format: uuid */
            board_id?: string | null;
            /**
             * Format: uuid
             * @description `Some(None)` clears; `Some(Some(v))` sets; `None` = no change.
             */
            client_id?: string | null;
            /** Format: int32 */
            correlation_window_secs?: number | null;
            enabled?: boolean | null;
            name?: string | null;
        };
        PatchAppointmentRequest: {
            kind?: null | components["schemas"]["AppointmentKind"];
            notes?: string | null;
            status?: null | components["schemas"]["AppointmentStatus"];
            /** Format: uuid */
            ticket_id?: string | null;
            title?: string | null;
            /** Format: int32 */
            travel_minutes?: number | null;
        };
        PatchContractRequest: components["schemas"]["MutationParams"] & {
            /** Format: int64 */
            amount_minor?: number | null;
            /** Format: date */
            billing_anchor?: string | null;
            billing_frequency?: null | components["schemas"]["BillingFrequency"];
            /** Format: uuid */
            client_id?: string | null;
            contract_type?: null | components["schemas"]["ContractType"];
            currency?: string | null;
            /** Format: date */
            effective_from?: string | null;
            /** Format: date */
            effective_until?: string | null;
            /** @description When present, replaces the whole fee schedule. */
            fees?: components["schemas"]["FeeSpec"][] | null;
            /** Format: int64 */
            hourly_rate_minor?: number | null;
            /** Format: int32 */
            included_minutes?: number | null;
            name?: string | null;
            notes?: string | null;
            number?: string | null;
            /** Format: int64 */
            overage_rate_minor?: number | null;
            /** @description When present, replaces the whole rate card. */
            rates?: components["schemas"]["RateSpec"][] | null;
            status?: null | components["schemas"]["ContractStatus"];
        };
        PatchOrgRequest: {
            name?: string | null;
            ticket_prefix?: string | null;
        };
        PatchPhaseRequest: components["schemas"]["MutationParams"] & {
            description?: string | null;
            is_milestone?: boolean | null;
            /** Format: int64 */
            milestone_amount_minor?: number | null;
            milestone_currency?: string | null;
            name?: string | null;
            /** Format: int32 */
            sort_order?: number | null;
            status?: null | components["schemas"]["PhaseStatus"];
        };
        PatchPortalUserRequest: {
            display_name?: string | null;
            /** @description Optional credential reset (hashed before storage). */
            password?: string | null;
            role?: null | components["schemas"]["PortalRole"];
            status?: null | components["schemas"]["PortalUserStatus"];
        };
        PatchProjectRequest: components["schemas"]["MutationParams"] & {
            billing_method?: null | components["schemas"]["BillingMethod"];
            /** Format: int64 */
            budget_minor?: number | null;
            /** Format: uuid */
            client_id?: string | null;
            currency?: string | null;
            /** @description `Some(Some(v))` sets; `Some(None)` clears; `None` = no change. */
            description?: string | null;
            /** Format: date */
            due_date?: string | null;
            name?: string | null;
            /** Format: date */
            start_date?: string | null;
            status?: null | components["schemas"]["ProjectStatus"];
        };
        PatchTaskRequest: components["schemas"]["MutationParams"] & {
            /** Format: uuid */
            assignee_id?: string | null;
            description?: string | null;
            /** Format: date-time */
            due_at?: string | null;
            is_milestone?: boolean | null;
            /** Format: int64 */
            milestone_amount_minor?: number | null;
            milestone_currency?: string | null;
            /** Format: uuid */
            phase_id?: string | null;
            /** Format: int32 */
            sort_order?: number | null;
            status?: null | components["schemas"]["TaskStatus"];
            title?: string | null;
        };
        PatchTemplateRequest: {
            description?: string | null;
            name?: string | null;
            spec?: unknown;
        };
        PatchTicketRequest: components["schemas"]["MutationParams"] & {
            /**
             * Format: uuid
             * @description `Some(Some(id))` assigns; `Some(None)` unassigns; `None` = no change.
             */
            assignee_id?: string | null;
            body?: string | null;
            /** Format: uuid */
            client_id?: string | null;
            /** Format: uuid */
            contact_id?: string | null;
            /**
             * @description Custom-field values to upsert. Partial semantics: only listed fields
             *     change, each validated against its definition.
             */
            custom_fields?: components["schemas"]["TicketFieldInput"][] | null;
            /** Format: date-time */
            due_at?: string | null;
            priority?: null | components["schemas"]["Priority"];
            /** Format: uuid */
            site_id?: string | null;
            title?: string | null;
        };
        PatchViewRequest: {
            is_default?: boolean | null;
            name?: string | null;
            spec?: unknown;
        };
        PauseTimerRequest: {
            /** @description Optional reason, preserved on the segment forever. */
            reason?: string | null;
            /**
             * Format: uuid
             * @description Omit to act on the caller's active timer.
             */
            session_id?: string | null;
        };
        /**
         * @description Payment receipt response. The invoice is nested (not flattened) so
         *     the stored replay payload — `{payment, invoice}` plus the merged
         *     mutation metadata — deserializes into this shape byte-identically.
         */
        PaymentMutationResponse: {
            invoice: components["schemas"]["Invoice"];
            mutation: components["schemas"]["MutationMeta"];
            payment: components["schemas"]["InvoicePayment"];
        };
        PhaseMutationResponse: components["schemas"]["ProjectPhase"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description Lifecycle of a phase.
         * @enum {string}
         */
        PhaseStatus: "pending" | "active" | "completed" | "cancelled";
        PolicyView: {
            /** Format: uuid */
            calendar_id?: string | null;
            /** Format: uuid */
            id: string;
            is_default: boolean;
            name: string;
            /** @description Targets keyed by priority ("p1".."p4"), business minutes. */
            targets: unknown;
            /** @description True while the policy governs itself via its embedded calendar. */
            uses_embedded_calendar: boolean;
        };
        /** @description Portal attachment view: metadata only (bytes stream on download). */
        PortalAttachmentView: {
            content_type: string;
            /** Format: date-time */
            created_at: string;
            filename: string;
            /** Format: uuid */
            id: string;
            sha256: string;
            /** Format: int64 */
            size_bytes: number;
        };
        PortalLoginRequest: {
            email: string;
            password: string;
        };
        PortalLoginResponse: {
            access_token: string;
            /** Format: int64 */
            expires_in: number;
            portal_user: components["schemas"]["PortalProfile"];
            refresh_token: string;
            /** Format: uuid */
            session_id: string;
        };
        /** @description The signed-in portal identity (also served by GET /api/portal/me). */
        PortalProfile: {
            /** Format: uuid */
            client_id: string;
            display_name: string;
            email: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            role: components["schemas"]["PortalRole"];
        };
        PortalRefreshRequest: {
            refresh_token: string;
            /** Format: uuid */
            session_id: string;
        };
        PortalReplyRequest: {
            body: string;
        };
        /**
         * @description Portal role tiers. `contact` is the default read/reply tier;
         *     `approver` is behaviorally identical in v1 but exists so approval
         *     grants (Stage 3 quote approval / Stage 4 optional client approvals)
         *     can be layered on without re-provisioning identities; `it_manager`
         *     ADDITIONALLY reads `in_house_update` notes. NO tier ever reads
         *     `internal` notes — enforced by `notes::visible_kinds`, tested in
         *     `policy` and `notes`.
         * @enum {string}
         */
        PortalRole: "contact" | "approver" | "it_manager";
        PortalTicketDetail: {
            can_view_in_house_updates: boolean;
            /** @description Only the kinds this tier may read, filtered by the domain policy. */
            comments: components["schemas"]["TicketComment"][];
            ticket: components["schemas"]["Ticket"];
        };
        PortalTicketListQuery: {
            /** Format: uuid */
            board_id?: string | null;
            category?: null | components["schemas"]["TicketStatusCategory"];
            /** Format: int64 */
            limit?: number;
            /** Format: int64 */
            offset?: number;
        };
        PortalUser: {
            /** Format: uuid */
            client_id: string;
            /** Format: date-time */
            created_at: string;
            display_name: string;
            email: string;
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            role: components["schemas"]["PortalRole"];
            status: components["schemas"]["PortalUserStatus"];
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description Lifecycle of a portal identity. `invited` = admin-provisioned, has not
         *     logged in yet; first successful login flips to `active`. `disabled`
         *     blocks login AND every authenticated portal request (checked per
         *     request, never cached in the token).
         * @enum {string}
         */
        PortalUserStatus: "invited" | "active" | "disabled";
        /**
         * @description One resolved preference row: which chain link supplied it (`user`,
         *     `org_default`, or `builtin`), the channel toggles, and the quiet-hours
         *     window with its timezone.
         */
        PreferenceView: {
            email: boolean;
            in_app: boolean;
            kind: components["schemas"]["NotificationKind"];
            quiet_from?: string | null;
            quiet_until?: string | null;
            source: string;
            timezone: string;
        };
        PreviewQuery: {
            /**
             * Format: date
             * @description Window start (inclusive), ISO date.
             */
            from?: string | null;
            /**
             * Format: date
             * @description Window end (inclusive), ISO date.
             */
            to?: string | null;
        };
        /**
         * @description Industry-standard four-tier priority.
         * @enum {string}
         */
        Priority: "p1" | "p2" | "p3" | "p4";
        /**
         * @description A project engagement under a client (or internal when `client_id` is
         *     `None`).
         */
        Project: {
            billing_method: components["schemas"]["BillingMethod"];
            /**
             * Format: int64
             * @description Project budget in minor units; `None` = unbudgeted.
             */
            budget_minor?: number | null;
            /** Format: uuid */
            client_id?: string | null;
            /** Format: date-time */
            created_at: string;
            /** @description ISO 4217 currency for `budget_minor` (required when a budget is set). */
            currency?: string | null;
            description?: string | null;
            /** Format: date */
            due_date?: string | null;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: date */
            start_date?: string | null;
            status: components["schemas"]["ProjectStatus"];
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description The instantiation result: the applied project plus the expansion
         *     counts (clients render "created N phases, M tasks" without refetching).
         */
        ProjectInstantiateResponse: components["schemas"]["Project"] & {
            mutation: components["schemas"]["MutationMeta"];
            /** Format: int64 */
            phase_count: number;
            /** Format: int64 */
            task_count: number;
        };
        ProjectListQuery: {
            /** Format: uuid */
            client_id?: string | null;
            /** Format: int64 */
            limit?: number;
            /** Format: int64 */
            offset?: number;
            status?: null | components["schemas"]["ProjectStatus"];
        };
        /**
         * @description Instant-first mutation response for the project entity (IF2): applied
         *     snapshot at the top level plus the additive `mutation` metadata.
         */
        ProjectMutationResponse: components["schemas"]["Project"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description A phase groups tasks into a delivery stage. A phase flagged
         *     `is_milestone` is itself a billing milestone.
         */
        ProjectPhase: {
            /** Format: date-time */
            completed_at?: string | null;
            /** Format: date-time */
            created_at: string;
            description?: string | null;
            /** Format: uuid */
            id: string;
            is_milestone: boolean;
            /**
             * Format: int64
             * @description Billing amount released when this milestone phase completes
             *     (minor units; `None` = informational milestone).
             */
            milestone_amount_minor?: number | null;
            /** @description ISO 4217 code for `milestone_amount_minor`. */
            milestone_currency?: string | null;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            project_id: string;
            /** Format: int32 */
            sort_order: number;
            status: components["schemas"]["PhaseStatus"];
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description Lifecycle of a project.
         * @enum {string}
         */
        ProjectStatus: "planning" | "active" | "on_hold" | "completed" | "cancelled";
        /**
         * @description A unit of project work. A task flagged `is_milestone` is a billing
         *     milestone: completing it emits `project.milestone_completed`.
         */
        ProjectTask: {
            /** Format: uuid */
            assignee_id?: string | null;
            /** Format: date-time */
            completed_at?: string | null;
            /** Format: date-time */
            created_at: string;
            description?: string | null;
            /** Format: date-time */
            due_at?: string | null;
            /** Format: uuid */
            id: string;
            is_milestone: boolean;
            /** Format: int64 */
            milestone_amount_minor?: number | null;
            milestone_currency?: string | null;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            phase_id?: string | null;
            /** Format: uuid */
            project_id: string;
            /** Format: int32 */
            sort_order: number;
            status: components["schemas"]["TaskStatus"];
            title: string;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description A reusable project blueprint. `spec` follows `core-projects::template`
         *     (`TemplateSpec`, additive-only evolution); instantiation is a plain
         *     transactional insert plan — templates are data, not code (policy §5).
         */
        ProjectTemplate: {
            /** Format: date-time */
            created_at: string;
            description?: string | null;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
            spec: Record<string, never>;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description A saved task-list view: filter/group/sort spec over one project's tasks
         *     (`project_id: None` = org-wide). The `spec` is a versioned, additive-only
         *     JSON document validated at the route layer; unknown fields are tolerated
         *     (policy §2 posture for declared customizations-as-data).
         */
        ProjectView: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            is_default: boolean;
            name: string;
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            project_id?: string | null;
            spec: Record<string, never>;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description Proposal classification: who will see the change if it lands.
         * @enum {string}
         */
        ProposalClassification: "internal" | "client_visible" | "external_commit";
        /** @description GET /api/agent-proposals query. */
        ProposalListQuery: {
            /** @description Opaque keyset cursor from a previous page's `next_cursor`. */
            cursor?: string | null;
            /**
             * Format: int64
             * @description Page size, clamped to [1, 200].
             */
            limit?: number | null;
            /** @description Filter by lifecycle state. */
            status?: string | null;
        };
        /**
         * @description Proposal lifecycle state.
         * @enum {string}
         */
        ProposalStatus: "proposed" | "approved" | "declined" | "withdrawn" | "applied";
        PublicProvider: {
            display_name: string;
            /** @description URL path segment for /api/auth/oidc/{name}/login. */
            name: string;
        };
        PublishSkillRequest: {
            /** @description `{capabilities: [...], prompts: [...], rules: [...]}`. */
            manifest: Record<string, never>;
            /**
             * @description Slug; version is derived server-side (max + 1) — append-only
             *     history, never an in-place edit.
             */
            slug: string;
        };
        /** @description Request to store or update a sealed BYOK API key. */
        PutByokSecretRequest: components["schemas"]["MutationParams"] & {
            /** @description Plaintext API key / token to be sealed at rest. */
            api_key: string;
            /**
             * @description Provider name: an AI provider ("openai", "anthropic", "ollama",
             *     "custom") or an export-sync connector ("qbo", "xero").
             */
            provider: string;
        };
        PutCapabilityRequest: {
            /** @description `<namespace>:read` | `<namespace>:write`. */
            capability: string;
            mode: components["schemas"]["CapabilityMode"];
        };
        PutChecklistTemplateRequest: {
            /** @description The complete required-step list; replaces any previous items. */
            items: string[];
            name: string;
            /** @description Enforcement toggle (defaults-first: OFF until explicitly enabled). */
            require_checklist_close?: boolean;
        };
        PutExposureRequest: {
            /** Format: uuid */
            board_id: string;
            exposed: boolean;
            /**
             * @description Empty = all statuses visible (v1 default). Non-empty narrows to
             *     statuses OF THAT BOARD — unknown ids are rejected, not silently kept.
             */
            visible_status_ids: string[];
        };
        RangeParams: {
            /** Format: date-time */
            from?: string | null;
            /** Format: date-time */
            to?: string | null;
            /** Format: uuid */
            user_id?: string | null;
        };
        /** @description T&M rate-card row spec (hourly minor units per `work_type`). */
        RateSpec: {
            /** Format: int64 */
            rate_minor: number;
            work_type: string;
        };
        RecordPaymentRequest: components["schemas"]["MutationParams"] & {
            /**
             * Format: int64
             * @description Amount to collect, minor units; must not exceed the outstanding
             *     balance. Currency is always the invoice's own.
             */
            amount_minor: number;
            notes?: string | null;
        };
        RecoveryCodesResponse: {
            /** @description Plaintext codes — displayed once, stored only as argon2id hashes. */
            recovery_codes: string[];
        };
        RefreshRequest: {
            refresh_token: string;
            /**
             * Format: uuid
             * @description Session id returned alongside the refresh token.
             */
            session_id: string;
        };
        RegenerateRecoveryRequest: {
            code: string;
            password: string;
        };
        RegisterRequest: {
            display_name: string;
            email: string;
            /** @description Organization name — becomes the tenant for this deployment. */
            org_name: string;
            /** @description URL-safe org slug (unique). */
            org_slug: string;
            password: string;
            /** @description Ticket number prefix, e.g. "MER" → MER-2841. Defaults to "MER". */
            ticket_prefix?: string | null;
            /** @default UTC */
            timezone: string | null;
        };
        RegistryResponse: {
            settings: components["schemas"]["DeclarationView"][];
        };
        ReplaceLadderRequest: {
            steps: components["schemas"]["LadderStepSpec"][];
        };
        /** @description GET /api/report-definitions envelope. */
        ReportDefinitionList: {
            definitions: components["schemas"]["ReportDefinitionView"][];
        };
        /**
         * @description Mutation envelope: applied definition (flattened) + read-your-write
         *     metadata (IF2).
         */
        ReportDefinitionMutationResponse: components["schemas"]["ReportDefinitionView"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description A stored report definition with its registry-validated spec
         *     reassembled (`spec.view` == the stored `kind` column).
         */
        ReportDefinitionView: {
            created_at: string;
            /** Format: uuid */
            created_by: string;
            description?: string | null;
            /** Format: uuid */
            id: string;
            name: string;
            scope: components["schemas"]["DefinitionScope"];
            spec: components["schemas"]["ReportSpec"];
            updated_at: string;
        };
        /** @description JSON envelope for a report page. */
        ReportPage: {
            /**
             * @description Stable column order (JSON objects here preserve insertion order via
             *     serde_json preserve_order feature transparently at runtime).
             */
            columns: string[];
            kind: string;
            /** @description Pass back as `cursor`; absent when exhausted or unpaged. */
            next_cursor?: string | null;
            rows: unknown[];
            /** @description Whole-window aggregates (never paginated). */
            summary: unknown;
            /** @description Versioned read-model view backing this kind (documentation parity). */
            view: string;
        };
        /** @description Mutation envelope: applied schedule + read-your-write metadata (IF2). */
        ReportScheduleMutationResponse: components["schemas"]["ReportScheduleView"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /** @description GET /api/report-schedules envelope. */
        ReportSchedulePage: {
            /** @description Pass back as `cursor`; absent when exhausted. */
            next_cursor?: string | null;
            /** @description Deterministically ordered `(name, id)`. */
            schedules: components["schemas"]["ReportScheduleView"][];
        };
        /**
         * @description One delivery schedule with its next-run preview (the materialized
         *     `next_run_at` the worker claims against) and definition coordinates.
         */
        ReportScheduleView: {
            cadence: components["schemas"]["ScheduleCadence"];
            created_at: string;
            /** Format: uuid */
            created_by: string;
            /**
             * Format: int32
             * @description 1..=28 (monthly only).
             */
            day_of_month?: number | null;
            /**
             * Format: int32
             * @description 0 = Monday .. 6 = Sunday (weekly only).
             */
            day_of_week?: number | null;
            definition_kind: string;
            /** @description Denormalized for the listing UI (`definition_name`, `definition_kind`). */
            definition_name: string;
            enabled: boolean;
            /** Format: uuid */
            id: string;
            last_error?: string | null;
            last_run_at?: string | null;
            name: string;
            /** @description Next-run preview — the instant the worker will next claim. */
            next_run_at: string;
            /** @description Recipient staff user ids. */
            recipients: string[];
            /** Format: uuid */
            report_definition_id: string;
            /** @description Local wall-clock fire time, `HH:MM`. */
            time_of_day: string;
            timezone: string;
            updated_at: string;
            /**
             * Format: int32
             * @description Trailing-day window each run covers.
             */
            window_days: number;
        };
        /**
         * @description A stored report spec (TASKLIST R3). Every member is registry-validated;
         *     see [`ReportSpec::validate`].
         */
        ReportSpec: {
            filters?: components["schemas"]["SpecFilter"][];
            /**
             * @description Group-by dimensions (any projected column except `org_id`). Empty
             *     means a single grand-total row.
             */
            group_by?: string[];
            /** @description Aggregate measures, one or more. */
            measures: components["schemas"]["MeasureSpec"][];
            /**
             * @description The versioned read-model view this report reads (the stored `kind`
             *     column). NEVER a physical table — the enum has no other variants.
             */
            view: components["schemas"]["ViewId"];
            /** @description Presentation hint (never affects the compiled SQL). */
            viz: components["schemas"]["VizHint"];
        };
        RestoreDocumentRequest: components["schemas"]["MutationParams"] & {
            change_summary?: string | null;
            /**
             * Format: int32
             * @description The historical version whose content the NEW version copies.
             */
            version: number;
        };
        /** @description What the revert restored. */
        RevertDetail: {
            /** @description Record fields whose values were restored. */
            fields: string[];
            /**
             * Format: int64
             * @description The write-trail seq the revert stepped back over.
             */
            reverted_seq: number;
            /** @description Whether this was a whole-record step (no explicit field list). */
            whole_record: boolean;
        };
        /** @description POST /api/tickets/{id}/revert body. */
        RevertTicketRequest: components["schemas"]["MutationParams"] & {
            /**
             * @description Explicit per-field revert: restore ONLY these fields to their values
             *     at the target write. Omit for a whole-record step (rejected with
             *     `undo_reverts_only` while the record is closed).
             */
            fields?: string[] | null;
            /**
             * Format: int64
             * @description The write-trail `seq` to step back over (from the undo stack).
             */
            seq: number;
        };
        /**
         * @description Wire format selector for an import payload.
         * @enum {string}
         */
        RowFormat: "json" | "csv";
        RowOutcome: {
            /** @enum {string} */
            status: "ok";
        } | {
            /** @enum {string} */
            status: "would_update";
        } | {
            reason: string;
            /** @enum {string} */
            status: "would_skip";
        } | {
            errors: string[];
            /** @enum {string} */
            status: "failed";
        };
        /**
         * @description Per-row dry-run/commit report entry. The wire shape contract from D2:
         *     every row maps to exactly one of `errors[]`, `ok`, `would_skip`,
         *     `would_update`.
         */
        RowReport: {
            external_key?: string | null;
            outcome: components["schemas"]["RowOutcome"];
            /** Format: int64 */
            row_index: number;
        };
        /** @description The encoded row source shared by dry-run and chunk posts. */
        RowsPayload: {
            /**
             * @description Full CSV text including header row (`format=csv`). UTF-8, BOM
             *     tolerated; RFC 4180 quoting supported.
             */
            csv?: string | null;
            format: components["schemas"]["RowFormat"];
            /**
             * @description Row objects as a JSON array of flat objects (`format=json`). Nulls
             *     behave exactly like absent keys.
             */
            json_rows: Record<string, never>;
        };
        /**
         * @description POST /imports/batches/{id}/run response: the batch is queued for the
         *     worker exactly once; poll the batch view for counters.
         */
        RunBatchResponse: {
            /** Format: uuid */
            batch_id: string;
            /** Format: uuid */
            job_id: string;
            /** Format: int32 */
            staged_chunks: number;
        };
        /**
         * @description Per-row fate report — the D2 contract shape
         *     (`row_index → errors[] | ok | would_skip | would_update`).
         */
        RunReportResponse: {
            /** Format: uuid */
            batch_id: string;
            conflict_policy: components["schemas"]["ConflictPolicy"];
            dry_run: boolean;
            entity_kind: components["schemas"]["EntityKind"];
            rows: components["schemas"]["RowReport"][];
            summary: components["schemas"]["ChunkSummaryWire"];
        };
        /**
         * @description Wire cadence of a schedule (snake_case slugs matching the stored
         *     column and the worker's parser).
         * @enum {string}
         */
        ScheduleCadence: "daily" | "weekly" | "monthly";
        /** @description GET /api/report-schedules query. */
        ScheduleListQuery: {
            /** @description Opaque keyset cursor from a previous page's `next_cursor`. */
            cursor?: string | null;
            /**
             * Format: int64
             * @description Page size, clamped to [1, 200].
             */
            limit?: number | null;
        };
        SearchQuery: {
            /** Format: int64 */
            limit?: number;
            /** Format: int64 */
            offset?: number;
            /**
             * @description Free-text query using websearch syntax: `"exact phrases"`,
             *     `-excluded`, plain terms AND-ed. Tokens ≥3 chars also match as
             *     substrings via the trigram fallback.
             */
            q: string;
        };
        SearchResponse: {
            /** @description Kinds the principal may see, mirrored into the query filter. */
            kinds: string[];
            /** Format: int64 */
            limit: number;
            /** Format: int64 */
            offset: number;
            /** @description The trimmed query actually executed. */
            query: string;
            results: components["schemas"]["SearchResult"][];
            /**
             * Format: int64
             * @description Total matches across all permitted kinds.
             */
            total: number;
        };
        SearchResult: {
            /** Format: uuid */
            entity_id: string;
            /** @description `ticket` | `comment` | `client` | `site` | `contact`. */
            entity_kind: string;
            /**
             * Format: uuid
             * @description comment → its ticket id; NULL otherwise.
             */
            parent_id?: string | null;
            /**
             * Format: double
             * @description Blended FTS + trigram relevance score (higher is better).
             */
            score: number;
            /** @description Raw body excerpt for client-side snippet rendering. */
            snippet: string;
            title: string;
        };
        /**
         * @description Why a timer segment ended. Preserved forever — the pause reason is
         *     billable-work metadata, not a transient UI state.
         * @enum {string}
         */
        SegmentEnd: "stopped" | "paused" | "switched";
        SessionInfo: {
            chain_root: boolean;
            /** Format: date-time */
            created_at: string;
            /** @description True for the session making the request. */
            current: boolean;
            /** Format: date-time */
            expires_at: string;
            /** Format: uuid */
            id: string;
            user_agent?: string | null;
        };
        SetChecklistItemRequest: components["schemas"]["MutationParams"] & {
            checked: boolean;
        };
        SetDependenciesRequest: {
            /**
             * @description The FULL finish-to-start dependency set for the task (PUT
             *     semantics): every id must be a task of the same project; the set
             *     must not create a cycle (typed 422 names the offending edge).
             */
            depends_on: string[];
        };
        SetTicketFieldValueRequest: {
            value: unknown;
        };
        SetTransitionsRequest: {
            /**
             * @description The complete allowed matrix. Empty list returns the board to
             *     floor-mode governance.
             */
            edges: components["schemas"]["TransitionEdge"][];
        };
        /**
         * @description The value shapes the registry understands. A stored value must satisfy
         *     its declaration's kind or the write is rejected (validation is applied
         *     before persistence, never after).
         */
        SettingKind: {
            /** @enum {string} */
            type: "boolean";
        } | {
            /** @enum {string} */
            type: "integer";
            /** @description Inclusive integer range. */
            value: {
                /** Format: int64 */
                max: number;
                /** Format: int64 */
                min: number;
            };
        } | {
            /** @enum {string} */
            type: "text";
            /** @description Free text with a hard length cap. */
            value: {
                max_len: number;
            };
        } | {
            /** @enum {string} */
            type: "enumeration";
        };
        /**
         * @description A concrete setting value. Only three payload types exist by design —
         *     richer structures are entities, not settings.
         */
        SettingValue: boolean | number | string;
        Site: {
            address?: string | null;
            city?: string | null;
            /** Format: uuid */
            client_id: string;
            country?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            /**
             * Format: double
             * @description lat/lon in decimal degrees for routing/travel estimates.
             */
            latitude?: number | null;
            /** Format: double */
            longitude?: number | null;
            name: string;
            /** Format: uuid */
            org_id: string;
            postal_code?: string | null;
            region?: string | null;
            timezone?: string | null;
        };
        SkillListQuery: {
            /**
             * @description When present, every version of this slug (newest first) instead of
             *     the latest version of each slug.
             */
            slug?: string | null;
        };
        /**
         * @description An org-level business-hours calendar row (`sla_calendars`). The math
         *     type is [`BusinessCalendar`]; this carries its identity.
         */
        SlaCalendar: {
            /**
             * @description Flattened calendar body (timezone/windows/holidays/maintenance);
             *     `calendar.name` mirrors `name` on write.
             */
            calendar: Record<string, never>;
            /** Format: uuid */
            id: string;
            name: string;
            /** Format: uuid */
            org_id: string;
        };
        SlaVerdictRequest: {
            body?: string | null;
            /**
             * @description Extra recipients from the ladder step; the assignee is added
             *     server-side.
             */
            notify_user_ids: string[];
            /** Format: uuid */
            org_id: string;
            /**
             * Format: uuid
             * @description Ladder step that fired (ledger key).
             */
            step_id: string;
            /** @description 'first_response' | 'resolution' */
            target: string;
            /** Format: uuid */
            ticket_id: string;
            /** @description Human-facing headline, e.g. "[MER-12] SLA risk on resolution". */
            title: string;
            /** @description 'risk' | 'breach' | 'escalate' */
            trigger: string;
        };
        /** @description One spec filter: catalog column + operator + typed operand. */
        SpecFilter: {
            /**
             * @description Column name from the view's FILTERABLE inventory (the narrower v1
             *     allowlist — filtered-but-unselected columns surprise consumers).
             */
            column: string;
            op: components["schemas"]["FilterOp"];
            value?: null | components["schemas"]["FilterValue"];
        };
        SsoExchangeRequest: {
            /** Format: uuid */
            code: string;
        };
        SsoExchangeResponse: components["schemas"]["TokenPair"] | {
            linked: boolean;
        };
        /**
         * @description POST /imports/batches/{id}/stage request: uploads ONE chunk of rows to
         *     the ObjectStore under a deterministic key without executing anything.
         */
        StageChunkRequest: components["schemas"]["RowsPayload"] & {
            /**
             * Format: uuid
             * @description Optional saved field-mapping template applied at STAGE time, so the
             *     stored chunk is already in canonical columns.
             */
            template_id?: string | null;
        };
        StageChunkResponse: {
            /** Format: uuid */
            batch_id: string;
            /**
             * Format: int32
             * @description Index of the chunk just staged (its deterministic key suffix).
             */
            chunk_index: number;
            /**
             * Format: int32
             * @description Total staged chunks after this upload — the worker's runbook length.
             */
            staged_chunks: number;
        };
        StartTimerRequest: components["schemas"]["MutationParams"] & {
            /** Format: uuid */
            ticket_id: string;
        };
        StopTimerRequest: components["schemas"]["MutationParams"] & {
            billable?: boolean | null;
            notes?: string | null;
            reason?: string | null;
            /**
             * Format: uuid
             * @description Omit to stop the caller's active timer.
             */
            session_id?: string | null;
        };
        TaskListQuery: {
            /** Format: uuid */
            assignee_id?: string | null;
            /** Format: int64 */
            limit?: number;
            milestone?: boolean | null;
            /** Format: int64 */
            offset?: number;
            /** Format: uuid */
            phase_id?: string | null;
            status?: null | components["schemas"]["TaskStatus"];
        };
        TaskMutationResponse: components["schemas"]["ProjectTask"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description Lifecycle of a task. `completed` is reached ONLY through the dedicated
         *     complete endpoint, which enforces dependency readiness and emits the
         *     completion (+ milestone billing seam) events.
         * @enum {string}
         */
        TaskStatus: "pending" | "in_progress" | "blocked" | "completed" | "cancelled";
        TeamRangeQuery: {
            /**
             * Format: date-time
             * @description Inclusive window start (default: today's UTC midnight minus 1 day).
             */
            from?: string | null;
            /**
             * Format: date-time
             * @description Exclusive window end (default: `from` + 8 days).
             */
            to?: string | null;
        };
        /**
         * @description One technician column: capacity stats plus everything the board draws
         *     in the column (bookings, on-call, exclusions, availability windows).
         */
        TechnicianView: {
            appointments: components["schemas"]["DispatchAppointment"][];
            availability_windows: components["schemas"]["AvailabilityWindow"][];
            /**
             * Format: int64
             * @description Working minutes covered by availability windows minus exclusions
             *     over the day window (`core_domain::scheduling::covered_minutes`).
             */
            available_minutes: number;
            /**
             * Format: int64
             * @description Union of live booking minutes clipped to the day window.
             */
            booked_minutes: number;
            display_name: string;
            exclusions: components["schemas"]["CalendarExclusion"][];
            on_call: components["schemas"]["OnCallBlock"][];
            timezone: string;
            /** Format: uuid */
            user_id: string;
            /**
             * Format: int64
             * @description 0–100+, where >100 is over-capacity (the Ember warning state).
             */
            utilization_percent: number;
        };
        TemplateListQuery: {
            /** @description Narrow the listing to one source system. */
            source_system?: string | null;
        };
        TemplateMutationResponse: components["schemas"]["ProjectTemplate"];
        TemplateViewResponse: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by: string;
            created_by_kind: string;
            entity_kind: components["schemas"]["EntityKind"];
            /** Format: uuid */
            id: string;
            mapping: Record<string, never>;
            name: string;
            source_system: string;
            /** Format: date-time */
            updated_at: string;
        };
        /** @description Request to test the connection to an AI provider. */
        TestConnectionRequest: {
            /** @description Provider to test. If omitted, tests the currently active provider in settings. */
            provider?: string | null;
            /** @description Optional prompt to test with. */
            test_prompt?: string | null;
        };
        /** @description Result of testing an AI provider connection. */
        TestConnectionResponse: {
            /** Format: int64 */
            latency_ms?: number | null;
            /** @description Descriptive status message or reason on failure. */
            message: string;
            ok: boolean;
            provider: string;
        };
        TestFireResponse: {
            /**
             * Format: uuid
             * @description The envelope id to look for in the delivery log.
             */
            event_id: string;
        };
        Ticket: {
            /** Format: uuid */
            assignee_id?: string | null;
            /** Format: uuid */
            board_id: string;
            body?: string | null;
            /** @description Denormalized from the board status for cheap filtering/sorting. */
            category: components["schemas"]["TicketStatusCategory"];
            /** Format: uuid */
            client_id?: string | null;
            /** Format: uuid */
            contact_id?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            due_at?: string | null;
            /** Format: uuid */
            id: string;
            number: components["schemas"]["TicketNumber"];
            /** Format: uuid */
            org_id: string;
            priority: components["schemas"]["Priority"];
            /** Format: uuid */
            requester_id?: string | null;
            /** Format: date-time */
            resolved_at?: string | null;
            /** Format: uuid */
            site_id?: string | null;
            /** Format: uuid */
            status_id: string;
            title: string;
            /** Format: date-time */
            updated_at: string;
        };
        /**
         * @description Same additive envelope for checklist writes: the full checklist view at
         *     the top level (unchanged shape) plus the read-your-write metadata for
         *     the affected item.
         */
        TicketChecklistMutationResponse: components["schemas"]["TicketChecklistView"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description GET response for a ticket's checklist: instances, derived progress,
         *     and the board's live enforcement posture. `blocking` lists the unchecked
         *     REQUIRED items when enforcement is active (empty otherwise).
         */
        TicketChecklistView: {
            blocking: string[];
            enforcement_active: boolean;
            items: components["schemas"]["ChecklistItem"][];
            progress: components["schemas"]["ChecklistProgress"];
        };
        /**
         * @description A structured ticket note. Taxonomy: internal / client-visible /
         *     in-house update. `kind` gates portal visibility and email fan-out.
         */
        TicketComment: {
            /** Format: uuid */
            author_id?: string | null;
            body: string;
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            kind: components["schemas"]["NoteKind"];
            /** Format: uuid */
            org_id: string;
            /** Format: uuid */
            ticket_id: string;
            /** Format: date-time */
            updated_at?: string | null;
        };
        /** @description Same additive envelope for comments (plain comment body). */
        TicketCommentMutationResponse: components["schemas"]["TicketComment"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /** @description One stored ticket value with its definition context. */
        TicketCustomFieldValue: {
            /** Format: uuid */
            field_id: string;
            kind: string;
            name: string;
            required: boolean;
            /** Format: int32 */
            sort_order: number;
            value: unknown;
        };
        /**
         * @description One value assignment on a ticket create/patch: raw JSON validated against
         *     its definition server-side.
         */
        TicketFieldInput: {
            /** Format: uuid */
            field_id: string;
            value: unknown;
        };
        TicketListQuery: {
            /** Format: uuid */
            assignee_id?: string | null;
            /** Format: uuid */
            board_id?: string | null;
            category?: null | components["schemas"]["TicketStatusCategory"];
            /** Format: uuid */
            client_id?: string | null;
            /** Format: int64 */
            limit?: number;
            /** Format: int64 */
            offset?: number;
            priority?: null | components["schemas"]["Priority"];
        };
        /**
         * @description Instant-first mutation response (IF2): the enriched ticket fields at the
         *     top level (unchanged shape) plus the additive `mutation` metadata — the
         *     applied snapshot, monotonic per-entity `seq`/`version`, and the prior
         *     and new values that make undo a core capability.
         */
        TicketMutationResponse: components["schemas"]["TicketWithFields"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /**
         * @description Human-facing ticket number, e.g. `MER-2841`. Globally unique per org,
         *     assigned from a per-org sequence.
         */
        TicketNumber: string;
        /**
         * @description The reverted ticket with the mutation envelope, plus what was reverted.
         *     Deserializable so an IF1 idempotent replay can reconstruct the exact
         *     stored response.
         */
        TicketRevertResponse: components["schemas"]["TicketWithFields"] & {
            mutation: components["schemas"]["MutationMeta"];
            revert: components["schemas"]["RevertDetail"];
        };
        /**
         * @description Per-ticket SLA clock state. Accumulates paused business seconds so a
         *     Waiting stretch does not consume the resolution budget.
         */
        TicketSlaState: {
            breached: boolean;
            /** Format: date-time */
            first_responded_at?: string | null;
            /** Format: date-time */
            first_response_due?: string | null;
            /** Format: uuid */
            org_id: string;
            /**
             * Format: int64
             * @description Business seconds accumulated while the clock was paused.
             */
            paused_business_secs: number;
            /**
             * Format: date-time
             * @description When the current pause began (None while running).
             */
            paused_since?: string | null;
            /** Format: uuid */
            policy_id: string;
            /** Format: date-time */
            resolution_due?: string | null;
            /** Format: uuid */
            ticket_id: string;
        };
        /** @description Live SLA clock readout for one ticket (badge + countdown surfaces). */
        TicketSlaStatus: {
            breached: boolean;
            /**
             * Format: int64
             * @description Business minutes left on the resolution budget (negative = over).
             */
            minutes_remaining?: number | null;
            /** @description True while a Waiting-category pause is open. */
            paused: boolean;
            state: components["schemas"]["TicketSlaState"];
        };
        /**
         * @description The four semantic categories every ticket status maps to (§5).
         *     Customization (labels, ordering, per-board sets) can never add, remove, or
         *     re-bind these: SLA timers, pause logic, automations, billing, and reports
         *     bind to the category, not the label.
         * @enum {string}
         */
        TicketStatusCategory: "open" | "in_progress" | "waiting" | "closed";
        /** @description Same additive envelope for status transitions (plain ticket body). */
        TicketStatusMutationResponse: components["schemas"]["Ticket"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /** @description A ticket payload enriched with its custom-field values (detail + list). */
        TicketWithFields: components["schemas"]["Ticket"] & {
            custom_fields: components["schemas"]["TicketCustomFieldValue"][];
        };
        /**
         * @description A durable, reviewable record of worked time. Created from a timer session
         *     or entered manually; subject to bulk review and approval workflows.
         */
        TimeEntry: {
            billable: boolean;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            ended_at: string;
            /** Format: uuid */
            id: string;
            /**
             * Format: int64
             * @description Worked seconds — authoritative, derived on write, never recomputed.
             */
            minutes: number;
            notes?: string | null;
            /** Format: uuid */
            org_id: string;
            /** Format: date-time */
            started_at: string;
            /** Format: uuid */
            ticket_id: string;
            /**
             * Format: uuid
             * @description Origin: timer-derived entries keep their session for audit.
             */
            timer_session_id?: string | null;
            /** Format: uuid */
            user_id: string;
            work_type?: string | null;
        };
        /** @description Same additive envelope for the stop flow (time-entry body). */
        TimeEntryMutationResponse: components["schemas"]["TimeEntry"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        TimeSettingsResponse: {
            /**
             * Format: int32
             * @description Local hour (member timezones) the EOD audit fires at.
             */
            eod_nudge_hour: number;
            eod_nudges_enabled: boolean;
            /**
             * Format: int32
             * @description Active days logging under this many minutes are gap days.
             */
            time_gap_min_minutes: number;
        };
        /**
         * @description Instant-first mutation response (IF2): session fields at the top level
         *     (unchanged shape) plus the additive `mutation` metadata.
         */
        TimerMutationResponse: components["schemas"]["TimerSession"] & {
            mutation: components["schemas"]["MutationMeta"];
        };
        /** @description A contiguous running interval within a timer session. */
        TimerSegment: {
            end_kind?: null | components["schemas"]["SegmentEnd"];
            /**
             * Format: date-time
             * @description None while this segment is running.
             */
            ended_at?: string | null;
            /** @description Free-text reason recorded at pause time ("on call with vendor"). */
            reason?: string | null;
            /** Format: date-time */
            started_at: string;
        };
        /**
         * @description One user's timer against one ticket. At most one session per user is
         *     ever running — enforced by [`TimerSession::start_switch`].
         */
        TimerSession: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            org_id: string;
            segments: components["schemas"]["TimerSegment"][];
            /** Format: date-time */
            started_at: string;
            /** Format: date-time */
            stopped_at?: string | null;
            /** Format: uuid */
            ticket_id: string;
            /** Format: uuid */
            user_id: string;
        };
        /** @description Token pair returned by all auth flows. */
        TokenPair: {
            access_token: string;
            /**
             * Format: int64
             * @description Access token TTL in seconds.
             */
            expires_in: number;
            refresh_token: string;
            /** Format: uuid */
            session_id: string;
        };
        TotpDisableRequest: {
            /** @description Current TOTP code or a recovery code. */
            code: string;
            password: string;
        };
        TotpEnrollBeginResponse: {
            /** @description Standard otpauth:// URI (renderable as QR by clients). */
            otpauth_url: string;
            /** @description Base32 secret for manual authenticator entry. Shown exactly once. */
            secret_base32: string;
        };
        TotpEnrollVerifyRequest: {
            code: string;
        };
        /**
         * @description One directed edge in a board's explicit transition matrix. Both ids must
         *     belong to statuses of the SAME board (enforced here, and by composite FKs
         *     in storage).
         */
        TransitionEdge: {
            /** Format: uuid */
            from_status_id: string;
            /** Format: uuid */
            to_status_id: string;
        };
        /** @description One field of one applied write — per-field undo granularity. */
        UndoFieldStep: {
            /** Format: uuid */
            actor_id: string;
            actor_kind: string;
            changed: boolean;
            created_at: string;
            field: string;
            /** @description Value after the write (absent keys read as `null`). */
            new_value: Record<string, never> | null;
            op: string;
            /** @description Value before the write (absent keys read as `null`). */
            prior: Record<string, never> | null;
            /** Format: int64 */
            seq: number;
            /** Format: uuid */
            write_id: string;
        };
        /**
         * @description The record's undo posture: `open` = near-total undo; `reverts_only` =
         *     explicit actioned per-field reverts only (record is closed).
         * @enum {string}
         */
        UndoMode: "open" | "reverts_only";
        UndoStackQuery: {
            /**
             * Format: int64
             * @description Maximum steps / field rows returned (1–200, default 50). Newest
             *     first; the first step's `seq` is the record's current version.
             */
            limit?: number | null;
        };
        UndoStackResponse: {
            /**
             * Format: int64
             * @description The record's current version — echo it as `version` on a revert.
             */
            current_version: number;
            entity: string;
            /** Format: uuid */
            entity_id: string;
            /** @description Per-field history, newest write first. */
            fields: components["schemas"]["UndoFieldStep"][];
            /** @description Near-total (`open`) vs actioned-reverts-only (`reverts_only`). */
            mode: components["schemas"]["UndoMode"];
            /** @description Applied writes, newest first. */
            steps: components["schemas"]["UndoStep"][];
        };
        /** @description One applied write on the record — one step of the undo stack. */
        UndoStep: {
            /** Format: uuid */
            actor_id: string;
            /** @description `human` | `agent` | ... (the audit actor vocabulary). */
            actor_kind: string;
            /** @description Applied entity snapshot. */
            after: Record<string, never>;
            /** @description Prior entity snapshot; `null` on creates. */
            before: Record<string, never> | null;
            created_at: string;
            idempotency_key?: string | null;
            /** @description Machine op name (`create`, `update`, `status_change`, `revert`, ...). */
            op: string;
            /**
             * @description The revert path can restore this write's prior values (`create`
             *     steps are visible history but not value-restorable).
             */
            restorable: boolean;
            /**
             * Format: int64
             * @description Per-entity monotonic write number (the version this write produced).
             */
            seq: number;
            /**
             * Format: uuid
             * @description Trail row id (stable reference for actioned reverts).
             */
            write_id: string;
        };
        UnreadCount: {
            /** Format: int64 */
            count: number;
        };
        UpdateArticleRequest: components["schemas"]["MutationParams"] & {
            body?: string | null;
            /**
             * Format: uuid
             * @description `null` clears the category; omit to leave it untouched.
             */
            category_id?: string | null;
            client_visible?: boolean | null;
            status?: null | components["schemas"]["KbArticleStatus"];
            title?: string | null;
        };
        UpdateCalendarRequest: {
            holidays?: unknown;
            maintenance?: unknown;
            name?: string | null;
            timezone?: string | null;
            windows?: unknown;
        };
        UpdateCategoryRequest: components["schemas"]["MutationParams"] & {
            client_visible?: boolean | null;
            description?: string | null;
            name?: string | null;
            /** Format: int32 */
            position?: number | null;
        };
        UpdateDocumentRequest: components["schemas"]["MutationParams"] & {
            body?: string | null;
            change_summary?: string | null;
            title?: string | null;
        };
        UpdateFieldDefRequest: {
            name?: string | null;
            /** @description Replaces the option set for choice kinds. */
            options?: string[] | null;
            required?: boolean | null;
            /** Format: int32 */
            sort_order?: number | null;
        };
        UpdateGroupRequest: {
            name?: string | null;
            /** Format: int32 */
            position?: number | null;
        };
        UpdatePreferenceRequest: {
            email: boolean;
            in_app: boolean;
            kind: components["schemas"]["NotificationKind"];
            /** @description `HH:MM` pair; both bounds together or neither. */
            quiet_from?: string | null;
            quiet_until?: string | null;
            /** @description IANA timezone for the window; defaults to UTC. */
            timezone?: string | null;
        };
        /**
         * @description PUT /api/report-definitions/{id} body — full-replace semantics: every
         *     member is applied; `description: null` clears it.
         */
        UpdateReportDefinitionRequest: components["schemas"]["MutationParams"] & {
            description?: string | null;
            name: string;
            scope?: null | components["schemas"]["DefinitionScope"];
            spec: components["schemas"]["ReportSpec"];
        };
        /**
         * @description PUT /api/report-schedules/{id} body — full-replace semantics; every
         *     member is applied and the next-run preview recomputes.
         */
        UpdateReportScheduleRequest: components["schemas"]["MutationParams"] & {
            cadence: components["schemas"]["ScheduleCadence"];
            /** Format: int32 */
            day_of_month?: number | null;
            /** Format: int32 */
            day_of_week?: number | null;
            enabled?: boolean | null;
            name: string;
            recipients: string[];
            /** Format: uuid */
            report_definition_id: string;
            time_of_day: string;
            timezone: string;
            /** Format: int32 */
            window_days?: number | null;
        };
        UpdateRuleRequest: {
            actions?: unknown;
            /**
             * Format: uuid
             * @description `Some(Some(id))` scopes; `Some(None)` clears to all boards.
             */
            board_id?: string | null;
            conditions?: unknown;
            enabled?: boolean | null;
            name?: string | null;
            trigger_event?: string | null;
        };
        UpdateSettingRequest: {
            key: string;
            /** @description Must satisfy the declaration's kind or the request is rejected 422. */
            value: unknown;
        };
        UpdateStatusRequest: {
            category?: null | components["schemas"]["TicketStatusCategory"];
            is_entry?: boolean | null;
            name?: string | null;
            /** Format: int32 */
            sort_order?: number | null;
        };
        /** @description PUT /api/imports/templates/{id} request — all fields optional. */
        UpdateTemplateRequest: {
            entity_kind: components["schemas"]["EntityKind"];
            mapping: Record<string, never>;
            name?: string | null;
            source_system?: string | null;
        };
        UpdateTimeSettingsRequest: {
            /** Format: int32 */
            eod_nudge_hour: number;
            eod_nudges_enabled: boolean;
            /** Format: int32 */
            time_gap_min_minutes: number;
        };
        UpdateWebhookRequest: {
            enabled?: boolean | null;
            event_types?: string[] | null;
            url?: string | null;
        };
        UpsertMemberRequest: {
            role: components["schemas"]["BoardRole"];
            /** Format: uuid */
            user_id: string;
        };
        UpsertProviderRequest: {
            client_id: string;
            /** @description Write-only: stored encrypted at rest, never returned. */
            client_secret: string;
            /** @description Typically `{issuer}/.well-known/openid-configuration`. */
            discovery_url: string;
            display_name: string;
            enabled?: boolean;
            issuer: string;
            jit_provisioning?: boolean;
        };
        /** @description One effective value: registry default overlaid by any stored override. */
        ValueView: {
            key: string;
            /** @description JSON payload typed by the declaration's kind. */
            value: unknown;
        };
        VerdictResponse: {
            /**
             * @description False when the ledger had already recorded this fire (idempotent
             *     replay after a worker retry) — nothing was written or notified.
             */
            applied: boolean;
            reason?: string | null;
        };
        /**
         * @description Versioned read-model views. Variant ↔ physical view name is frozen
         *     alongside the schema-version suffix; adding a view is additive.
         *
         *     The serde encoding is the view SLUG (`snake_case`) — the same string the
         *     stored-spec `kind` column carries (R3 registry backing) — so a definition
         *     round-trips through JSON without a parallel name table.
         * @enum {string}
         */
        ViewId: "tickets" | "time_entries" | "sla_outcomes" | "gap_days" | "first_response";
        ViewListQuery: {
            /**
             * Format: uuid
             * @description Scope to one project's views; omit for the org's ENTIRE view set.
             */
            project_id?: string | null;
        };
        ViewMutationResponse: components["schemas"]["ProjectView"];
        /**
         * @description Visualization hint the report builder renders with (ADR 0001: Recharts
         *     bar/line/pie/table on the frontend). Pure presentation metadata — it
         *     never influences the compiled SQL.
         * @enum {string}
         */
        VizHint: "bar" | "line" | "pie" | "table";
        VoidInvoiceRequest: components["schemas"]["MutationParams"];
        WebauthnBeginResponse: {
            /** @description Opaque ceremony binding — pass back to finish. */
            challenge_token: string;
            /** @description CreationOptions for navigator.credentials.create()/get(). */
            public_key: Record<string, never>;
        };
        WebauthnCredentialInfo: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            id: string;
            label: string;
            /** Format: date-time */
            last_used_at?: string | null;
        };
        WebauthnLoginFinishRequest: {
            challenge_token: string;
            /** @description The PublicKeyCredential from navigator.credentials.get(). */
            credential: Record<string, never>;
        };
        WebauthnRegisterFinishRequest: {
            challenge_token: string;
            /** @description The PublicKeyCredential from navigator.credentials.create(). */
            credential: Record<string, never>;
            /** @description Label shown on the security settings page. */
            label?: string | null;
        };
        WebauthnRemoveRequest: {
            password: string;
        };
        WebhookList: {
            webhooks: components["schemas"]["WebhookView"][];
        };
        /**
         * @description Returned by create ONLY — carries the signing secret in plaintext one
         *     final time. There is deliberately no read-back path.
         */
        WebhookSecretResponse: {
            secret: string;
            webhook: components["schemas"]["WebhookView"];
        };
        WebhookView: {
            /** Format: date-time */
            created_at: string;
            /** Format: uuid */
            created_by?: string | null;
            enabled: boolean;
            /** @description Empty array = wildcard (all event types). */
            event_types: string[];
            /** Format: uuid */
            id: string;
            url: string;
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    agent_proposals_list: {
        parameters: {
            query?: {
                /** @description Filter by lifecycle state. */
                status?: string | null;
                /** @description Opaque keyset cursor from a previous page's `next_cursor`. */
                cursor?: string | null;
                /** @description Page size, clamped to [1, 200]. */
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentProposalPage"];
                };
            };
        };
    };
    agent_proposals_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateProposalRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CreateProposalResponse"];
                };
            };
            /** @description capability not granted (deny-by-default) */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    agent_proposals_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentProposalView"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    agent_proposals_approve: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DecideProposalRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentProposalView"];
                };
            };
            /** @description not a permitted approver */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description not in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description proposal already decided */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    agent_proposals_decline: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DecideProposalRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentProposalView"];
                };
            };
            /** @description not a permitted approver */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description not in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description proposal already decided */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    agent_proposals_withdraw: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentProposalView"];
                };
            };
            /** @description only the proposing agent or an admin may withdraw */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description not in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description proposal already decided */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    agent_skills_list: {
        parameters: {
            query?: {
                /**
                 * @description When present, every version of this slug (newest first) instead of
                 *     the latest version of each slug.
                 */
                slug?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentSkillList"];
                };
            };
        };
    };
    agent_skills_publish: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PublishSkillRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentSkillView"];
                };
            };
            /** @description owner/admin surface */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid slug or manifest */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    agent_skills_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AgentSkillView"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    alert_sources_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AlertSourceList"];
                };
            };
        };
    };
    alert_sources_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateAlertSourceRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AlertSourceMutationResponse"];
                };
            };
            /** @description source_ref collision or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    alert_sources_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchAlertSourceRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AlertSourceMutationResponse"];
                };
            };
            /** @description source not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    alerts_list: {
        parameters: {
            query?: {
                source_id?: string | null;
                ticket_id?: string | null;
                limit?: number;
                offset?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AlertListResponse"];
                };
            };
        };
    };
    alerts_ingest: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Public ingest routing id of an alert source */
                source_ref: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["IngestAlertRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["IngestResponse"];
                };
            };
            /** @description missing or invalid signature */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description alert source disabled */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description unknown source_ref */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    api_keys_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiKeyList"];
                };
            };
        };
    };
    api_keys_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateApiKeyRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiKeyCreatedResponse"];
                };
            };
        };
    };
    api_keys_revoke: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    appointment_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateAppointmentRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Appointment"];
                };
            };
            /** @description slot already booked */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    appointment_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Appointment"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    appointment_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchAppointmentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Appointment"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    appointment_cancel: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Appointment"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    appointment_move: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["MoveAppointmentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Appointment"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description target slot already booked */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    attachments_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AttachmentDeleted"];
                };
            };
        };
    };
    attachments_download: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/octet-stream": number[];
                };
            };
        };
    };
    audit_list: {
        parameters: {
            query?: {
                /** @description Legacy cursor (kept for compatibility): strictly older than this. */
                before?: string | null;
                /** @description Keyset cursor from a previous page's `next_cursor`. */
                cursor?: string | null;
                /** @description Actor filter: user uuid or "system". */
                actor?: string | null;
                /** @description Action prefix filter, e.g. "auth" matches every auth.* event. */
                action?: string | null;
                /** @description Inclusive lower bound on occurred_at. */
                from?: string | null;
                /** @description Inclusive upper bound on occurred_at. */
                to?: string | null;
                limit?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AuditPage"];
                };
            };
        };
    };
    handlers_login: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LoginRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LoginResponse"];
                };
            };
            /** @description bad credentials */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    handlers_logout: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RefreshRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_recovery_regenerate: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RegenerateRecoveryRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RecoveryCodesResponse"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_totp_disable: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["TotpDisableRequest"];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_totp_enroll_begin: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TotpEnrollBeginResponse"];
                };
            };
        };
    };
    mfa_totp_enroll_verify: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["TotpEnrollVerifyRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RecoveryCodesResponse"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_verify: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["MfaVerifyRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TokenPair"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_webauthn_login_begin: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["MfaTokenRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WebauthnBeginResponse"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_webauthn_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WebauthnCredentialInfo"][];
                };
            };
        };
    };
    mfa_webauthn_remove: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["WebauthnRemoveRequest"];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_webauthn_login_finish: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["WebauthnLoginFinishRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TokenPair"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mfa_webauthn_register_begin: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WebauthnBeginResponse"];
                };
            };
        };
    };
    mfa_webauthn_register_finish: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["WebauthnRegisterFinishRequest"];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    oidc_providers_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OidcProviderInfo"][];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    oidc_providers_public: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PublicProvider"][];
                };
            };
        };
    };
    oidc_provider_upsert: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                name: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpsertProviderRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OidcProviderInfo"];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    oidc_provider_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                name: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    oidc_callback: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                name: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description redirect to web app */
            303: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    oidc_link_begin: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                name: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LinkBeginResponse"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    oidc_login_redirect: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                name: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description redirect to provider */
            303: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    handlers_refresh: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RefreshRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TokenPair"];
                };
            };
        };
    };
    handlers_register: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RegisterRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TokenPair"];
                };
            };
            /** @description slug or email already exists */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation error */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    auth_sessions_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SessionInfo"][];
                };
            };
        };
    };
    auth_session_revoke: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    automation_rules_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": Record<string, never>[];
                };
            };
        };
    };
    automation_rules_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateRuleRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": Record<string, never>;
                };
            };
        };
    };
    automation_rules_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": Record<string, never>;
                };
            };
        };
    };
    automation_rules_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateRuleRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": Record<string, never>;
                };
            };
        };
    };
    availability_list: {
        parameters: {
            query?: {
                /** @description Whose windows. Defaults to the caller; another user requires owner/admin. */
                user_id?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AvailabilityListResponse"];
                };
            };
        };
    };
    availability_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateAvailabilityRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AvailabilityWindow"];
                };
            };
        };
    };
    availability_delete: {
        parameters: {
            query: {
                /** @description Owner of the row being deleted (must match self-or-admin rule). */
                user_id: string;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    billing_invoice_runs_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoiceRun"][];
                };
            };
        };
    };
    billing_invoice_runs_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateInvoiceRunRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoiceRunResponse"];
                };
            };
            /** @description mixed-currency run or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failure */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    billing_invoice_runs_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoiceRun"];
                };
            };
            /** @description run not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    billing_preview: {
        parameters: {
            query?: {
                /** @description Window start (inclusive), ISO date. */
                from?: string | null;
                /** @description Window end (inclusive), ISO date. */
                to?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OrgBillingPreview"];
                };
            };
            /** @description missing or inverted window */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    board_groups_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardGroup"][];
                };
            };
        };
    };
    board_groups_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateGroupRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardGroup"];
                };
            };
        };
    };
    board_groups_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    board_groups_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateGroupRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardGroup"];
                };
            };
        };
    };
    boards_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Board"][];
                };
            };
        };
    };
    boards_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateBoardRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Board"];
                };
            };
        };
    };
    boards_toggle_favorite: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["FavoriteRequest"];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    boards_nav: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardNavEntry"][];
                };
            };
        };
    };
    boards_get_checklist_template: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChecklistTemplateView"];
                };
            };
        };
    };
    boards_put_checklist_template: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PutChecklistTemplateRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChecklistTemplateView"];
                };
            };
        };
    };
    boards_delete_checklist_template: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    fields_list_for_board: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["FieldDefView"][];
                };
            };
        };
    };
    fields_create_def: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateFieldDefRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["FieldDefView"];
                };
            };
            /** @description invalid definition */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    fields_delete_def: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                field_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    fields_update_def: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                field_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateFieldDefRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["FieldDefView"];
                };
            };
        };
    };
    boards_assign_group: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AssignGroupRequest"];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    boards_list_members: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardMemberView"][];
                };
            };
        };
    };
    boards_upsert_member: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpsertMemberRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardMemberView"];
                };
            };
        };
    };
    boards_remove_member: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                user_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    boards_list_statuses: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardStatus"][];
                };
            };
        };
    };
    boards_add_status: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateStatusRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardStatus"];
                };
            };
        };
    };
    boards_delete_status: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                status_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    boards_update_status: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                status_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateStatusRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardStatus"];
                };
            };
        };
    };
    boards_list_transitions: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardTransitionsResponse"];
                };
            };
        };
    };
    boards_set_transitions: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetTransitionsRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardTransitionsResponse"];
                };
            };
        };
    };
    calendar_exclusion_list: {
        parameters: {
            query?: {
                user_id?: string | null;
                from?: string | null;
                to?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CalendarExclusion"][];
                };
            };
        };
    };
    calendar_exclusion_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateExclusionRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CalendarExclusion"];
                };
            };
        };
    };
    calendar_exclusion_delete: {
        parameters: {
            query: {
                /** @description Owner of the row being deleted (must match self-or-admin rule). */
                user_id: string;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    calendar_day: {
        parameters: {
            query?: {
                /**
                 * @description Whose calendar. Defaults to the caller; another user requires
                 *     owner/admin.
                 */
                user_id?: string | null;
                /**
                 * @description Local date in the TARGET user's timezone. Day view: the day itself.
                 *     Week view: any date inside the week (Monday-start). Defaults to
                 *     today in that timezone.
                 */
                date?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CalendarViewResponse"];
                };
            };
        };
    };
    calendar_team: {
        parameters: {
            query?: {
                /** @description Inclusive window start (default: today's UTC midnight minus 1 day). */
                from?: string | null;
                /** @description Exclusive window end (default: `from` + 8 days). */
                to?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CalendarViewResponse"];
                };
            };
        };
    };
    calendar_week: {
        parameters: {
            query?: {
                /**
                 * @description Whose calendar. Defaults to the caller; another user requires
                 *     owner/admin.
                 */
                user_id?: string | null;
                /**
                 * @description Local date in the TARGET user's timezone. Day view: the day itself.
                 *     Week view: any date inside the week (Monday-start). Defaults to
                 *     today in that timezone.
                 */
                date?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CalendarViewResponse"];
                };
            };
        };
    };
    clients_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Client"][];
                };
            };
        };
    };
    clients_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateClientRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ClientMutationResponse"];
                };
            };
            /** @description idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    clients_get_one: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Client"];
                };
            };
        };
    };
    clients_list_assets: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DeviceAsset"][];
                };
            };
        };
    };
    clients_create_asset: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateAssetRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DeviceAsset"];
                };
            };
        };
    };
    board_exposures_list: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardExposureView"][];
                };
            };
        };
    };
    board_exposures_put: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PutExposureRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BoardExposureView"];
                };
            };
            /** @description board or client missing */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    clients_list_contacts: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Contact"][];
                };
            };
        };
    };
    clients_create_contact: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateContactRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Contact"];
                };
            };
        };
    };
    portal_users_list: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalUser"][];
                };
            };
        };
    };
    portal_users_create: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreatePortalUserRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalUser"];
                };
            };
            /** @description email already provisioned */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    clients_list_sites: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Site"][];
                };
            };
        };
    };
    clients_create_site: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateSiteRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Site"];
                };
            };
        };
    };
    contracts_list: {
        parameters: {
            query?: {
                client_id?: string | null;
                /** @description Exact status: `draft|active|ended|cancelled`. */
                status?: string | null;
                /** @description Only contracts effective on this date. */
                effective_on?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Contract"][];
                };
            };
        };
    };
    contracts_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateContractRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ContractMutationResponse"];
                };
            };
            /** @description duplicate contract number, idempotency key reused, or version conflict */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failure */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    contracts_get_one: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ContractDetail"];
                };
            };
        };
    };
    contracts_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ContractDetail"];
                };
            };
            /** @description contract not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description contract has invoices */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    contracts_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchContractRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ContractMutationResponse"];
                };
            };
            /** @description contract not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict, duplicate number, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failure */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    contracts_billing_preview: {
        parameters: {
            query?: {
                /** @description Window start (inclusive), ISO date. */
                from?: string | null;
                /** @description Window end (inclusive), ISO date. */
                to?: string | null;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BillingEvaluation"];
                };
            };
            /** @description contract not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description missing or inverted window */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    dispatch_assign: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AssignRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DispatchAppointment"];
                };
            };
            /** @description slot already booked */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    dispatch_bulk_move: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["BulkMoveRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BulkMoveResponse"];
                };
            };
            /** @description target slot already booked (typed conflict) */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    dispatch_day: {
        parameters: {
            query?: {
                /** @description The day to assemble; defaults to today (UTC). */
                date?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DispatchDayView"];
                };
            };
        };
    };
    documents_list: {
        parameters: {
            query?: {
                /** @description Title substring narrowing; ranked full-text search rides /api/search. */
                q?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Document"][];
                };
            };
        };
    };
    documents_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateDocumentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DocumentMutationResponse"];
                };
            };
            /** @description idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description title/body rejected by document rules */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    documents_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DocumentView"];
                };
            };
            /** @description document not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    documents_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateDocumentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DocumentMutationResponse"];
                };
            };
            /** @description document not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description title/body rejected by document rules */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    documents_restore: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RestoreDocumentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DocumentMutationResponse"];
                };
            };
            /** @description document or restore target not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description restore rejected by document rules */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    documents_list_versions: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DocumentVersion"][];
                };
            };
            /** @description document not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    documents_get_version: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                version: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DocumentVersion"];
                };
            };
            /** @description document or version not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    imports_list_batches: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BatchViewResponse"][];
                };
            };
        };
    };
    imports_create_batch: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateBatchRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BatchCreatedResponse"];
                };
            };
        };
    };
    imports_get_batch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                batch_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BatchViewResponse"];
                };
            };
        };
    };
    imports_post_chunk: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                batch_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ChunkRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RunReportResponse"];
                };
            };
        };
    };
    imports_finish_batch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                batch_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BatchViewResponse"];
                };
            };
        };
    };
    imports_run_batch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                batch_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RunBatchResponse"];
                };
            };
        };
    };
    imports_stage_chunk: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                batch_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["StageChunkRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["StageChunkResponse"];
                };
            };
        };
    };
    imports_dry_run: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DryRunRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RunReportResponse"];
                };
            };
        };
    };
    imports_list_templates: {
        parameters: {
            query?: {
                /** @description Narrow to one source system */
                source_system?: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateViewResponse"][];
                };
            };
        };
    };
    imports_create_template: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateTemplateRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateViewResponse"];
                };
            };
        };
    };
    imports_get_template: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                template_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateViewResponse"];
                };
            };
        };
    };
    imports_update_template: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                template_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateTemplateRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateViewResponse"];
                };
            };
        };
    };
    imports_delete_template: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                template_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateViewResponse"];
                };
            };
        };
    };
    internal_effects: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EffectsRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": Record<string, never>;
                };
            };
        };
    };
    internal_sla_verdict: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SlaVerdictRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["VerdictResponse"];
                };
            };
        };
    };
    invoices_list: {
        parameters: {
            query?: {
                client_id?: string | null;
                contract_id?: string | null;
                /** @description Exact status: `draft|issued|paid|void`. */
                status?: string | null;
                /**
                 * @description Exact dunning state:
                 *     `none|reminder_sent|overdue|final_notice|collections`.
                 */
                dunning_state?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Invoice"][];
                };
            };
        };
    };
    invoices_get_one: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoiceDetail"];
                };
            };
            /** @description invoice not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    invoices_document: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/html": string;
                };
            };
            /** @description invoice not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    invoices_dunning_transition: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DunningTransitionRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoiceMutationResponse"];
                };
            };
            /** @description invoice not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invoice not issuable, version conflict, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description backwards or same-state transition */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    invoices_payments_list: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoicePayment"][];
                };
            };
            /** @description invoice not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    invoices_payments_create: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RecordPaymentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PaymentMutationResponse"];
                };
            };
            /** @description invoice not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invoice not payable, overpayment, version conflict, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failure */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    invoices_void: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["VoidInvoiceRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvoiceMutationResponse"];
                };
            };
            /** @description invoice not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invoice not voidable, version conflict, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_list_articles: {
        parameters: {
            query?: {
                category_id?: string | null;
                status?: null | components["schemas"]["KbArticleStatus"];
                /** @description Only articles passing the effective client-facing gate. */
                client_visible?: boolean;
                /** @description Title substring narrowing; ranked full-text search rides /api/search. */
                q?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["KbArticle"][];
                };
            };
        };
    };
    kb_create_article: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateArticleRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ArticleMutationResponse"];
                };
            };
            /** @description slug already in use or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description title/body rejected by KB rules */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_get_article: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["KbArticle"];
                };
            };
            /** @description article not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_delete_article: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": unknown;
                };
            };
            /** @description article not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_update_article: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateArticleRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ArticleMutationResponse"];
                };
            };
            /** @description article not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description archived content lock, version conflict, slug reuse, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description title/body rejected by KB rules */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_list_categories: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["KbCategory"][];
                };
            };
        };
    };
    kb_create_category: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateCategoryRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CategoryMutationResponse"];
                };
            };
            /** @description slug already in use or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_delete_category: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": unknown;
                };
            };
            /** @description category not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    kb_update_category: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateCategoryRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CategoryMutationResponse"];
                };
            };
            /** @description category not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict, slug reuse, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mcp_capabilities_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CapabilityGrantList"];
                };
            };
        };
    };
    mcp_capability_put: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PutCapabilityRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CapabilityGrantView"];
                };
            };
            /** @description owner/admin surface */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid capability or dial position */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mcp_capability_revoke: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                capability: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    handlers_me: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MeResponse"];
                };
            };
        };
    };
    notification_prefs_get: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PreferenceView"][];
                };
            };
        };
    };
    notification_prefs_update: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdatePreferenceRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PreferenceView"];
                };
            };
            /** @description invalid quiet-hours window or timezone */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    notifications_list: {
        parameters: {
            query?: {
                unread_only?: boolean;
                limit?: number;
                /** @description Opaque keyset cursor from a previous page's `next_cursor`. */
                cursor?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["NotificationPage"];
                };
            };
        };
    };
    notifications_mark_read_bulk: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["MarkReadBulkRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MarkReadResult"];
                };
            };
            /** @description empty or oversized id list */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    notifications_mark_all_read: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MarkReadResult"];
                };
            };
        };
    };
    notifications_unread_count: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["UnreadCount"];
                };
            };
        };
    };
    notifications_mark_read: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    on_call_list: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /**
                 * @description Whose calendar. Defaults to the caller; another user requires
                 *     owner/admin.
                 */
                user_id: string | null;
                /**
                 * @description Local date in the TARGET user's timezone. Day view: the day itself.
                 *     Week view: any date inside the week (Monday-start). Defaults to
                 *     today in that timezone.
                 */
                date: string | null;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OnCallBlock"][];
                };
            };
        };
    };
    on_call_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateOnCallRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OnCallBlock"];
                };
            };
            /** @description overlapping rotation */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    on_call_delete: {
        parameters: {
            query: {
                /** @description Owner of the row being deleted (must match self-or-admin rule). */
                user_id: string;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    org_profile_get: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OrgProfileResponse"];
                };
            };
        };
    };
    org_profile_patch: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchOrgRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OrgProfileResponse"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    org_export_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ExportCreatedResponse"];
                };
            };
        };
    };
    org_export_download: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                org_id: string;
                export_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ExportBundle"];
                };
            };
        };
    };
    org_manifest: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CustomizationManifest"];
                };
            };
        };
    };
    portal_users_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    portal_users_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchPortalUserRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalUser"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    portal_login: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PortalLoginRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalLoginResponse"];
                };
            };
            /** @description bad credentials */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    portal_me: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalProfile"];
                };
            };
        };
    };
    portal_refresh: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PortalRefreshRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalLoginResponse"];
                };
            };
        };
    };
    portal_list_tickets: {
        parameters: {
            query?: {
                board_id?: string | null;
                category?: null | components["schemas"]["TicketStatusCategory"];
                limit?: number;
                offset?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Ticket"][];
                };
            };
        };
    };
    portal_get_ticket: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalTicketDetail"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    portal_list_attachments: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PortalAttachmentView"][];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    portal_download_attachment: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                attachment_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/octet-stream": number[];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    portal_reply: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PortalReplyRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketComment"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_templates_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectTemplate"][];
                };
            };
        };
    };
    project_templates_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateTemplateRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateMutationResponse"];
                };
            };
            /** @description template name already in use */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid template spec */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_templates_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description template not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_templates_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchTemplateRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TemplateMutationResponse"];
                };
            };
            /** @description template not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid template spec */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_templates_instantiate: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["InstantiateTemplateRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectInstantiateResponse"];
                };
            };
            /** @description template not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid template spec or request */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_views_list: {
        parameters: {
            query?: {
                /** @description Scope to one project's views; omit for the org's ENTIRE view set. */
                project_id?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectView"][];
                };
            };
        };
    };
    project_views_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateViewRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ViewMutationResponse"];
                };
            };
            /** @description project not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description view name already in use in the scope */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_views_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description view not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    project_views_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchViewRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ViewMutationResponse"];
                };
            };
            /** @description view not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_list: {
        parameters: {
            query?: {
                status?: null | components["schemas"]["ProjectStatus"];
                client_id?: string | null;
                limit?: number;
                offset?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Project"][];
                };
            };
        };
    };
    projects_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateProjectRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectMutationResponse"];
                };
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_get_one: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Project"];
                };
            };
            /** @description not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchProjectRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectMutationResponse"];
                };
            };
            /** @description project not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_list_phases: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectPhase"][];
                };
            };
        };
    };
    projects_create_phase: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreatePhaseRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PhaseMutationResponse"];
                };
            };
            /** @description project not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_patch_phase: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                phase_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchPhaseRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PhaseMutationResponse"];
                };
            };
            /** @description phase not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_complete_phase: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                phase_id: string;
            };
            cookie?: never;
        };
        requestBody?: {
            content: {
                "application/json": components["schemas"]["MutationParams"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PhaseMutationResponse"];
                };
            };
            /** @description phase not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description already completed, version conflict, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_list_tasks: {
        parameters: {
            query?: {
                phase_id?: string | null;
                status?: null | components["schemas"]["TaskStatus"];
                assignee_id?: string | null;
                milestone?: boolean | null;
                limit?: number;
                offset?: number;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectTask"][];
                };
            };
        };
    };
    projects_create_task: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateTaskRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskMutationResponse"];
                };
            };
            /** @description project not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_patch_task: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                task_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchTaskRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskMutationResponse"];
                };
            };
            /** @description task not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_complete_task: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                task_id: string;
            };
            cookie?: never;
        };
        requestBody?: {
            content: {
                "application/json": components["schemas"]["MutationParams"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskMutationResponse"];
                };
            };
            /** @description task not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description dependencies unsatisfied, already completed, version conflict, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_get_dependencies: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                task_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DependencyListResponse"];
                };
            };
            /** @description not found */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    projects_set_dependencies: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                task_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetDependenciesRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DependencyListResponse"];
                };
            };
            /** @description task not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description dependency set rejected (self-dep, cross-project, or cycle) */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_definitions_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportDefinitionList"];
                };
            };
        };
    };
    report_definitions_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateReportDefinitionRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportDefinitionMutationResponse"];
                };
            };
            /** @description missing reporting:create */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description name conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description spec failed registry validation */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_definitions_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportDefinitionView"];
                };
            };
            /** @description not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_definitions_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateReportDefinitionRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportDefinitionMutationResponse"];
                };
            };
            /** @description definition outside the caller's writable scope */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description rls_sparse: target not visible */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description spec failed registry validation */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_definitions_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportDefinitionMutationResponse"];
                };
            };
            /** @description definition outside the caller's writable scope */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description rls_sparse: target not visible */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_schedules_list: {
        parameters: {
            query?: {
                /** @description Opaque keyset cursor from a previous page's `next_cursor`. */
                cursor?: string | null;
                /** @description Page size, clamped to [1, 200]. */
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportSchedulePage"];
                };
            };
        };
    };
    report_schedules_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateReportScheduleRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportScheduleMutationResponse"];
                };
            };
            /** @description admin-only surface */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description name conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed (shape, recipients, definition) */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_schedules_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportScheduleView"];
                };
            };
            /** @description not in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_schedules_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateReportScheduleRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportScheduleMutationResponse"];
                };
            };
            /** @description admin-only surface */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description rls_sparse: target not visible */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description name conflict or version conflict */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description validation failed */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_schedules_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportScheduleMutationResponse"];
                };
            };
            /** @description admin-only surface */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description rls_sparse: target not visible */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    report_get: {
        parameters: {
            query?: {
                /** @description Inclusive window start (UTC day, YYYY-MM-DD). Default: 30 trailing days. */
                from?: string | null;
                /** @description Inclusive window end (UTC day). Max span with `from`: 366 days. */
                to?: string | null;
                /** @description Opaque keyset cursor from a previous page's `next_cursor`. */
                cursor?: string | null;
                /** @description Page size, clamped to [1, 500]. */
                limit?: number;
                /** @description `"csv"` streams text/csv instead of the JSON envelope. */
                format?: string | null;
                /**
                 * @description Manager-only narrowing onto one staff member; non-managers are
                 *     ALWAYS forced onto themselves regardless of this parameter.
                 */
                user_id?: string | null;
                /** @description SLA kinds: restrict to one governing policy. */
                policy_id?: string | null;
                /** @description Time rollups: filter billable/non-billable entries. */
                billable?: boolean | null;
                /** @description ticket_aging only: snapshot instant for ages (default: now). */
                as_of?: string | null;
            };
            header?: never;
            path: {
                kind: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Report page (JSON envelope or CSV stream) */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReportPage"];
                };
            };
        };
    };
    scheduling_conflict_check: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ConflictCheckRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ConflictCheckResponse"];
                };
            };
        };
    };
    search_query: {
        parameters: {
            query: {
                /**
                 * @description Free-text query using websearch syntax: `"exact phrases"`,
                 *     `-excluded`, plain terms AND-ed. Tokens ≥3 chars also match as
                 *     substrings via the trigram fallback.
                 */
                q: string;
                limit?: number;
                offset?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SearchResponse"];
                };
            };
        };
    };
    settings_ai_byok_put: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PutByokSecretRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ByokSecretMutationResponse"];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description idempotency key reused or version conflict */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid provider or key */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    settings_ai_byok_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                provider: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ByokSecretMutationResponse"];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description rls_sparse: no key stored for that provider */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    settings_ai_status: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ByokOverviewResponse"];
                };
            };
        };
    };
    settings_ai_test: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["TestConnectionRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TestConnectionResponse"];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    settings_org_get: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ValueView"][];
                };
            };
        };
    };
    settings_org_put: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateSettingRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ValueView"];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description unknown key or invalid value */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    settings_registry: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RegistryResponse"];
                };
            };
        };
    };
    settings_user_get: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ValueView"][];
                };
            };
        };
    };
    settings_user_put: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateSettingRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ValueView"];
                };
            };
            /** @description unknown key or invalid value */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    sla_calendars_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SlaCalendar"][];
                };
            };
        };
    };
    sla_calendars_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateCalendarRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SlaCalendar"];
                };
            };
        };
    };
    sla_calendars_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["HealthResponse"];
                };
            };
        };
    };
    sla_calendars_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateCalendarRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SlaCalendar"];
                };
            };
        };
    };
    sla_policies_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PolicyView"][];
                };
            };
        };
    };
    sla_ladder_get: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["EscalationStep"][];
                };
            };
        };
    };
    sla_ladder_replace: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ReplaceLadderRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["EscalationStep"][];
                };
            };
        };
    };
    sso_exchange: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SsoExchangeRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SsoExchangeResponse"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_list: {
        parameters: {
            query?: {
                board_id?: string | null;
                assignee_id?: string | null;
                client_id?: string | null;
                category?: null | components["schemas"]["TicketStatusCategory"];
                priority?: null | components["schemas"]["Priority"];
                limit?: number;
                offset?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketWithFields"][];
                };
            };
        };
    };
    tickets_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateTicketRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketMutationResponse"];
                };
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description missing required custom fields or invalid value */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_my_work: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MyWorkItem"][];
                };
            };
        };
    };
    tickets_get_one: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketWithFields"];
                };
            };
        };
    };
    tickets_patch: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PatchTicketRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketMutationResponse"];
                };
            };
            /** @description ticket not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid custom-field value */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    attachments_list_for_ticket: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Attachment"][];
                };
            };
        };
    };
    attachments_upload: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/octet-stream": number[];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Attachment"];
                };
            };
        };
    };
    tickets_get_checklist: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketChecklistView"];
                };
            };
        };
    };
    tickets_add_checklist_item: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AddChecklistItemRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketChecklistMutationResponse"];
                };
            };
            /** @description ticket not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_set_checklist_item: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                item_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetChecklistItemRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketChecklistMutationResponse"];
                };
            };
            /** @description ticket or item not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_list_comments: {
        parameters: {
            query?: {
                /**
                 * @description Optional taxonomy filter (`internal`, `client_visible`,
                 *     `in_house_update`). Omit for all kinds (staff view).
                 */
                kind?: null | components["schemas"]["NoteKind"];
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketComment"][];
                };
            };
        };
    };
    tickets_add_comment: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AddCommentRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketCommentMutationResponse"];
                };
            };
            /** @description ticket not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    attachments_list_for_comment: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                comment_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Attachment"][];
                };
            };
        };
    };
    fields_set_ticket_value: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                field_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetTicketFieldValueRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketCustomFieldValue"];
                };
            };
            /** @description value does not validate against the definition */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    fields_clear_ticket_value: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                field_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description field is required and cannot be emptied */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_revert: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RevertTicketRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketRevertResponse"];
                };
            };
            /** @description version conflict, idempotency key reuse, or undo_reverts_only (closed record, whole-record revert) */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description unknown target seq, non-restorable step/field, or nothing to restore */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_sla_status: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketSlaStatus"];
                };
            };
        };
    };
    tickets_change_status: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ChangeStatusRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TicketStatusMutationResponse"];
                };
            };
            /** @description ticket not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description pre-close checklist blocked, version conflict, or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description invalid transition */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    tickets_ticket_time_entries: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimeEntry"][];
                };
            };
        };
    };
    tickets_undo_stack: {
        parameters: {
            query?: {
                /**
                 * @description Maximum steps / field rows returned (1–200, default 50). Newest
                 *     first; the first step's `seq` is the record's current version.
                 */
                limit?: number | null;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["UndoStackResponse"];
                };
            };
            /** @description ticket not visible in scope */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    timers_list_entries: {
        parameters: {
            query?: {
                ticket_id?: string | null;
                user_id?: string | null;
                since?: string | null;
                limit?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimeEntry"][];
                };
            };
        };
    };
    timers_create_entry: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateTimeEntryRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimeEntry"];
                };
            };
        };
    };
    time_gaps_user: {
        parameters: {
            query?: {
                /** @description Inclusive local-day start (YYYY-MM-DD). Defaults to yesterday. */
                from?: string | null;
                /** @description Inclusive local-day end (YYYY-MM-DD). Defaults to `from`. */
                to?: string | null;
                /** @description Manager report only: whose days. Requires owner/admin. */
                user_id?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["GapReportResponse"];
                };
            };
        };
    };
    time_gaps_me: {
        parameters: {
            query?: {
                /** @description Inclusive local-day start (YYYY-MM-DD). Defaults to yesterday. */
                from?: string | null;
                /** @description Inclusive local-day end (YYYY-MM-DD). Defaults to `from`. */
                to?: string | null;
                /** @description Manager report only: whose days. Requires owner/admin. */
                user_id?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["GapReportResponse"];
                };
            };
        };
    };
    time_settings_get: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimeSettingsResponse"];
                };
            };
        };
    };
    time_settings_put: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateTimeSettingsRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimeSettingsResponse"];
                };
            };
        };
    };
    timers_active: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": Record<string, never>;
                };
            };
        };
    };
    timers_pause: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PauseTimerRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimerSession"];
                };
            };
        };
    };
    timers_resume: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PauseTimerRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimerSession"];
                };
            };
        };
    };
    timers_start: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["StartTimerRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimerMutationResponse"];
                };
            };
            /** @description ticket not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    timers_stop: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["StopTimerRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimeEntryMutationResponse"];
                };
            };
            /** @description session not visible in scope (rls_sparse) */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description version conflict or idempotency key reused */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    timers_working_set: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TimerSession"][];
                };
            };
        };
    };
    webhooks_list: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WebhookList"];
                };
            };
        };
    };
    webhooks_create: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateWebhookRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WebhookSecretResponse"];
                };
            };
        };
    };
    webhooks_delete: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    webhooks_update: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UpdateWebhookRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WebhookView"];
                };
            };
        };
    };
    webhooks_deliveries_list: {
        parameters: {
            query?: {
                cursor?: string | null;
                limit?: number;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DeliveryPage"];
                };
            };
        };
    };
    webhooks_test_fire: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TestFireResponse"];
                };
            };
        };
    };
}
