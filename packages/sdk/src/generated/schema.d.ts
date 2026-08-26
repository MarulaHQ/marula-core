export interface paths {
    "/api/audit-events": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/audit-events — append-only audit trail, newest first. */
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
         *     bridge), verifies the argon2 hash under RLS scope, issues tokens.
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
        /** POST /api/auth/logout — revokes the refresh session. */
        post: operations["handlers_logout"];
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
         * POST /api/auth/refresh — rotates the refresh token (old one dies with the
         *     transaction; replay of a rotated token fails).
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
        /** POST /api/boards/favorite — toggles a board favorite for the current user. */
        post: operations["boards_toggle_favorite"];
        delete?: never;
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
         *     semantic category.
         */
        post: operations["boards_add_status"];
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
         *     across all boards (the unified cross-board view from §1).
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
        /** GET /api/tickets/{id}/checklist */
        get: operations["tickets_get_checklist"];
        put?: never;
        /** POST /api/tickets/{id}/checklist — appends a pre-close checklist item. */
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
        /** POST /api/tickets/{id}/checklist/{item_id} — toggles one checklist item. */
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
         *     domain transition rules, pre-close checklist enforcement, SLA pause
         *     bookkeeping, and the `ticket.status_changed` event, all in one tx.
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
        };
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
        ChangeStatusRequest: {
            /** Format: uuid */
            status_id: string;
        };
        /**
         * @description One line of a ticket's pre-close checklist. Closing a ticket whose
         *     category target requires a checklist fails while any item is unchecked.
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
            text: string;
            /** Format: uuid */
            ticket_id: string;
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
        RefreshRequest: {
            refresh_token: string;
            /**
             * Format: uuid
             * @description Session id returned alongside the refresh token.
             */
            session_id: string;
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
        SetChecklistItemRequest: {
            checked: boolean;
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
        UnreadCount: {
            /** Format: int64 */
            count: number;
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
                before?: string | null;
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
                    "application/json": components["schemas"]["AuditEvent"][];
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
                    "application/json": components["schemas"]["TokenPair"];
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
                    "application/json": components["schemas"]["Ticket"][];
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
                    "application/json": components["schemas"]["ChecklistItem"][];
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
                    "application/json": components["schemas"]["ChecklistItem"][];
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
                    "application/json": components["schemas"]["ChecklistItem"][];
                };
            };
        };
    };
    tickets_list_comments: {
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
            /** @description checklist incomplete or invalid transition */
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
