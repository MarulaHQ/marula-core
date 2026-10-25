//! Support stack for the two-weeks zero-loss harness: deterministic DB
//! bootstrap (same env contract as the other test suites), an AppState
//! wired to recording/flaky transports and a scripted scanner, a seeded
//! scenario generator over simulated time, and the drain loop that runs
//! the real claim/backoff/report queue primitives with an injected clock.

#![allow(dead_code)]

use core_api::email::{self, InboundStatus, MailStack};
use core_api::state::AppState;
use core_db::emails;
use core_mail::inbound::InboundEmail;
use core_mail::outbound::{backoff_delay, OutboundKind};
use core_mail::scan::{ScanError, ScanProvider, ScanVerdict};
use core_mail::transport::recording::FlakyTransport;
use core_mail::{MailTransport, RecordingTransport};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use uuid::Uuid;

// --- database bootstrap (env URL else reusable container) -------------------

struct Bootstrap {
    admin_url: String,
    _container: Option<testcontainers::ContainerAsync<testcontainers::GenericImage>>,
}

static BOOTSTRAP: tokio::sync::OnceCell<Bootstrap> = tokio::sync::OnceCell::const_new();

async fn bootstrap() -> &'static Bootstrap {
    BOOTSTRAP
        .get_or_init(|| async {
            if let Ok(url) =
                std::env::var("MARULA_TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"))
            {
                let admin = connect_with_retry(&url).await;
                reset_schema(&admin).await;
                core_db::run_migrations(&admin).await.expect("migrate");
                return Bootstrap {
                    admin_url: url,
                    _container: None,
                };
            }
            let request = testcontainers::ContainerRequest::from(
                testcontainers::GenericImage::new("pgvector/pgvector", "pg18")
                    .with_wait_for(testcontainers::core::WaitFor::message_on_stderr(
                        "database system is ready to accept connections",
                    ))
                    .with_exposed_port(testcontainers::core::ContainerPort::Tcp(5432)),
            )
            .with_container_name("marula-mail-harness-pg18")
            .with_reuse(testcontainers::core::ReuseDirective::Always)
            .with_env_var("POSTGRES_USER", "marula")
            .with_env_var("POSTGRES_PASSWORD", "marula")
            .with_env_var("POSTGRES_DB", "marula_harness");
            let container = request.start().await.expect("docker required");
            let host = container.get_host().await.expect("host");
            let port = container.get_host_port_ipv4(5432_u16).await.expect("port");
            let admin_url = format!("postgres://marula:marula@{host}:{port}/marula_harness");
            let admin = connect_with_retry(&admin_url).await;
            reset_schema(&admin).await;
            core_db::run_migrations(&admin).await.expect("migrate");
            Bootstrap {
                admin_url,
                _container: Some(container),
            }
        })
        .await
}

async fn connect_with_retry(url: &str) -> PgPool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(pool) = core_db::connect(url).await {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return pool;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "postgres unreachable at {url}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn reset_schema(admin: &PgPool) {
    sqlx::query(
            "DO $$ BEGIN DROP SCHEMA IF EXISTS ops CASCADE; DROP SCHEMA IF EXISTS public CASCADE; END $$",
        )
        .execute(admin)
        .await
        .expect("drop");
    sqlx::query("CREATE SCHEMA public")
        .execute(admin)
        .await
        .expect("create");
}

fn app_url(admin_url: &str) -> String {
    let (scheme, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    match rest.split_once('@') {
        Some((_, host)) => format!("{scheme}://marula_app:marula_app@{host}"),
        None => format!("{scheme}://marula_app:marula_app@{rest}"),
    }
}

/// Scripted scanner: `[VIRUS]` in the body ⇒ infected; anything else
/// clean. Deterministic stand-in for clamd.
pub struct ScriptedScan;

#[async_trait::async_trait]
impl ScanProvider for ScriptedScan {
    fn name(&self) -> &'static str {
        "scripted"
    }

    async fn scan(&self, body_text: &str) -> Result<ScanVerdict, ScanError> {
        Ok(ScanVerdict {
            infected: body_text.contains("[VIRUS]"),
            skipped: false,
            detail: None,
        })
    }
}

// --- scenario model ----------------------------------------------------------

/// One simulated pipeline input, ordered by simulated seconds offset.
#[derive(Clone)]
pub enum SimEvent {
    /// A provider POST (fresh message or identical replay of an earlier one).
    Webhook {
        at_secs: i64,
        msg: Box<InboundEmail>,
    },
    /// Subject-token fallback reply; the ticket number is resolved at RUN
    /// time (ticket numbers do not exist at generation time).
    TokenReply { at_secs: i64, rfc: String },
    /// Staff answering the requester via the API comment-hook enqueue path.
    StaffReply {
        at_secs: i64,
        ticket_title_hint: String,
    },
}

/// Generator-side ledger so assertions compare INTENT against OUTCOME.
#[derive(Default)]
pub struct Scenario {
    pub events: Vec<SimEvent>,
    pub inbound_ids: Vec<String>,
    templates: HashMap<String, InboundEmail>,
    pub last_ticket_number: Option<String>,
}

impl Scenario {
    /// Registers a fresh provider delivery (template kept for replays).
    pub fn inbound(&mut self, at_secs: i64, msg: InboundEmail) {
        self.templates
            .insert(msg.rfc_message_id.clone(), msg.clone());
        self.inbound_ids.push(msg.rfc_message_id.clone());
        self.events.push(SimEvent::Webhook {
            at_secs,
            msg: Box::new(msg),
        });
    }

    /// A threaded reply referencing a prior Message-ID.
    pub fn reply(&mut self, at_secs: i64, rfc: &str, target_rfc: &str, sender: &str) {
        let msg = InboundEmail {
            rfc_message_id: rfc.to_string(),
            in_reply_to: Some(target_rfc.to_string()),
            references: vec![target_rfc.to_string()],
            subject: Some("Re: printer".into()),
            from_address: sender.to_string(),
            from_name: None,
            to: vec!["support@acme.test".into()],
            cc: vec![],
            text: Some("still broken, adding detail".into()),
            html: None,
            received_at: None,
            attachments: vec![],
        };
        self.templates.insert(rfc.to_string(), msg.clone());
        self.inbound_ids.push(rfc.to_string());
        self.events.push(SimEvent::Webhook {
            at_secs,
            msg: Box::new(msg),
        });
    }

    /// Registers a subject-token fallback delivery (`[MER-n]` in the
    /// subject); the number is filled in when the pipeline reaches it.
    pub fn token_reply(&mut self, at_secs: i64, rfc: &str) {
        self.inbound_ids.push(rfc.to_string());
        self.events.push(SimEvent::TokenReply {
            at_secs,
            rfc: rfc.to_string(),
        });
    }

    /// Provider re-delivers an EARLIER message byte-for-byte.
    pub fn replay(&mut self, at_secs: i64, rfc: &str) {
        if let Some(template) = self.templates.get(rfc) {
            self.events.push(SimEvent::Webhook {
                at_secs,
                msg: Box::new(template.clone()),
            });
        }
    }

    /// Staff answers the requester (exercises the comment-hook enqueue).
    pub fn staff_reply(&mut self, at_secs: i64, ticket_title_hint: &str) {
        self.events.push(SimEvent::StaffReply {
            at_secs,
            ticket_title_hint: ticket_title_hint.into(),
        });
    }

    pub fn last_ticket_number(&self) -> &str {
        self.last_ticket_number.as_deref().unwrap_or("")
    }

    pub fn all_inbound_ids(&self) -> &[String] {
        &self.inbound_ids
    }
}

pub struct HarnessOrg {
    #[allow(unused)]
    pub org_id: Uuid,
    #[allow(unused)]
    pub board_id: Uuid,
}

// --- the harness --------------------------------------------------------------

pub struct MailHarness {
    pub admin: PgPool,
    pub app: PgPool,
    pub state: AppState,
    /// The SAME memory store `state.store` points at, typed for
    /// interaction assertions (put counts, op log) in attachment tests.
    pub store: Arc<core_store::memory::MemoryStore>,
    pub recording: Arc<RecordingTransport>,
    pub flaky: Arc<FlakyTransport<Arc<RecordingTransport>>>,
    pub org: HarnessOrg,
}

impl MailHarness {
    pub async fn boot() -> Self {
        let b = bootstrap().await;
        let admin = connect_with_retry(&b.admin_url).await;
        let app = connect_with_retry(&app_url(&b.admin_url)).await;

        let recording = Arc::new(RecordingTransport::new());
        let flaky = Arc::new(FlakyTransport::new(Arc::clone(&recording), 1));

        let mut config =
            core_api::config::Config::from_env_allow_dev_fallback().expect("test config");
        config.email_from_address = "support@acme.test".into();
        config.email_from_name = Some("Acme Support".into());

        let store = Arc::new(core_store::memory::MemoryStore::new());
        let mail_stack = Arc::new(MailStack {
            transport: Arc::clone(&flaky) as Arc<dyn MailTransport>,
            scanner: Arc::new(ScriptedScan),
            from_address: config.email_from_address.clone(),
            from_name: config.email_from_name.clone(),
        });
        let state = AppState {
            plugin_pool: Default::default(),
            pool: app.clone(),
            config: Arc::new(config),
            mail: mail_stack,
            store: std::sync::Arc::clone(&store) as std::sync::Arc<dyn core_store::ObjectStore>,
            routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
            oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
            reporting: core_api::state::AppState::reporting_pool_lazy(&app_url(&b.admin_url))
                .expect("lazy reporting pool"),
        };

        // Single-org world: default board + SLA + catch-all route + staff.
        let org = Uuid::now_v7();
        let mut tx = admin.begin().await.expect("tx");
        // A slug per boot: the harness boots once per test, and the tests in a
        // binary run in parallel, so a fixed slug collided on `orgs_slug_key`
        // the moment a file held more than one case.
        let slug = format!("acme-{}", &org.simple().to_string()[..12]);
        core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", &slug, "MER")
            .await
            .expect("org");
        core_db::orgs::seed_default_roles(&mut tx, org)
            .await
            .expect("roles");
        core_db::orgs::seed_default_board(&mut tx, org)
            .await
            .expect("board");
        core_db::orgs::seed_default_sla_policy(&mut *tx, org)
            .await
            .expect("sla");
        // Wave 39 lane 1: capture surfaces validate work_type against the
        // org taxonomy, so the harness org gets the standard set too
        // (mirrors sign-up provisioning + the demo seeder).
        core_db::time_categories::seed_default_time_categories(&mut tx, org)
            .await
            .expect("time categories");
        let board: Uuid =
            sqlx::query_scalar("SELECT id FROM boards WHERE org_id=$1 AND is_default")
                .bind(org)
                .fetch_one(&mut *tx)
                .await
                .expect("board id");
        emails::upsert_inbound_settings(&mut tx, org, "support@acme.test", board)
            .await
            .expect("catchall");
        tx.commit().await.expect("commit");

        Self {
            admin,
            app,
            state,
            store,
            recording,
            flaky,
            org: HarnessOrg {
                org_id: org,
                board_id: board,
            },
        }
    }

    /// Fresh deterministic scenario builder.
    pub fn scenario(&self) -> Scenario {
        Scenario::default()
    }

    /// Runs every webhook through the REAL pipeline in simulated order and
    /// performs the staff-reply enqueues through the same helper the API
    /// hook uses. Returns outcome counters for intent-vs-outcome checks.
    pub async fn run_inbound(&self, sim: &mut Scenario) -> OutcomeCounters {
        let mut events = std::mem::take(&mut sim.events);
        events.sort_by_key(|e| match e {
            SimEvent::Webhook { at_secs, .. } => *at_secs,
            SimEvent::TokenReply { at_secs, .. } => *at_secs,
            SimEvent::StaffReply { at_secs, .. } => *at_secs,
        });
        let mut counters = OutcomeCounters::default();
        for event in events {
            match event {
                SimEvent::Webhook { msg, .. } => {
                    match email::process_inbound(&self.state, *msg).await {
                        Ok(InboundStatus::Commented) => counters.commented += 1,
                        Ok(InboundStatus::TicketCreated) => {
                            counters.tickets_created += 1;
                            sim.last_ticket_number =
                            Some(sqlx::query_scalar::<_, String>(
                                "SELECT number FROM tickets WHERE org_id=$1 ORDER BY created_at DESC LIMIT 1",
                            )
                            .bind(self.org.org_id)
                            .fetch_one(&self.admin)
                            .await
                            .expect("latest number"));
                        }
                        Ok(InboundStatus::Quarantined) => counters.quarantined += 1,
                        Ok(InboundStatus::Duplicate) => counters.duplicate += 1,
                        Ok(InboundStatus::Unroutable) => counters.unroutable += 1,
                        Err(e) => panic!("pipeline failed: {e}"),
                    }
                }
                SimEvent::TokenReply { rfc, .. } => {
                    let number = sim.last_ticket_number.clone().unwrap_or_default();
                    let msg = InboundEmail {
                        rfc_message_id: rfc,
                        in_reply_to: None,
                        references: vec![],
                        subject: Some(format!("[{number}] lost the thread tag")),
                        from_address: "client@example.test".into(),
                        from_name: None,
                        to: vec!["support@acme.test".into()],
                        cc: vec![],
                        text: Some("replying by keeping the bracket tag".into()),
                        html: None,
                        received_at: None,
                        attachments: vec![],
                    };
                    match email::process_inbound(&self.state, msg).await {
                        Ok(InboundStatus::Commented) => counters.commented += 1,
                        Ok(other) => panic!("token reply became {other:?}"),
                        Err(e) => panic!("pipeline failed: {e}"),
                    }
                }
                SimEvent::StaffReply {
                    ticket_title_hint, ..
                } => {
                    self.staff_reply_enqueue(&ticket_title_hint).await;
                    counters.staff_replies += 1;
                }
            }
        }
        counters
    }

    /// Mirrors routes/tickets.rs add_comment: enqueue-only inside an
    /// org-bound app-role transaction; the worker transports later.
    async fn staff_reply_enqueue(&self, title_hint: &str) {
        // Harness-side intent resolution reads via the owner pool (the
        // production hook would already hold an authed org context).
        let ticket_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM tickets WHERE org_id=$2 AND title LIKE $1 || '%' ORDER BY created_at DESC LIMIT 1",
        )
        .bind(title_hint)
        .bind(self.org.org_id)
        .fetch_one(&self.admin)
        .await
        .expect("ticket exists");
        let (number, requester_email): (String, String) = sqlx::query_as(
            "SELECT t.number, m.from_address FROM tickets t \
             JOIN email_messages m ON m.ticket_id=t.id AND m.direction='inbound' \
                  AND m.scan_status='clean' \
             WHERE t.id=$1 ORDER BY m.created_at LIMIT 1",
        )
        .bind(ticket_id)
        .fetch_one(&self.admin)
        .await
        .expect("originating email");
        let mut tx = self.app.begin().await.expect("tx");
        core_db::set_org(&mut tx, self.org.org_id)
            .await
            .expect("set_org");
        email::enqueue_outbound(
            &mut tx,
            email::OutboundEnqueue {
                mail: &self.state.mail,
                org_id: self.org.org_id,
                to_addresses: vec![requester_email],
                kind: &OutboundKind::CommentNotification {
                    ticket_number: number,
                    ticket_title: title_hint.to_string(),
                    excerpt: "we are on it".into(),
                },
                in_reply_to: None,
                references: Vec::new(),
                ticket_id: Some(ticket_id),
                // Seeded directly for transport coverage: nobody wrote this
                // one, so the workspace signature closes it.
                signer: None,
                overrides: email::EnvelopeOverrides::default(),
            },
        )
        .await
        .expect("enqueue staff reply");
        tx.commit().await.expect("commit");
    }

    /// Drains the outbound queue through the flaky transport using the SAME
    /// queue primitives production does, advancing a simulated clock so
    /// backoff elapses instantly. Owner-role connection is fine here: this
    /// IS the cross-org worker role equivalent, and the clock injection is
    /// the whole point of the harness variant.
    pub async fn drain_outbound(&self) {
        let mut clock = chrono::Utc::now();
        for _ in 0..500 {
            let mut tx = self.admin.begin().await.expect("tx");
            let batch = emails::claim_due_outbound(&mut *tx, None, 100, 300.0, clock)
                .await
                .expect("claim");
            tx.commit().await.expect("commit");

            if batch.is_empty() {
                let pending: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM email_outbound WHERE status IN ('queued','sending')",
                )
                .fetch_one(&self.admin)
                .await
                .expect("pending");
                if pending == 0 {
                    break;
                }
                clock += chrono::Duration::seconds(600);
                continue;
            }

            for claimed in batch {
                let wire = core_mail::internal::ClaimedOutbound {
                    id: claimed.id,
                    org_id: claimed.org_id,
                    rfc_message_id: claimed.rfc_message_id.clone(),
                    from_address: claimed.from_address.clone(),
                    from_name: claimed.from_name.clone(),
                    // The simulated worker fixture carries no board identity,
                    // so there is no Reply-To to resolve (compile fix, Wave 34
                    // lane 6: the fast-forwarded lane-8 change added the field).
                    reply_to: None,
                    to_addresses: claimed.to_addresses.clone(),
                    subject: claimed.subject.clone(),
                    body_text: claimed.body_text.clone(),
                    in_reply_to: claimed.in_reply_to.clone(),
                    references: claimed.references.clone(),
                    attempts: claimed.attempts,
                };
                let result = self.state.mail.transport.send(&wire.into_outbound()).await;
                let mut tx = self.admin.begin().await.expect("tx");
                match result {
                    Ok(receipt) => {
                        emails::apply_send_outcome(
                            &mut *tx,
                            &emails::SendOutcome {
                                id: claimed.id,
                                ok: true,
                                provider_message_id: receipt.provider_message_id,
                                error: None,
                                retry_delay_secs: 0,
                                permanent: false,
                            },
                            clock,
                        )
                        .await
                        .expect("apply sent");
                    }
                    Err(e) => {
                        emails::apply_send_outcome(
                            &mut *tx,
                            &emails::SendOutcome {
                                id: claimed.id,
                                ok: false,
                                provider_message_id: None,
                                error: Some(e.to_string()),
                                retry_delay_secs: backoff_delay(i64::from(claimed.attempts))
                                    .num_seconds()
                                    .max(1),
                                permanent: e.permanent(),
                            },
                            clock,
                        )
                        .await
                        .expect("apply retry");
                    }
                }
                tx.commit().await.expect("commit");
            }
            clock += chrono::Duration::seconds(120);
        }
    }

    /// rfc_message_id → how many times the transport recorded it.
    pub async fn transport_recordings(&self) -> HashMap<String, usize> {
        let mut map: HashMap<String, usize> = HashMap::new();
        for m in self.recording.recorded().await {
            *map.entry(m.rfc_message_id).or_insert(0) += 1;
        }
        map
    }

    /// rfc_message_id → attempts counted by the flaky wrapper.
    pub async fn flaky_attempts(&self) -> Vec<(String, usize)> {
        let rfcs: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT m.rfc_message_id FROM email_messages m \
             JOIN email_outbound o ON o.email_message_id=m.id WHERE o.status='sent'",
        )
        .fetch_all(&self.admin)
        .await
        .expect("sent rfcs");
        let mut out = Vec::new();
        for rfc in rfcs {
            let attempts = self.flaky.attempt_count(&rfc).await;
            out.push((rfc, attempts));
        }
        out
    }
}

#[derive(Default, Debug)]
pub struct OutcomeCounters {
    pub commented: usize,
    pub tickets_created: usize,
    pub quarantined: usize,
    pub duplicate: usize,
    pub unroutable: usize,
    pub staff_replies: usize,
}
