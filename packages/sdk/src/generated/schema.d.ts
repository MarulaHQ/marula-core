export interface paths {
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
        /** POST /api/clients */
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
         *     assignee changes.
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
         *     emits `ticket.checklist_updated` in the same transaction.
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
         *     fan out into the notification center.
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
        };
        AddChecklistItemRequest: {
            text: string;
        };
        AddCommentRequest: {
            /**
             * @description Attachments uploaded earlier against this ticket (composer flow:
             *     upload first, link when the comment is created). Absent/empty in
             *     older clients — purely additive.
             */
            attachment_ids?: string[];
            body: string;
            kind: components["schemas"]["NoteKind"];
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
        ChangeStatusRequest: {
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
        CreateClientRequest: {
            name: string;
            notes?: string | null;
        };
        CreateContactRequest: {
            email?: string | null;
            name: string;
            phone?: string | null;
            title?: string | null;
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
        CreateTicketRequest: {
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
        CreateWebhookRequest: {
            /** @description Exact event types to subscribe; empty subscribes to everything. */
            event_types?: string[];
            /** @description HTTPS endpoint receiving `POST`s. (http:// allowed for local dev.) */
            url: string;
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
        FavoriteRequest: {
            /** Format: uuid */
            board_id: string;
            favorite: boolean;
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
        NotificationKind: "mention" | "assignment" | "sla_risk" | "sla_breach" | "escalation" | "missing_time" | "ticket_update";
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
        PatchOrgRequest: {
            name?: string | null;
            ticket_prefix?: string | null;
        };
        PatchPortalUserRequest: {
            display_name?: string | null;
            /** @description Optional credential reset (hashed before storage). */
            password?: string | null;
            role?: null | components["schemas"]["PortalRole"];
            status?: null | components["schemas"]["PortalUserStatus"];
        };
        PatchTicketRequest: {
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
        PauseTimerRequest: {
            /** @description Optional reason, preserved on the segment forever. */
            reason?: string | null;
            /**
             * Format: uuid
             * @description Omit to act on the caller's active timer.
             */
            session_id?: string | null;
        };
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
        /**
         * @description Industry-standard four-tier priority.
         * @enum {string}
         */
        Priority: "p1" | "p2" | "p3" | "p4";
        PublicProvider: {
            display_name: string;
            /** @description URL path segment for /api/auth/oidc/{name}/login. */
            name: string;
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
        SetChecklistItemRequest: {
            checked: boolean;
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
        SsoExchangeRequest: {
            /** Format: uuid */
            code: string;
        };
        SsoExchangeResponse: components["schemas"]["TokenPair"] | {
            linked: boolean;
        };
        StartTimerRequest: {
            /** Format: uuid */
            ticket_id: string;
        };
        StopTimerRequest: {
            billable?: boolean | null;
            notes?: string | null;
            reason?: string | null;
            /**
             * Format: uuid
             * @description Omit to stop the caller's active timer.
             */
            session_id?: string | null;
        };
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
         * @description Human-facing ticket number, e.g. `MER-2841`. Globally unique per org,
         *     assigned from a per-org sequence.
         */
        TicketNumber: string;
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
        UnreadCount: {
            /** Format: int64 */
            count: number;
        };
        UpdateCalendarRequest: {
            holidays?: unknown;
            maintenance?: unknown;
            name?: string | null;
            timezone?: string | null;
            windows?: unknown;
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
                    "application/json": components["schemas"]["Client"];
                };
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
                    "application/json": components["schemas"]["TicketWithFields"];
                };
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
                    "application/json": components["schemas"]["TicketWithFields"];
                };
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
                    "application/json": components["schemas"]["TicketChecklistView"];
                };
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
                    "application/json": components["schemas"]["TicketChecklistView"];
                };
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
                    "application/json": components["schemas"]["TicketComment"];
                };
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
                    "application/json": components["schemas"]["Ticket"];
                };
            };
            /** @description pre-close checklist incomplete (board enforces) */
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
                    "application/json": components["schemas"]["TimerSession"];
                };
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
                    "application/json": components["schemas"]["TimeEntry"];
                };
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
