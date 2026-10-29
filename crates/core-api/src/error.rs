//! Typed API errors with HTTP mapping. No panics on the request path.
//!
//! Instant-first contract (docs/instant-first.md IF3): every error body
//! carries a machine-readable `reason` code clients branch on — never prose
//! to parse. The legacy `code` (HTTP status) and `message` keys stay so the
//! change is purely additive for existing consumers:
//!
//! ```json
//! { "error": { "code": 409, "reason": "version_conflict", "message": "...", ...extra } }
//! ```
//!
//! Route handlers added by parallel lanes converge on this envelope for
//! free: returning `ApiError` from any handler is all it takes. Reason
//! vocabulary: `not_found`, `rls_sparse`, `unauthorized`, `forbidden`,
//! `validation_failed`, `conflict`,
//! `org_slug_taken`, `edge_source_unknown`, `edge_assertion_invalid`, `edge_identity_unlinked`, `user_email_taken`, `role_name_taken`,
//! `sku_taken`, `quote_number_taken`, `stage_name_taken`,
//! `category_name_taken`, `settings_key_taken`, `alert_source_ref_taken`,
//! `calendar_sync_connection_name_taken`,
//! `already_seeded`, `version_conflict`, `idempotency_key_reused`,
//! `pre_close_checklist_blocked`, `tasks_unassigned`,
//! `dispatch_move_conflict`,
//! `import_chunk_rejected`,
//! `migration_cursor_mismatch`, `migration_chunk_reused`, `rate_limited`, `auth_rate_limited`,
//! `task_dependencies_unsatisfied`, `undo_reverts_only`,
//! `time_entry_locked`,
//! `run_retry_refused`,
//! `service_unavailable`, `internal_error`, `upload_quarantined`,
//! `upload_refused`.
//!
//! The vocabulary above IS the machine-readable reason-code registry
//! (Wave 13 lane 1): every error this API emits carries one of these
//! codes, and nothing else identifiable crosses the wire — no constraint
//! or index names, no SQL text, no serde/type names, no stack traces.
//! Unknown database constraint violations are logged server-side with
//! full detail and surface to clients as the generic `conflict` code;
//! framework rejections (JSON/query extractors, method-not-allowed) are
//! sanitized into the envelope by [`typed_rejections`].

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    /// The mutation target is not visible under the caller's row-level
    /// scope (RLS filtered it, it was deleted, or it never existed — the
    /// three are deliberately indistinguishable, which is the RLS security
    /// property). Mutations get this instead of `not_found` so clients can
    /// branch: a write that did not land because the target left the
    /// visible scope reconciles by resyncing, not by re-POSTing.
    #[error("mutation target not visible in scope")]
    RlsSparse,
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    /// A body past the malware scanner's stream limit (Wave 62 lane
    /// `p62-attachment-verdicts`). Its own reason because the honest answer is
    /// "too large to scan", not "the scanner is unavailable": the first is the
    /// user's to act on and the second is the operator's.
    #[error("too large to scan: {limit_bytes} bytes is the scanner's limit")]
    UploadTooLarge {
        /// The configured stream cap, echoed so a client can size its retry.
        limit_bytes: u64,
    },
    /// Wave 22 typed 409 for PSP declines (the error-envelope follow-up):
    /// reason stays `payment_declined`, the TYPED decline code rides the
    /// extra field, and the provider's raw decline text is preserved
    /// there too — third-party prose never becomes the primary message.
    #[error("payment declined ({code})")]
    PaymentDeclined {
        /// Typed decline component (`payment_declined` codes).
        code: String,
        /// The provider's raw decline text (kept as extra detail).
        provider_reason: String,
    },
    /// Typed 403 webhook rejections (Wave 22 Stripe receiver): the
    /// signature never verified, the webhook secret is unconfigured, or
    /// the event arrived outside the acceptance window — none of which
    /// is author auth, so it stays its own reason code.
    #[error("stripe webhook rejected: {0}")]
    StripeWebhookRejected(&'static str),
    /// Typed 409 for an If-Match-style version mismatch (instant-first
    /// IF1): the caller's `version` no longer matches the entity's current
    /// monotonic seq, so the write was rejected before anything changed.
    #[error("version conflict: entity moved from version {provided} to {expected}")]
    VersionConflict {
        /// The entity's current version (what the caller must re-read).
        expected: i64,
        /// The stale version the caller presented.
        provided: i64,
    },
    /// Typed 409 when an idempotency key is reused with a different
    /// request payload (or against a different endpoint family). The key
    /// is a replay contract, not a dedupe hint: same key + different body
    /// is a client bug we refuse to guess at.
    #[error("idempotency key reused with a different request")]
    IdempotencyKeyReused,
    /// Typed 409 for a close attempt blocked by the pre-close checklist:
    /// carries the unchecked REQUIRED item texts so clients can render the
    /// exact blockers without a second round-trip.
    #[error("pre-close checklist incomplete")]
    ChecklistBlocked { missing: Vec<String> },
    /// Typed 409 for the Wave 17 workbar close gate (docs/notes-tasks-
    /// workbar.md §4.1): a close-category status change was attempted while
    /// checklist items remain UNASSIGNED (neither checked nor N-A). Carries
    /// one gate item per outstanding row so any closing surface (raw API
    /// callers included) renders the exact decisions needed — Option A
    /// batches them through `/checklist/complete-all`, Option B navigates
    /// the Notes & Tasks panel.
    #[error("tasks remain unassigned")]
    TasksUnassigned {
        items: Vec<crate::routes::tickets::TaskGateItem>,
    },
    /// Typed 409 for a rejected dispatch bulk-move (the private build tracker 2.19): names
    /// the offending move index and the blocking booking so the board can
    /// flag the exact drop target and surface the conflict inline.
    #[error("dispatch move conflicts with existing booking")]
    DispatchMoveConflict {
        /// Index into the request's `moves` array that was rejected.
        index: usize,
        appointment_id: Uuid,
        /// Title of the blocking booking (empty when the rejection is a
        /// non-overlap failure such as an invalid span).
        blocker_title: String,
    },
    /// Typed 409 for a board move whose current status's SEMANTIC category
    /// has no counterpart on the target board (Wave 11 lane 3): carries
    /// every target-board status (id/name/category, deterministic order)
    /// so the client can render the pick list and retry with an explicit
    /// `status_id` — no second round-trip to discover the options.
    #[error("no status on the target board matches the current semantic category")]
    BoardMoveStatusConflict {
        /// Board the ticket was headed to.
        target_board_id: Uuid,
        /// The ticket's current status id (on the source board).
        from_status_id: Uuid,
        /// Semantic category that had no counterpart on the target board.
        from_category: String,
        /// Every status on the target board, sort order first.
        target_statuses: Vec<crate::routes::taxonomy::TargetStatusOption>,
    },
    /// Capability present but disabled by configuration (fail-closed
    /// surfaces such as the internal effect surface without its token).
    #[error("service unavailable: {0}")]
    Unavailable(String),
    /// Typed 422 for an upload the scan gate refused (Wave 52 lanes P6a and
    /// P6b): the bytes were scanned, a signature was found, and NOTHING was
    /// admitted — the object is held under its quarantine key for an
    /// operator. Kept distinct from `Validation` because retrying the same
    /// bytes cannot succeed, and the client should say what was found
    /// rather than re-prompt for a different filename.
    ///
    /// The four facts an upload surface needs to say that in its own words
    /// ride the envelope, so no client has to parse the message: which
    /// object (`sha256`), what it was called (`filename`), what decided
    /// (`scanner`), what it found (`detail`), and that the bytes are held
    /// (`held_for_operator`) — that last one is what stops a UI from
    /// offering "retry the same file" for a refusal that retrying cannot
    /// change.
    #[error("upload rejected by the malware scan")]
    UploadQuarantined {
        /// Lowercase hex sha256 of the held object — its identity.
        sha256: String,
        /// The filename the uploader gave (or the surface minted).
        filename: String,
        /// Adapter that made the call (`clamd`, a test double).
        scanner: String,
        /// The scanner's detail string (usually the signature name).
        detail: Option<String>,
    },
    /// Typed 422 for an upload the scan gate refused on a surface that has
    /// nowhere to hold bytes (Wave 52 lane P6d, the secret vault): the scan
    /// ran, a signature was found, and NOTHING exists afterwards — no
    /// object, no verdict row, no notification, no quarantine copy.
    ///
    /// Kept distinct from [`Self::UploadQuarantined`] because that variant
    /// describes held bytes an operator can release, and this one describes
    /// an upload that is simply gone. The vault is the surface that needs
    /// the distinction: it seals user plaintext before storing, so a
    /// quarantine copy would be the one place the plaintext of a record the
    /// user marked secret sits unsealed. A client branches on `reason`;
    /// neither variant carries a quarantine id, and `held_for_operator`
    /// rides this one as `false` so a UI that read the quarantine answer
    /// does not offer the same "an operator will deal with it" story.
    #[error("upload rejected by the malware scan")]
    UploadRefused {
        /// Lowercase hex sha256 of the bytes that were refused.
        sha256: String,
        /// What the upload was called on this surface (the record's title on
        /// the vault, which has no filename header).
        filename: String,
        /// Adapter that made the call (`clamd`, a test double).
        scanner: String,
        /// The scanner's detail string (usually the signature name).
        detail: Option<String>,
    },
    /// A fail-policy import chunk was rejected transactionally (the private build tracker
    /// D2): nothing from that chunk was written; the full per-row report
    /// rides the 409 so clients can correct and retry without re-upload.
    #[error("import chunk rejected by conflict_policy=fail")]
    ImportChunkRejected {
        report: crate::routes::imports::RunReportResponse,
    },
    /// Typed 409 for a completion attempt whose finish-to-start
    /// dependencies are not all completed yet (the private build tracker Phase 3 projects
    /// line): names the task and the blocking dependency ids so the client
    /// can render exactly what is outstanding.
    #[error("task dependencies not satisfied")]
    TaskDependenciesUnsatisfied {
        /// The task that was denied completion.
        task_id: Uuid,
        /// Dependency ids still not completed.
        blocking: Vec<Uuid>,
    },
    /// Typed 409 for an IF7 whole-record revert against a CLOSED record:
    /// closing is the undo checkpoint, so only explicit, actioned
    /// per-field reverts (a named `fields` list) are accepted — never a
    /// silent rewrite of the whole record past the close.
    #[error("record is closed: only explicit per-field reverts are permitted")]
    UndoRevertsOnly,
    /// Typed 409 for a workflow-run retry the ledger refused (Wave 43
    /// lane 14). Either the run is not in a state an operator may re-run
    /// (still going, still parked, or already succeeded), or re-running it
    /// would repeat a step that declares `idempotent = false` — the
    /// message names every offending node so the operator surface can say
    /// exactly what stopped the retry instead of parsing prose.
    #[error("workflow run retry refused: {message}")]
    RunRetryRefused {
        /// The refusal, with the offending node ids when it is a
        /// double-apply refusal.
        message: String,
    },
    /// Typed 409 for an org-slug collision at registration (Wave 13 lane
    /// 1): carries a suggested available slug so the client can offer
    /// one-click accept instead of an opaque 409. The raw constraint name
    /// (`orgs_slug_key`) never crosses the wire.
    #[error("that workspace address is already taken")]
    SlugTaken {
        /// A currently-available alternative slug, when one could be
        /// derived cheaply at write time.
        suggestion: Option<String>,
    },
    /// Typed 403 refusal at the AI completion edge (ADR 0008, Wave 18
    /// lane 1): the runner/assist client branches on `reason` —
    /// `ai_surface_not_granted`, `ai_model_not_allowed`,
    /// `ai_no_models_configured`, `ai_hard_cap_reached`,
    /// `ai_no_credential`, `ai_surface_disabled`, `ai_surface_internal`.
    /// Degrade-never-5xx: the refusal NEVER masks as a server fault, so
    /// the human workflow degrades gracefully instead of erroring.
    #[error("{message}")]
    AiCallRefused {
        /// Machine-readable refusal reason (clients branch on this).
        reason: &'static str,
        /// Fixed human-readable message.
        message: String,
        /// The surface the call was attempted on.
        surface: String,
    },
    /// Typed 409 for a unique-constraint collision with a KNOWN, typed
    /// meaning (the constraint registry below): `kind` is the machine
    /// reason code, `message` the fixed human copy. Unknown constraints
    /// never reach this variant — they fall through to a sanitized
    /// generic conflict with the detail logged server-side only.
    #[error("{message}")]
    UniqueConflict {
        /// Machine-readable reason code (e.g. `user_email_taken`).
        kind: &'static str,
        /// Fixed human-readable message for the client.
        message: &'static str,
    },
    /// Typed 409 for a cross-org adoption of a version whose manifest nobody
    /// signed (Wave 59 lane `p59-digest-only`, migration 0172).
    ///
    /// `manifest_authenticated = false` is the honest verdict for a GitHub pull,
    /// a local archive, a legacy digest-only registry statement and a declarative
    /// publish. What it must never be is a licence to hand that manifest to
    /// another tenant as publisher-signed — the catalog is server-wide, so an
    /// install by slug reaches any publisher's entry. The org that staged the row
    /// is the one that read the consent screen and may still adopt it, so the
    /// refusal names the entry and the version, not the caller's permission.
    #[error("this version's manifest is not signed by a publisher")]
    ManifestUnauthenticated {
        /// The catalog entry that was the target of the adoption.
        catalog_id: Uuid,
        /// Its slug, as the client asked for it.
        slug: String,
        /// The pinned version whose manifest carries the weaker claim.
        version: String,
    },
    /// Typed 401 for a federation (bundle) login refusal (Wave 13 lane
    /// 7): auth failures carry a registry reason code, not prose to
    /// parse. `reason` is the machine code clients branch on;
    /// `message` is fixed human copy. Verification internals (why a
    /// signature/issuer/audience check failed) are logged server-side
    /// and never cross the wire — all invalid assertions collapse to
    /// the single `edge_assertion_invalid` code so attackers cannot
    /// fingerprint which check tripped.
    #[error("{message}")]
    AuthDenied {
        /// Machine-readable reason code (e.g. `edge_assertion_invalid`).
        reason: &'static str,
        /// Fixed human-readable message for the client.
        message: &'static str,
    },
    /// Typed 428 (Precondition Required) for a destructive MFA change that
    /// arrived without a fresh step-up (Wave 55, `p55-mfa-step-up`).
    ///
    /// `reason` is the machine code a client branches on: `step_up_required`
    /// (this sign-in lineage has never proved a factor), `step_up_stale` (it
    /// did, and the window has passed), `step_up_factor_not_allowed` (the
    /// factor it proved cannot authorize THIS change — a live TOTP code
    /// cannot replace the TOTP secret it belongs to, and a passkey cannot
    /// remove itself), or `mfa_reenrollment_required` (an administrator reset
    /// the factors, so the only permitted move is enrolment). `endpoint`
    /// names the call that satisfies it and `ttl_secs` the window, so no
    /// client has to parse the message to find out what to do.
    #[error("{message}")]
    MfaStepUpRequired {
        /// Machine-readable reason code.
        reason: &'static str,
        /// Fixed human-readable copy naming the next step.
        message: String,
        /// The endpoint that establishes a fresh step-up.
        endpoint: &'static str,
        /// Seconds a fresh step-up stays usable.
        ttl_secs: i64,
    },
    /// Typed 409 for a dev-seed attempt against an already-seeded demo
    /// org (Wave 13 lane 1): the reason code (not the prose) is what the
    /// ops page branches on for the "force reseed" second click.
    #[error("demo org already seeded")]
    AlreadySeeded,
    /// Typed 409 for a migration connector chunk pushed out of position
    /// (the private build tracker D3): the checkpoint cursor expects a different index. The
    /// tool resumes from `expected` (GET the session view) — nothing was
    /// written.
    #[error("migration chunk cursor mismatch: expected chunk {expected}, got {provided}")]
    MigrationCursorMismatch {
        /// The chunk index the checkpoint actually expects next.
        expected: i64,
        /// The index the caller presented.
        provided: i64,
    },
    /// Typed 409 when an already-committed migration chunk is re-sent with
    /// a DIFFERENT payload (the private build tracker D3). Same index + same payload is a
    /// free replay of the stored report; drifting content under one index
    /// is a client bug we refuse to guess at.
    #[error("migration chunk already committed with different content")]
    MigrationChunkReused {
        /// The offending chunk index.
        chunk_index: i64,
    },
    /// Typed 429: the per-key rate limit for the migration connector
    /// surface is exhausted for this window. Carries the seconds until the
    /// window resets (also emitted as the Retry-After header).
    #[error("rate limit exceeded; retry after {retry_after_secs}s")]
    RateLimited {
        /// Seconds until the current fixed window resets.
        retry_after_secs: i64,
    },
    /// Typed 429 for the auth-throttle family (Wave 19 lane 5): a
    /// durable per-IP + per-actor lockout is engaged on this auth
    /// surface. Same envelope shape as [`ApiError::RateLimited`] with the
    /// distinct `auth_rate_limited` reason so clients can branch without
    /// conflating connector budgets with credential gates. LOGIN is the
    /// deliberate exception — there the 401 uniform-shape answer is kept
    /// (threat model §A1: no lock-state enumeration).
    #[error("auth rate limit engaged; retry after {retry_after_secs}s")]
    AuthThrottled {
        /// Seconds until the current exponential-backoff lock expires.
        retry_after_secs: i64,
    },
    /// Typed 409 for a time-entry mutation the invoicing lock refuses
    /// (Wave 14 lane 1): once an entry is consumed by an invoice it is
    /// frozen — no submit, decide, reopen, or content edit. Carries the
    /// entry id so the client can flag the exact blocked row.
    #[error("time entry is locked by invoicing")]
    TimeEntryLocked {
        /// The locked entry.
        entry_id: Uuid,
    },
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

/// Maps a Postgres unique-violation constraint name onto its typed
/// (reason code, human message) pair. This table is the ONLY place a
/// constraint name is interpreted, and the name itself never reaches a
/// response: unknown entries fall through to the sanitized generic
/// conflict so a new constraint degrades to `conflict`, never to a leak.
fn map_unique_constraint(constraint: &str) -> Option<(&'static str, &'static str)> {
    match constraint {
        "orgs_slug_key" => Some(("org_slug_taken", "that workspace address is already taken")),
        // Wave 47 lane A8: a custom domain is unique across orgs — a second
        // org claiming a hostname another already holds gets a typed 409
        // rather than the generic conflict.
        "orgs_custom_domain_key" => Some((
            "custom_domain_taken",
            "that custom domain is already claimed by another workspace",
        )),
        "users_org_id_email_key" => Some((
            "user_email_taken",
            "an account with this email already exists",
        )),
        "roles_org_id_name_key" => {
            Some(("role_name_taken", "a role with this name already exists"))
        }
        "uq_catalog_items_sku" => Some(("sku_taken", "an item with this SKU already exists")),
        "quotes_number_key" => Some((
            "quote_number_taken",
            "a quote with this number already exists",
        )),
        "pipeline_stages_name_key" => {
            Some(("stage_name_taken", "a stage with this name already exists"))
        }
        "uq_ticket_categories_root_name" | "uq_ticket_categories_child_name" => Some((
            "category_name_taken",
            "a category with this name already exists",
        )),
        "settings_key_unique" => Some((
            "settings_key_taken",
            "a setting with this key already exists",
        )),
        // Wave 43 track 2 lane 12 (migration 0127): an offline-replayed
        // sign-off create lands on its own idempotency key. Typed so the
        // client's write queue can recognise its replay as already applied.
        "sign_offs_org_id_idempotency_key_key" => Some((
            "sign_off_already_captured",
            "this sign-off was already captured; the offline queue replay is a no-op",
        )),
        // Same lane: a consent version collision (two admins adding the
        // next version at once) rather than two revisions racing to N.
        "sign_off_consents_org_id_version_key" => Some((
            "consent_version_taken",
            "that consent version already exists; reload and add the next one",
        )),
        // Wave 22 Marketplace v2 (migration 0097): both new unique
        // indexes get typed reasons per the migration-conventions rule.
        "uq_plugin_publisher_identity" => Some((
            "publisher_already_verified",
            "this publisher identity is already in the org's registry",
        )),
        "uq_plugin_rating_per_user" => Some((
            "rating_targeted",
            "you already rated this release; re-rate with PUT instead",
        )),
        "uq_voip_call_id" => Some((
            "voip_call_replayed",
            "this call id was already recorded on this org",
        )),
        "idx_alert_sources_ref" => Some((
            "alert_source_ref_taken",
            "an alert source with this ref already exists",
        )),
        // Wave 13 lane 7 (identity federation). Both pre-auth names are
        // claimed deployment-wide, not per org (Wave 54 lane S3e, migration
        // 0159): the login and exchange paths resolve a bare name before a
        // tenant is known, so a second workspace offering the same name
        // would take over the first one's resolution. The per-org
        // constraints stay mapped because a writer that bypasses the
        // upsert's `ON CONFLICT (org_id, name)` still collides there, and
        // both collisions are the same UX problem — pick another name.
        "oidc_providers_global_name_key" | "oidc_providers_org_id_name_key" => Some((
            "oidc_provider_name_taken",
            "that single sign-on name is already claimed by a workspace; choose a different one",
        )),
        "edge_assertion_configs_global_name_key" | "edge_assertion_configs_org_id_name_key" => {
            Some((
                "edge_source_name_taken",
                "that edge source name is already claimed by a workspace; choose a different one",
            ))
        }
        "federation_links_source_kind_source_id_subject_key" => Some((
            "federation_link_taken",
            "this federated identity is already linked to an account",
        )),
        "kb_embed_providers_org_id_slug_key" => Some((
            "kb_embed_provider_slug_taken",
            "an embed provider with this slug already exists",
        )),
        "kb_embeds_org_id_article_id_url_key" => Some((
            "kb_embed_url_taken",
            "this embed url is already attached to the article",
        )),
        // Wave 13 lane 5 (template registry).
        "templates_org_id_kind_slug_key" => Some((
            "template_slug_taken",
            "a template with this slug already exists for this kind",
        )),
        // Wave 14 lane 1 (time approvals): a concurrent decision raced the
        // per-entry seq assignment — the caller replays the decision.
        "uq_time_entry_approval_seq" => Some((
            "time_entry_approval_conflict",
            "this approval decision conflicts with one recorded at the same moment",
        )),
        // Wave 14 lane 2 (two-way calendar sync).
        "calendar_sync_connections_org_id_user_id_name_key" => Some((
            "calendar_sync_connection_name_taken",
            "a calendar-sync connection with this name already exists for this user",
        )),
        // Wave 14 lane 3 (coaching scorecards): a concurrent settings
        // upsert raced — the caller re-reads and retries the toggle.
        "uq_scorecard_settings_user" => Some((
            "scorecard_settings_conflict",
            "your scorecard settings were just updated elsewhere; reload and retry",
        )),
        // Wave 14 lane 5 (plugin runtime v2 + marketplace registry).
        "plugin_catalog_slug_key" => Some((
            "plugin_slug_taken",
            "a catalog entry with this slug already exists",
        )),
        "plugin_versions_catalog_id_version_key" => Some((
            "plugin_version_taken",
            "this version already exists for the catalog entry",
        )),
        "plugin_installs_org_id_catalog_id_key" => Some((
            "plugin_already_installed",
            "this plugin is already installed for this org",
        )),
        // Wave 19 lane 2 (signing-key rotation, migration 0095): the key_id
        // is a permanent trust identity — a compromised id can NEVER be
        // re-registered after its revoke.
        "plugin_signing_keys_key_id_key" => Some((
            "signing_key_id_taken",
            "a signing key with this key_id is already registered \
             (a revoked key_id stays occupied by design)",
        )),
        // Wave 16 lane 3 (native vault, ADR 0006, migration 0078).
        "secret_folders_scope_key" => Some((
            "folder_slug_taken",
            "a folder with this slug already exists here",
        )),
        "secret_records_scope_key" => Some((
            "record_title_taken",
            "a record with this title already exists in this folder",
        )),
        "secret_fields_record_key" => {
            Some(("field_key_taken", "this field already exists on the record"))
        }
        "secret_grants_role_folder_key" => Some((
            "secret_grant_taken",
            "a grant for this role and folder already exists",
        )),
        // Wave 16 lane 7 (ADR 0009 distribution, migration 0079): one live
        // PENDING staged row per (org, manifest name+version) — re-stage
        // refuses until the older row is reviewed or withdrawn.
        "plugin_staged_items_pending_org_artifact_key" => Some((
            "staged_import_already_pending",
            "this artifact (name + version) is already staged and awaiting review",
        )),
        // 0040 skills line: the append-only (org, slug, version) ladder
        // only ever refuses when two concurrent publishes grabbed the same
        // max+1 slot (the routes compute version from MAX(version)).
        "agent_skills_org_id_slug_version_key" => Some((
            "agent_skill_version_taken",
            "this version of the skill was just published; republish to append fresh",
        )),
        // Wave 17 lane 13 (backend gaps): the pinned-results identity
        // only ever refuses a genuine concurrent upsert race — the
        // handler answers the stored pin on the idempotent path.
        "uq_search_pins_result" => Some((
            "search_pin_conflict",
            "this result was just pinned elsewhere; reload your pins and retry",
        )),
        // Wave 18 lane 1 (ADR 0008 ai_policy_ledger, migration 0081): the
        // matrix identity constraints. Catalog conflicts are a real admin
        // mistake (dup pair); the rest only refuse concurrent upsert
        // races (handlers answer idempotently otherwise).
        "uq_ai_model_catalog_pair" => Some((
            "ai_model_catalog_pair_taken",
            "this (provider, model) pair is already in the catalog",
        )),
        "uq_ai_surface_grants_grantee" => Some((
            "ai_surface_grant_taken",
            "this surface grant already exists; edit the existing row instead",
        )),
        "uq_ai_model_credentials_owner" => Some((
            "ai_model_credential_taken",
            "a credential of this kind for this provider already exists at this scope",
        )),
        "uq_ai_caps_scope" => Some((
            "ai_caps_scope_taken",
            "a cap for this scope and period already exists",
        )),
        "uq_ai_model_picks_surface" => Some((
            "ai_model_pick_taken",
            "a pick for this surface already exists at this scope",
        )),
        "uq_ai_surfaces_key" => Some((
            "ai_surface_key_taken",
            "this surface key is already registered",
        )),
        // Wave 18 lane 2b: per-edge ledger idempotence. The write path
        // dedupes via ON CONFLICT DO NOTHING, so this surfaces only when
        // a caller bypasses record_usage — still typed, never generic.
        "uq_ai_usage_ledger_edge_request" => Some((
            "ai_request_duplicate",
            "this request was already recorded on this surface",
        )),
        _ => None,
    }
}

/// Decides how a unique violation surfaces: known constraints produce a
/// fully typed error, unknown ones emit the sanitized generic conflict
/// (the name is logged server-side only).
fn unique_violation(e: &sqlx::Error) -> Option<ApiError> {
    let db = e.as_database_error()?;
    let constraint = db.constraint()?;
    Some(match map_unique_constraint(constraint) {
        Some((kind, message)) => {
            tracing::warn!(
                constraint,
                reason = kind,
                "unique violation mapped to typed reason"
            );
            ApiError::UniqueConflict { kind, message }
        }
        None => {
            tracing::error!(
                constraint,
                "unique violation on unmapped constraint; sending sanitized generic conflict"
            );
            ApiError::Conflict(
                "this value conflicts with an existing record; adjust it and retry".into(),
            )
        }
    })
}

impl From<core_db::DbError> for ApiError {
    fn from(e: core_db::DbError) -> Self {
        match &e {
            core_db::DbError::NotFound => ApiError::NotFound,
            core_db::DbError::Conflict(msg) => ApiError::Conflict(msg.clone()),
            core_db::DbError::Sql(sqlx::Error::RowNotFound) => ApiError::NotFound,
            core_db::DbError::Sql(err @ sqlx::Error::Database(_)) => unique_violation(err)
                .unwrap_or_else(|| {
                    tracing::error!(error = %e, "database error");
                    ApiError::Internal(anyhow::anyhow!(e.to_string()))
                }),
            _ => {
                tracing::error!(error = %e, "database error");
                ApiError::Internal(anyhow::anyhow!(e.to_string()))
            }
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::from(core_db::DbError::Sql(e))
    }
}

impl From<core_domain::DomainError> for ApiError {
    fn from(e: core_domain::DomainError) -> Self {
        ApiError::Validation(e.to_string())
    }
}

/// The workflow node registry's typed refusals (Wave 43 lane 13). They
/// ride the same validation posture as every other domain refusal, so a
/// caller sees a 422 naming the node rather than a 500.
impl From<core_domain::node_registry::NodeRegistryError> for ApiError {
    fn from(e: core_domain::node_registry::NodeRegistryError) -> Self {
        Self::from(core_domain::DomainError::from(e))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, reason, message, extra) = match &self {
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "not found".to_string(),
                None,
            ),
            ApiError::RlsSparse => (
                StatusCode::NOT_FOUND,
                "rls_sparse",
                "mutation target not visible in scope".to_string(),
                None,
            ),
            ApiError::Unauthorized(m) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", m.clone(), None)
            }
            ApiError::AuthDenied { reason, message } => {
                (StatusCode::UNAUTHORIZED, *reason, message.to_string(), None)
            }
            ApiError::MfaStepUpRequired {
                reason,
                message,
                endpoint,
                ttl_secs,
            } => (
                StatusCode::PRECONDITION_REQUIRED,
                *reason,
                message.clone(),
                Some(json!({ "endpoint": endpoint, "ttl_secs": ttl_secs })),
            ),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone(), None),
            ApiError::Validation(m) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                m.clone(),
                None,
            ),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone(), None),
            ApiError::PaymentDeclined {
                code,
                provider_reason,
            } => (
                StatusCode::CONFLICT,
                "payment_declined",
                format!(
                    "the payment was declined ({code}); the provider's reason is kept in the extra field"
                ),
                Some(json!({ "code": code, "provider_reason": provider_reason })),
            ),
            ApiError::StripeWebhookRejected(m) => (
                StatusCode::FORBIDDEN,
                "stripe_webhook_rejected",
                m.to_string(),
                None,
            ),
            ApiError::VersionConflict { expected, provided } => (
                StatusCode::CONFLICT,
                "version_conflict",
                format!("version conflict: entity moved from version {provided} to {expected}"),
                Some(json!({ "expected": expected, "provided": provided })),
            ),
            ApiError::IdempotencyKeyReused => (
                StatusCode::CONFLICT,
                "idempotency_key_reused",
                "idempotency key reused with a different request".to_string(),
                None,
            ),
            ApiError::ChecklistBlocked { missing } => (
                StatusCode::CONFLICT,
                "pre_close_checklist_blocked",
                "pre-close checklist incomplete".to_string(),
                Some(json!({ "missing": missing })),
            ),
            ApiError::TasksUnassigned { items } => (
                StatusCode::CONFLICT,
                "tasks_unassigned",
                "tasks remain unassigned — every task needs a completed / N-A status before this ticket can close".to_string(),
                Some(json!({ "items": items })),
            ),
            ApiError::DispatchMoveConflict {
                index,
                appointment_id,
                blocker_title,
            } => (
                StatusCode::CONFLICT,
                "dispatch_move_conflict",
                "dispatch move conflicts with existing booking".to_string(),
                Some(json!({
                    "index": index,
                    "appointment_id": appointment_id,
                    "blocker_title": blocker_title,
                })),
            ),
            ApiError::BoardMoveStatusConflict {
                target_board_id,
                from_status_id,
                from_category,
                target_statuses,
            } => (
                StatusCode::CONFLICT,
                "board_move_status_conflict",
                "no status on the target board matches the current semantic category".to_string(),
                Some(json!({
                    "target_board_id": target_board_id,
                    "from_status_id": from_status_id,
                    "from_category": from_category,
                    "target_statuses": target_statuses,
                })),
            ),
            ApiError::ImportChunkRejected { report } => (
                StatusCode::CONFLICT,
                "import_chunk_rejected",
                "import chunk rejected by conflict_policy=fail".to_string(),
                Some(json!({ "report": report })),
            ),
            ApiError::TaskDependenciesUnsatisfied { task_id, blocking } => (
                StatusCode::CONFLICT,
                "task_dependencies_unsatisfied",
                "task dependencies not satisfied".to_string(),
                Some(json!({ "task_id": task_id, "blocking": blocking })),
            ),
            ApiError::UniqueConflict { kind, message } => {
                (StatusCode::CONFLICT, *kind, message.to_string(), None)
            }
            ApiError::AiCallRefused {
                reason,
                message,
                surface,
            } => (
                StatusCode::FORBIDDEN,
                *reason,
                message.clone(),
                Some(json!({ "surface": surface })),
            ),
            ApiError::SlugTaken { suggestion } => (
                StatusCode::CONFLICT,
                "org_slug_taken",
                "that workspace address is already taken".to_string(),
                Some(json!({ "suggestion": suggestion })),
            ),
            ApiError::AlreadySeeded => (
                StatusCode::CONFLICT,
                "already_seeded",
                "demo org already seeded; POST /api/dev/seed?force=1 to wipe and reseed"
                    .to_string(),
                None,
            ),
            ApiError::UndoRevertsOnly => (
                StatusCode::CONFLICT,
                "undo_reverts_only",
                "record is closed: only explicit per-field reverts are permitted".to_string(),
                None,
            ),
            // Wave 43 lane 14 (run ledger): the retry decision is typed so
            // a client can render exactly what stopped it. Two refusals
            // reach this arm — the run is not in a retryable state, or
            // re-running would repeat a step that declares
            // `idempotent = false` — and the message names the nodes, so
            // the client never has to parse prose to work out why.
            ApiError::RunRetryRefused { message } => (
                StatusCode::CONFLICT,
                "run_retry_refused",
                message.clone(),
                None,
            ),
            // Wave 59 (digest-only): the manifest signature verdict is the
            // reason a distribution refusal carries, so a client can tell
            // "nobody signed this" from "you lack the grant" without parsing
            // prose.
            ApiError::ManifestUnauthenticated {
                catalog_id,
                slug,
                version,
            } => (
                StatusCode::CONFLICT,
                "manifest_unauthenticated",
                "this version's manifest is not signed by a publisher; only the \
                 organization that staged it may adopt it"
                    .to_string(),
                Some(json!({
                    "catalog_id": catalog_id,
                    "slug": slug,
                    "version": version,
                })),
            ),
            ApiError::MigrationCursorMismatch { expected, provided } => (
                StatusCode::CONFLICT,
                "migration_cursor_mismatch",
                format!(
                    "migration chunk cursor mismatch: expected chunk {expected}, got {provided}"
                ),
                Some(json!({ "expected": expected, "provided": provided })),
            ),
            ApiError::MigrationChunkReused { chunk_index } => (
                StatusCode::CONFLICT,
                "migration_chunk_reused",
                "migration chunk already committed with different content".to_string(),
                Some(json!({ "chunk_index": chunk_index })),
            ),
            ApiError::RateLimited { retry_after_secs } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("rate limit exceeded; retry after {retry_after_secs}s"),
                Some(json!({ "retry_after_secs": retry_after_secs })),
            ),
            ApiError::AuthThrottled { retry_after_secs } => (
                StatusCode::TOO_MANY_REQUESTS,
                "auth_rate_limited",
                format!("auth rate limit engaged; retry after {retry_after_secs}s"),
                Some(json!({ "retry_after_secs": retry_after_secs })),
            ),
            ApiError::Unavailable(m) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                m.clone(),
                None,
            ),
            ApiError::UploadQuarantined {
                sha256,
                filename,
                scanner,
                detail,
            } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "upload_quarantined",
                "the file was rejected by the malware scanner and is held for an operator; \
                 nothing was attached"
                    .to_string(),
                Some(json!({
                    "sha256": sha256,
                    "filename": filename,
                    "scanner": scanner,
                    "detail": detail,
                    // The bytes are held, not discarded: an operator has to
                    // release or delete them. A UI that reads this knows a
                    // retry of the same file is pointless.
                    "held_for_operator": true,
                })),
            ),
            ApiError::UploadTooLarge { limit_bytes } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "upload_too_large_to_scan",
                format!(
                    "the file is too large to scan: this deployment accepts up to \
                     {limit_bytes} bytes for a scanned upload; nothing was stored"
                ),
                Some(json!({ "limit_bytes": limit_bytes, "held_for_operator": false })),
            ),
            ApiError::UploadRefused {
                sha256,
                filename,
                scanner,
                detail,
            } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "upload_refused",
                "the file was rejected by the malware scanner; nothing was stored"
                    .to_string(),
                Some(json!({
                    "sha256": sha256,
                    "filename": filename,
                    "scanner": scanner,
                    "detail": detail,
                    // The answer to the same question the quarantine refusal
                    // answers with `true`: nothing is being held, so there is
                    // no operator action and no release to wait for.
                    "held_for_operator": false,
                })),
            ),
            ApiError::TimeEntryLocked { entry_id } => (
                StatusCode::CONFLICT,
                "time_entry_locked",
                "time entry is locked by invoicing; nothing may change it".to_string(),
                Some(json!({ "entry_id": entry_id })),
            ),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal error".to_string(),
                None,
            ),
        };
        let mut error = json!({ "code": status.as_u16(), "reason": reason, "message": message });
        if let Some(extra) = extra {
            if let (Some(base), Some(ext)) = (error.as_object_mut(), extra.as_object()) {
                for (k, v) in ext {
                    base.insert(k.clone(), v.clone());
                }
            }
        }
        let mut response = (status, Json(json!({ "error": error }))).into_response();
        if let ApiError::RateLimited { retry_after_secs }
        | ApiError::AuthThrottled { retry_after_secs } = &self
        {
            if let Ok(value) = retry_after_secs.to_string().parse() {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Middleware: rewrites framework rejection bodies into the IF3 envelope.
///
/// axum's extractors and method router reject with `text/plain` bodies
/// carrying raw serde diagnostics (type names, serde paths, line/column)
/// — an internal-leak path across all 200+ handler argument sites (Wave
/// 13 lane 1). This layer, mounted over the whole router in
/// `build_router`, maps every such response onto the typed envelope and
/// keeps the raw text server-side. A deliberate no-op for everything
/// else: JSON envelope responses pass through untouched, and the
/// empty-body unmatched-404 sentinel (api_versioning/dev_surface
/// existence checks) has no content-type and also passes through.
pub async fn typed_rejections(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header::CONTENT_TYPE;

    let res = next.run(req).await;
    let status = res.status();
    if !status.is_client_error() && !status.is_server_error() {
        return res;
    }
    let is_text_rejection = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/plain"));
    if !is_text_rejection {
        return res;
    }

    let (_parts, body) = res.into_parts();
    let raw = axum::body::to_bytes(body, 64 * 1024)
        .await
        .unwrap_or_default();
    // Server-side only: full framework/serde diagnostics for debugging.
    tracing::debug!(
        status = status.as_u16(),
        raw = %String::from_utf8_lossy(&raw),
        "text/plain rejection sanitized into the IF3 envelope"
    );
    let (reason, message): (&str, &str) = match status {
        StatusCode::NOT_FOUND => ("not_found", "not found"),
        StatusCode::UNAUTHORIZED => ("unauthorized", "unauthorized"),
        StatusCode::FORBIDDEN => ("forbidden", "forbidden"),
        StatusCode::BAD_REQUEST => (
            "validation_failed",
            "request is malformed or has the wrong shape",
        ),
        _ => (
            "validation_failed",
            "request is malformed or has the wrong shape",
        ),
    };
    (
        status,
        Json(json!({
            "error": { "code": status.as_u16(), "reason": reason, "message": message }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use serde_json::Value;

    async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    /// The machine-readable reason-code registry: every variant must map
    /// to a distinct reason in the documented vocabulary. A new variant
    /// that forgets its code fails here, not in production. (The
    /// `import_chunk_rejected` variant is exercised by imports_flow tests.)
    #[tokio::test(flavor = "multi_thread")]
    async fn every_variant_carries_a_registry_reason() {
        let cases: Vec<(ApiError, &str)> = vec![
            (ApiError::NotFound, "not_found"),
            (ApiError::RlsSparse, "rls_sparse"),
            (ApiError::Unauthorized("x".into()), "unauthorized"),
            (ApiError::Forbidden("x".into()), "forbidden"),
            (ApiError::Validation("x".into()), "validation_failed"),
            (ApiError::Conflict("x".into()), "conflict"),
            (
                ApiError::VersionConflict {
                    expected: 2,
                    provided: 1,
                },
                "version_conflict",
            ),
            (ApiError::IdempotencyKeyReused, "idempotency_key_reused"),
            (
                ApiError::ChecklistBlocked { missing: vec![] },
                "pre_close_checklist_blocked",
            ),
            (
                ApiError::DispatchMoveConflict {
                    index: 0,
                    appointment_id: Uuid::now_v7(),
                    blocker_title: String::new(),
                },
                "dispatch_move_conflict",
            ),
            (
                ApiError::BoardMoveStatusConflict {
                    target_board_id: Uuid::now_v7(),
                    from_status_id: Uuid::now_v7(),
                    from_category: "work".into(),
                    target_statuses: vec![],
                },
                "board_move_status_conflict",
            ),
            (
                ApiError::TaskDependenciesUnsatisfied {
                    task_id: Uuid::now_v7(),
                    blocking: vec![],
                },
                "task_dependencies_unsatisfied",
            ),
            (ApiError::UndoRevertsOnly, "undo_reverts_only"),
            (
                ApiError::RunRetryRefused {
                    message: "email.send".into(),
                },
                "run_retry_refused",
            ),
            (ApiError::SlugTaken { suggestion: None }, "org_slug_taken"),
            (
                ApiError::ManifestUnauthenticated {
                    catalog_id: Uuid::now_v7(),
                    slug: "ticket-summarizer".into(),
                    version: "1.0.0".into(),
                },
                "manifest_unauthenticated",
            ),
            (
                ApiError::UniqueConflict {
                    kind: "user_email_taken",
                    message: "x",
                },
                "user_email_taken",
            ),
            (ApiError::AlreadySeeded, "already_seeded"),
            (
                ApiError::MigrationCursorMismatch {
                    expected: 1,
                    provided: 2,
                },
                "migration_cursor_mismatch",
            ),
            (
                ApiError::MigrationChunkReused { chunk_index: 1 },
                "migration_chunk_reused",
            ),
            (
                ApiError::RateLimited {
                    retry_after_secs: 1,
                },
                "rate_limited",
            ),
            (ApiError::Unavailable("x".into()), "service_unavailable"),
            (
                ApiError::UploadQuarantined {
                    sha256: "0".repeat(64),
                    filename: "invoice.pdf".into(),
                    scanner: "clamd".into(),
                    detail: Some("Eicar-Test-Signature".into()),
                },
                "upload_quarantined",
            ),
            (
                ApiError::UploadTooLarge { limit_bytes: 1024 },
                "upload_too_large_to_scan",
            ),
            (
                ApiError::UploadRefused {
                    sha256: "0".repeat(64),
                    filename: "Contract.pdf".into(),
                    scanner: "clamd".into(),
                    detail: Some("Eicar-Test-Signature".into()),
                },
                "upload_refused",
            ),
            (
                ApiError::Internal(anyhow::anyhow!("boom")),
                "internal_error",
            ),
        ];
        let mut seen = std::collections::HashSet::new();
        for (err, expected) in cases {
            let bytes = body_bytes(err.into_response()).await;
            let v: Value = serde_json::from_slice(&bytes).expect("envelope is JSON");
            let reason = v["error"]["reason"].as_str().expect("reason key");
            assert_eq!(reason, expected, "wrong reason for {expected}");
            assert!(
                seen.insert(reason.to_string()),
                "duplicate reason code: {reason}"
            );
            assert!(v["error"]["message"].is_string(), "message key missing");
        }
    }

    /// A body past the scanner's cap is a 422 that names the limit, not a 503
    /// that blames the scanner. The distinction is the whole point of the typed
    /// variant: an operator reading `service_unavailable` would retry the upload
    /// forever, while `upload_too_large_to_scan` says the file will never fit.
    /// Nothing was held, so the payload must not claim an operator has it.
    #[tokio::test(flavor = "multi_thread")]
    async fn too_large_is_422_and_says_the_limit_and_that_nothing_was_held() {
        let response = ApiError::UploadTooLarge { limit_bytes: 1024 }.into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let v: Value =
            serde_json::from_slice(&body_bytes(response).await).expect("envelope is JSON");
        assert_eq!(v["error"]["reason"], "upload_too_large_to_scan");
        assert_eq!(v["error"]["limit_bytes"], 1024);
        assert_eq!(
            v["error"]["held_for_operator"], false,
            "a file that never fit was not held, and the error must not say it was"
        );
        let message = v["error"]["message"].as_str().expect("message key");
        assert!(
            message.contains("1024"),
            "the message should name the limit a client has to shrink under: {message}"
        );
    }

    /// The constraint registry: every known constraint maps to a distinct
    /// typed reason code, and none of the codes collide with each other
    /// or with the generic `conflict`.
    #[test]
    fn constraint_registry_is_typed_and_disjoint() {
        let known = [
            "orgs_slug_key",
            "users_org_id_email_key",
            "roles_org_id_name_key",
            "uq_catalog_items_sku",
            "quotes_number_key",
            "pipeline_stages_name_key",
            "uq_ticket_categories_root_name",
            "uq_ticket_categories_child_name",
            "settings_key_unique",
            "idx_alert_sources_ref",
            "calendar_sync_connections_org_id_user_id_name_key",
            "uq_scorecard_settings_user",
            "plugin_catalog_slug_key",
            "plugin_versions_catalog_id_version_key",
            "plugin_installs_org_id_catalog_id_key",
            // Wave 18 lane 1 (ADR 0008 migration 0081): the AI-matrix
            // identity constraints surface typed, not generic.
            "uq_ai_model_catalog_pair",
            "uq_ai_surface_grants_grantee",
            "uq_ai_model_credentials_owner",
            "uq_ai_caps_scope",
            "uq_ai_model_picks_surface",
            "uq_ai_surfaces_key",
            "uq_ai_usage_ledger_edge_request",
            // Wave 19 lane 2 (signing-key rotation, migration 0095).
            "plugin_signing_keys_key_id_key",
            // Wave 54 lane S3e (migration 0159): the pre-auth SSO names are
            // claimed deployment-wide, and the per-org constraints they
            // replace stay mapped to the same codes.
            "oidc_providers_global_name_key",
            "oidc_providers_org_id_name_key",
            "edge_assertion_configs_global_name_key",
            "edge_assertion_configs_org_id_name_key",
        ];
        let mut seen = std::collections::HashSet::new();
        for constraint in known {
            let (kind, message) = map_unique_constraint(constraint)
                .unwrap_or_else(|| panic!("{constraint} unmapped"));
            assert_ne!(kind, "conflict");
            assert!(!message.is_empty());
            seen.insert(kind);
        }
        // Three pairs deliberately SHARE one reason code each: the two
        // taxonomy constraints (`category_name_taken` — root-name and
        // child-name collisions are the same UX problem), and the two SSO
        // name pairs (`oidc_provider_name_taken`, `edge_source_name_taken` —
        // a per-org and a deployment-wide collision of the same name are the
        // same answer to the admin). So 27 constraints map to 24 distinct
        // codes.
        assert_eq!(seen.len(), 24, "unexpected reason-code fan-out: {seen:?}");
        assert_eq!(
            map_unique_constraint("uq_ticket_categories_root_name").map(|(k, _)| k),
            map_unique_constraint("uq_ticket_categories_child_name").map(|(k, _)| k)
        );
        assert_eq!(
            map_unique_constraint("oidc_providers_global_name_key").map(|(k, _)| k),
            map_unique_constraint("oidc_providers_org_id_name_key").map(|(k, _)| k)
        );
        assert_eq!(
            map_unique_constraint("edge_assertion_configs_global_name_key").map(|(k, _)| k),
            map_unique_constraint("edge_assertion_configs_org_id_name_key").map(|(k, _)| k)
        );
        // Unknown constraints degrade, never map.
        assert_eq!(map_unique_constraint("some_future_constraint_key"), None);
        assert_eq!(map_unique_constraint(""), None);
    }

    /// The slug-taken envelope carries the `suggestion` extra key.
    #[tokio::test(flavor = "multi_thread")]
    async fn slug_taken_carries_suggestion() {
        let bytes = body_bytes(
            ApiError::SlugTaken {
                suggestion: Some("acme-2".into()),
            }
            .into_response(),
        )
        .await;
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["reason"], "org_slug_taken");
        assert_eq!(v["error"]["suggestion"], "acme-2");
    }

    /// A text/plain framework rejection is rewritten into the envelope
    /// with the raw serde/framework text kept server-side only.
    #[tokio::test(flavor = "multi_thread")]
    async fn typed_rejections_sanitize_text_plain_bodies() {
        use tower::ServiceExt;
        let app: axum::Router = axum::Router::new()
            .route(
                "/probe",
                axum::routing::post(|axum::Json(_req): axum::Json<serde_json::Value>| async {
                    "ok"
                }),
            )
            .layer(axum::middleware::from_fn(typed_rejections));
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/probe")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{\"not\":\"closed"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum 0.8: malformed JSON (JsonSyntaxError) is 400 BAD_REQUEST;
        // a well-formed body with the wrong shape (JsonDataError) is 422.
        assert_eq!(res.status(), axum::http::StatusCode::BAD_REQUEST);
        let bytes = body_bytes(res).await;
        let v: Value = serde_json::from_slice(&bytes).expect("rejection is envelope JSON");
        assert_eq!(v["error"]["reason"], "validation_failed");
        assert_eq!(v["error"]["code"], 400);
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            !text.contains("serde") && !text.contains("line ") && !text.contains("expected"),
            "serde diagnostics leaked into the response: {text}"
        );
    }

    /// Method-not-allowed rejections are EMPTY in axum (status-only, no
    /// body, no content-type): the middleware leaves them untouched —
    /// nothing to sanitize, and the empty-body sentinel stays intact.
    #[tokio::test(flavor = "multi_thread")]
    async fn method_not_allowed_stays_empty() {
        use tower::ServiceExt;
        let app: axum::Router = axum::Router::new()
            .route("/probe", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(typed_rejections));
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/probe")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
        assert!(
            res.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .is_none(),
            "405 must stay content-type-free (no leak, sentinel intact)"
        );
    }
}
