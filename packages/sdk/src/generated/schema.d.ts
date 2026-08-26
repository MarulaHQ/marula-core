export interface paths {
    "/api/audit-events": {
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
    "/api/auth/login": {
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
    "/api/auth/logout": {
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
    "/api/auth/mfa/recovery-codes/regenerate": {
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
    "/api/auth/mfa/totp/disable": {
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
    "/api/auth/mfa/totp/enroll-begin": {
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
    "/api/auth/mfa/totp/enroll-verify": {
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
    "/api/auth/mfa/verify": {
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
    "/api/auth/mfa/webauthn/begin": {
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
    "/api/auth/mfa/webauthn/credentials": {
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
    "/api/auth/mfa/webauthn/credentials/{id}/remove": {
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
    "/api/auth/mfa/webauthn/finish": {
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
    "/api/auth/mfa/webauthn/register/begin": {
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
    "/api/auth/mfa/webauthn/register/finish": {
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
    "/api/auth/oidc/providers": {
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
    "/api/auth/oidc/providers/public": {
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
    "/api/auth/oidc/providers/{name}": {
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
    "/api/auth/oidc/{name}/callback": {
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
    "/api/auth/oidc/{name}/link": {
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
    "/api/auth/oidc/{name}/login": {
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
    "/api/auth/refresh": {
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
    "/api/auth/register": {
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
    "/api/auth/sessions": {
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
    "/api/auth/sessions/{id}/revoke": {
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
    "/api/board-groups": {
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
    "/api/board-groups/{id}": {
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
    "/api/boards": {
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
    "/api/boards/favorite": {
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
    "/api/boards/nav": {
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
    "/api/boards/{id}/checklist-template": {
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
    "/api/boards/{id}/group": {
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
    "/api/boards/{id}/members": {
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
    "/api/boards/{id}/members/{user_id}": {
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
    "/api/boards/{id}/statuses": {
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
    "/api/boards/{id}/statuses/{status_id}": {
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
    "/api/boards/{id}/transitions": {
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
    "/api/clients": {
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
    "/api/clients/{id}": {
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
    "/api/clients/{id}/assets": {
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
    "/api/clients/{id}/contacts": {
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
    "/api/clients/{id}/sites": {
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
    "/api/me": {
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
    "/api/notification-preferences": {
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
    "/api/notifications": {
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
    "/api/notifications/mark-read": {
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
    "/api/notifications/read-all": {
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
    "/api/notifications/unread-count": {
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
    "/api/notifications/{id}/read": {
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
    "/api/sso/exchange": {
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
    "/api/tickets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/tickets — filtered, priority-ordered list. */
        get: operations["tickets_list"];
        put?: never;
        /**
         * POST /api/tickets — allocates the human ticket number, initializes SLA
         *     clocks, and emits `ticket.created` — all in one transaction.
         */
        post: operations["tickets_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/tickets/my-work": {
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
    "/api/tickets/{id}": {
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
         * PATCH /api/tickets/{id} — partial update; emits `ticket.updated` and
         *     `ticket.assigned` when the assignee changes.
         */
        patch: operations["tickets_patch"];
        trace?: never;
    };
    "/api/tickets/{id}/checklist": {
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
    "/api/tickets/{id}/checklist/{item_id}": {
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
    "/api/tickets/{id}/comments": {
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
    "/api/tickets/{id}/status": {
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
    "/api/tickets/{id}/time-entries": {
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
    "/api/time-entries": {
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
    "/api/timers/active": {
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
    "/api/timers/pause": {
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
    "/api/timers/resume": {
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
    "/api/timers/start": {
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
    "/api/timers/stop": {
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
    "/api/timers/working-set": {
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
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        /** @description Who performed the audited action. Append-only provenance record. */
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
        };
        AddChecklistItemRequest: {
            text: string;
        };
        AddCommentRequest: {
            body: string;
            kind: components["schemas"]["NoteKind"];
        };
        AssignGroupRequest: {
            /**
             * Format: uuid
             * @description `Some(group)` moves the board into that group; `None` ungroups it.
             */
            group_id?: string | null;
        };
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
        CreateAssetRequest: {
            kind?: string | null;
            name: string;
            tag: string;
        };
        CreateBoardRequest: {
            description?: string | null;
            is_default: boolean;
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
        CreateGroupRequest: {
            name: string;
            /** Format: int32 */
            position?: number | null;
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
        FavoriteRequest: {
            /** Format: uuid */
            board_id: string;
            favorite: boolean;
        };
        HealthResponse: {
            status: string;
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
        SetTransitionsRequest: {
            /**
             * @description The complete allowed matrix. Empty list returns the board to
             *     floor-mode governance.
             */
            edges: components["schemas"]["TransitionEdge"][];
        };
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
        /**
         * @description The four semantic categories every ticket status maps to (§5).
         *     Customization (labels, ordering, per-board sets) can never add, remove, or
         *     re-bind these: SLA timers, pause logic, automations, billing, and reports
         *     bind to the category, not the label.
         * @enum {string}
         */
        TicketStatusCategory: "open" | "in_progress" | "waiting" | "closed";
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
        UpdateStatusRequest: {
            category?: null | components["schemas"]["TicketStatusCategory"];
            is_entry?: boolean | null;
            name?: string | null;
            /** Format: int32 */
            sort_order?: number | null;
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
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
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
                    "application/json": components["schemas"]["Ticket"][];
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
                    "application/json": components["schemas"]["Ticket"];
                };
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
                    "application/json": components["schemas"]["Ticket"];
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
                    "application/json": components["schemas"]["Ticket"];
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
}
