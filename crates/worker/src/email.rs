//! Outbound email send loop (TASKLIST 2.4).
//!
//! Purpose: claim due sends from core-api, resolve the ORG's transport
//! adapter (settings + sealed credential, Wave 34 lane 8 — env transport
//! remains the fallback), perform the transport call, and report
//! per-message results back — the core-api-callback pattern that keeps
//! core-api the ONLY database writer (§4.1). Lease recovery on core-api's
//! side re-claims sends abandoned by a dead worker, so a crash between send
//! and report can duplicate at worst one message (at-least-once delivery;
//! the RFC Message-ID keeps threading stable across such redelivery).
//!
//! Non-purpose: no writes (the resolver and this loop only read), no
//! backoff math (core-api applies it), no rendering.
//!
//! None-mode: an org whose transport is `none` (or whose credential is
//! missing) reports a retryable failure so the ledger's backoff holds the
//! message — nothing is silently dropped and nothing is dead-lettered for a
//! configuration gap.

use crate::config::EmailLoopConfig;
use crate::mail_config::OrgMailResolver;
use core_mail::internal::{ClaimRequest, ClaimedOutbound, ResultsRequest, SendResult};
use core_mail::{MailTransport, TransportError};
use std::sync::Arc;
use std::time::Duration;

/// Error backoff. Independent of the idle poll cadence, so a transient
/// failure retries quickly even when an operator stretched `EMAIL_POLL_SECS`
/// for a small install.
const ERROR_BACKOFF: Duration = Duration::from_secs(2);

/// Runs the send loop until `shutdown` fires. Returns when told to stop;
/// transient HTTP/transport errors are logged and retried next cycle.
pub async fn run_email_loop(
    config: EmailLoopConfig,
    resolver: Arc<dyn OrgMailResolver>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    if config.internal_token.is_none() {
        tracing::warn!(
            "EMAIL_INTERNAL_TOKEN unset: outbound email loop disabled (mail stays queued)"
        );
        return;
    }
    let client = reqwest::Client::new();
    tracing::info!(
        api = %config.core_api_url,
        poll_secs = config.poll_secs,
        "outbound email loop started (per-org transport resolution)"
    );
    loop {
        if *shutdown.borrow_and_update() {
            return;
        }
        let idle = Duration::from_secs(config.poll_secs);
        match cycle(&config, &client, resolver.as_ref()).await {
            Ok(0) => tokio::time::sleep(idle).await,
            Ok(_) => {} // work remains; loop immediately
            Err(e) => {
                tracing::error!(error = %e, "email cycle failed");
                tokio::time::sleep(ERROR_BACKOFF).await;
            }
        }
        if shutdown.has_changed().unwrap_or(true) {
            return;
        }
    }
}

/// One claim→resolve→send→report pass. Returns how many messages were
/// processed.
async fn cycle(
    config: &EmailLoopConfig,
    client: &reqwest::Client,
    resolver: &dyn OrgMailResolver,
) -> anyhow::Result<usize> {
    let Some(token) = config.internal_token.as_deref() else {
        return Ok(0);
    };
    let claimed: Vec<ClaimedOutbound> = client
        .post(format!(
            "{}/api/internal/email/outbound/claim",
            config.core_api_url.trim_end_matches('/')
        ))
        .header("X-Internal-Token", token)
        .json(&ClaimRequest {
            lease_secs: config.lease_secs,
            limit: config.claim_batch,
        })
        .send()
        .await?
        .error_for_status()?
        .json::<core_mail::internal::ClaimResponse>()
        .await?
        .messages;

    if claimed.is_empty() {
        return Ok(0);
    }

    let mut results = Vec::with_capacity(claimed.len());
    for message in &claimed {
        // Resolution first: a configuration gap is a retryable report, not
        // a lost claim.
        let transport: Arc<dyn MailTransport> = match resolver.transport_for(message.org_id).await {
            Ok(transport) => transport,
            Err(reason) => {
                tracing::warn!(id = %message.id, %reason, "org mail transport unresolved");
                results.push(SendResult {
                    id: message.id,
                    ok: false,
                    attempts: message.attempts,
                    provider_message_id: None,
                    error: Some(reason),
                });
                continue;
            }
        };
        let outbound = message.clone().into_outbound();
        let result = match transport.send(&outbound).await {
            Ok(receipt) => {
                tracing::debug!(
                    id = %message.id,
                    transport = transport.name(),
                    "outbound email sent"
                );
                SendResult {
                    id: message.id,
                    ok: true,
                    attempts: message.attempts,
                    provider_message_id: receipt.provider_message_id,
                    error: None,
                }
            }
            Err(TransportError::Rejected(reason)) => {
                // Rejections are permanent for identical content but the
                // ledger still owns retry/dead decisions.
                tracing::warn!(id = %message.id, %reason, "transport rejected message");
                SendResult {
                    id: message.id,
                    ok: false,
                    attempts: message.attempts,
                    provider_message_id: None,
                    error: Some(reason),
                }
            }
            Err(TransportError::Unavailable(reason)) => {
                tracing::warn!(id = %message.id, reason, "transport unavailable");
                SendResult {
                    id: message.id,
                    ok: false,
                    attempts: message.attempts,
                    provider_message_id: None,
                    error: Some(reason),
                }
            }
        };
        results.push(result);
    }

    let applied: serde_json::Value = client
        .post(format!(
            "{}/api/internal/email/outbound/results",
            config.core_api_url.trim_end_matches('/')
        ))
        .header("X-Internal-Token", token)
        .json(&ResultsRequest { results })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    tracing::debug!(applied = %applied["applied"], "reported email results");
    Ok(claimed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_config::StaticResolver;
    use axum::routing::post;
    use axum::{Json, Router};
    use core_mail::internal::ClaimResponse;
    use core_mail::RecordingTransport;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fake_claimed(id: uuid::Uuid) -> ClaimedOutbound {
        ClaimedOutbound {
            id,
            org_id: uuid::Uuid::now_v7(),
            rfc_message_id: "m1@marula.local".into(),
            from_address: "support@acme.test".into(),
            from_name: None,
            reply_to: Some("projects@acme.test".into()),
            to_addresses: vec!["client@example.test".into()],
            subject: Some("Re: hi".into()),
            body_text: Some("body".into()),
            in_reply_to: None,
            references: vec![],
            attempts: 1,
        }
    }

    fn cfg(addr: std::net::SocketAddr, token: Option<&str>) -> EmailLoopConfig {
        EmailLoopConfig {
            core_api_url: format!("http://{addr}"),
            internal_token: token.map(str::to_string),
            transport_kind: core_mail::TransportKind::None,
            claim_batch: 10,
            lease_secs: 300,
            poll_secs: 2,
        }
    }

    #[tokio::test]
    async fn cycle_sends_each_claim_exactly_once_and_reports() {
        // Simplified end-to-end: point the cycle at a mock API serving ONE
        // claimed message; recording transport must record it once, and
        // the report must carry ok=true with the receipt id.
        let seen_results: Arc<std::sync::Mutex<Vec<SendResult>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let token_hits = Arc::new(AtomicUsize::new(0));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let claimed = vec![fake_claimed(uuid::Uuid::now_v7())];
        let results_clone = Arc::clone(&seen_results);
        let hits_clone = Arc::clone(&token_hits);
        tokio::spawn(async move {
            let app = Router::new()
                .route(
                    "/api/internal/email/outbound/claim",
                    post(
                        move |headers: axum::http::HeaderMap, Json(req): Json<ClaimRequest>| {
                            let claimed = claimed.clone();
                            let hits = Arc::clone(&hits_clone);
                            async move {
                                assert!(req.limit >= 1);
                                assert_eq!(
                                    headers
                                        .get("X-Internal-Token")
                                        .and_then(|v| v.to_str().ok()),
                                    Some("tok")
                                );
                                hits.fetch_add(1, Ordering::SeqCst);
                                Json(ClaimResponse { messages: claimed })
                            }
                        },
                    ),
                )
                .route(
                    "/api/internal/email/outbound/results",
                    post(move |Json(req): Json<ResultsRequest>| {
                        let seen = Arc::clone(&results_clone);
                        async move {
                            seen.lock().unwrap().extend(req.results);
                            Json(serde_json::json!({ "applied": 1 }))
                        }
                    }),
                );
            axum::serve(listener, app).await.unwrap();
        });

        let recording = Arc::new(RecordingTransport::new());
        let resolver = StaticResolver(Arc::clone(&recording) as Arc<dyn MailTransport>);
        let client = reqwest::Client::new();
        let n = cycle(&cfg(addr, Some("tok")), &client, &resolver)
            .await
            .expect("cycle succeeds");
        assert_eq!(n, 1);
        assert_eq!(recording.count().await, 1);
        // The reply-to resolved by core-api at claim time rides the send.
        let recorded = recording.recorded().await;
        assert_eq!(recorded[0].reply_to.as_deref(), Some("projects@acme.test"));
        let reported = seen_results.lock().unwrap();
        assert_eq!(reported.len(), 1);
        assert!(reported[0].ok);
        assert_eq!(
            reported[0].provider_message_id.as_deref(),
            Some("rec-m1@marula.local")
        );
    }

    #[tokio::test]
    async fn cycle_without_token_never_calls_the_api() {
        let resolver = StaticResolver(Arc::new(RecordingTransport::new()));
        let n = cycle(
            &cfg("127.0.0.1:1".parse().unwrap(), None),
            &reqwest::Client::new(),
            &resolver,
        )
        .await
        .expect("disabled loop is a no-op");
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn transport_failures_report_ok_false_with_error_detail() {
        struct Failing;
        #[async_trait::async_trait]
        impl MailTransport for Failing {
            fn name(&self) -> &'static str {
                "failing"
            }
            async fn send(
                &self,
                _m: &core_mail::OutboundEmail,
            ) -> Result<core_mail::SendReceipt, TransportError> {
                Err(TransportError::Unavailable("smtp down".into()))
            }
        }
        let seen_results: Arc<std::sync::Mutex<Vec<SendResult>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let claimed = vec![fake_claimed(uuid::Uuid::now_v7())];
        let clone = Arc::clone(&seen_results);
        tokio::spawn(async move {
            let app = Router::new()
                .route(
                    "/api/internal/email/outbound/claim",
                    post(move || {
                        let claimed = claimed.clone();
                        async move { Json(ClaimResponse { messages: claimed }) }
                    }),
                )
                .route(
                    "/api/internal/email/outbound/results",
                    post(move |Json(req): Json<ResultsRequest>| {
                        let seen = Arc::clone(&clone);
                        async move {
                            seen.lock().unwrap().extend(req.results);
                            Json(serde_json::json!({ "applied": 1 }))
                        }
                    }),
                );
            axum::serve(listener, app).await.unwrap();
        });
        let resolver = StaticResolver(Arc::new(Failing) as Arc<dyn MailTransport>);
        cycle(&cfg(addr, Some("tok")), &reqwest::Client::new(), &resolver)
            .await
            .expect("cycle completes even when sends fail");
        let reported = seen_results.lock().unwrap();
        assert_eq!(reported.len(), 1);
        assert!(!reported[0].ok);
        assert_eq!(reported[0].error.as_deref(), Some("smtp down"));
    }

    #[tokio::test]
    async fn unresolved_transport_reports_retryable_not_lost() {
        struct Broken;
        #[async_trait::async_trait]
        impl OrgMailResolver for Broken {
            async fn transport_for(
                &self,
                _org_id: uuid::Uuid,
            ) -> Result<Arc<dyn MailTransport>, String> {
                Err("no outbound transport is configured for this organization".into())
            }
        }
        let seen_results: Arc<std::sync::Mutex<Vec<SendResult>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let claimed = vec![fake_claimed(uuid::Uuid::now_v7())];
        let clone = Arc::clone(&seen_results);
        tokio::spawn(async move {
            let app = Router::new()
                .route(
                    "/api/internal/email/outbound/claim",
                    post(move || {
                        let claimed = claimed.clone();
                        async move { Json(ClaimResponse { messages: claimed }) }
                    }),
                )
                .route(
                    "/api/internal/email/outbound/results",
                    post(move |Json(req): Json<ResultsRequest>| {
                        let seen = Arc::clone(&clone);
                        async move {
                            seen.lock().unwrap().extend(req.results);
                            Json(serde_json::json!({ "applied": 1 }))
                        }
                    }),
                );
            axum::serve(listener, app).await.unwrap();
        });
        cycle(&cfg(addr, Some("tok")), &reqwest::Client::new(), &Broken)
            .await
            .expect("configuration gaps are reported, never fatal");
        let reported = seen_results.lock().unwrap();
        assert_eq!(reported.len(), 1);
        assert!(!reported[0].ok);
        assert!(reported[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("no outbound transport"));
    }
}
