//! Daily digest loop (Wave 34 lane 8).
//!
//! Purpose: once the local digest hour arrives for a user who opted in
//! (`email.digest = daily`), summarize their unread notifications in ONE
//! email. The worker owns the clock and the timezone read (`users.timezone`);
//! core-api owns rendering, the org policy, and the once-per-local-day
//! marker, so overlapping sweeps cannot double-send.
//!
//! Quiet hours do NOT suppress the digest: it is scheduled, not
//! interruptive — the chosen hour is the user's own quiet-hours answer.
//!
//! Non-purpose: no database writes (single-writer §4.1 — the marker and the
//! ledger row are core-api's), no rendering, no preference-chain evaluation
//! beyond the opt-in and the hour.

use chrono::Timelike as _;
use core_domain::settings as registry;
use core_mail::internal::{DigestEmailRequest, DigestItemDto, EnqueueEmailResponse};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Most rows a single digest lists before summarizing the remainder.
const MAX_DIGEST_ITEMS: usize = 25;

/// Loop settings, assembled by the host from the shared worker config.
#[derive(Debug, Clone)]
pub struct DigestLoopConfig {
    pub core_api_url: String,
    pub internal_token: Option<String>,
    /// Seconds between candidate sweeps. Each sweep only sends to users
    /// whose LOCAL hour currently equals their chosen hour.
    pub poll_secs: u64,
}

/// Runs the digest sweep until `shutdown` fires.
pub async fn run_digest_loop(
    pool: PgPool,
    config: DigestLoopConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    if config.internal_token.is_none() {
        tracing::warn!("MERIDIAN_INTERNAL_TOKEN unset: daily digest loop disabled");
        return;
    }
    let client = reqwest::Client::new();
    tracing::info!(api = %config.core_api_url, "daily digest loop started");
    loop {
        if *shutdown.borrow() {
            return;
        }
        match pass(&pool, &client, &config).await {
            Ok(0) => {}
            Ok(sent) => tracing::info!(sent, "daily digest pass enqueued mail"),
            Err(error) => tracing::error!(%error, "daily digest pass failed"),
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(Duration::from_secs(config.poll_secs.max(60))) => {}
        }
    }
}

/// One sweep: find opted-in users whose local hour matches, summarize
/// their unread notifications, and ask core-api to enqueue.
async fn pass(
    pool: &PgPool,
    client: &reqwest::Client,
    config: &DigestLoopConfig,
) -> anyhow::Result<usize> {
    let Some(token) = config.internal_token.as_deref() else {
        return Ok(0);
    };
    let candidates = core_db::settings::list_user_settings_with_text_value(
        pool,
        registry::EMAIL_DIGEST,
        "daily",
    )
    .await?;
    let mut enqueued = 0_usize;
    for (org_id, user_id) in candidates {
        let Ok(user) = core_db::users::get_user(pool, org_id, user_id).await else {
            continue;
        };
        if !user.is_active || user.email.trim().is_empty() {
            continue;
        }
        let hour = digest_hour(pool, org_id, user_id).await?;
        let tz: chrono_tz::Tz = user.timezone.parse().unwrap_or(chrono_tz::UTC);
        let local = chrono::Utc::now().with_timezone(&tz);
        if local.hour() != hour {
            continue;
        }
        let unread =
            core_db::notifications::list_for_user(pool, org_id, user_id, true, 100).await?;
        if unread.is_empty() {
            continue;
        }
        let total = unread.len();
        let mut items: Vec<DigestItemDto> = unread
            .into_iter()
            .take(MAX_DIGEST_ITEMS)
            .map(|n| DigestItemDto {
                title: n.title,
                detail: n.body,
            })
            .collect();
        if total > MAX_DIGEST_ITEMS {
            items.push(DigestItemDto {
                title: format!("and {} more unread notifications", total - MAX_DIGEST_ITEMS),
                detail: None,
            });
        }
        let request = DigestEmailRequest {
            org_id,
            user_id,
            day: local.format("%Y-%m-%d").to_string(),
            items,
        };
        let response = client
            .post(format!(
                "{}/api/internal/email/digest",
                config.core_api_url.trim_end_matches('/')
            ))
            .header("X-Internal-Token", token)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json::<EnqueueEmailResponse>()
            .await?;
        if response.outcome == "queued" {
            enqueued += 1;
        }
        tracing::debug!(
            %org_id,
            %user_id,
            day = %request.day,
            outcome = %response.outcome,
            "digest decision"
        );
    }
    Ok(enqueued)
}

/// The user's chosen send hour, defaulting to the registry default.
async fn digest_hour(
    pool: &PgPool,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> anyhow::Result<u32> {
    let stored =
        core_db::settings::get_user_value(pool, org_id, user_id, registry::EMAIL_DIGEST_HOUR)
            .await?;
    let fallback = registry::default_value(registry::EMAIL_DIGEST_HOUR)
        .and_then(|v| v.to_json().as_i64())
        .unwrap_or(7);
    let hour = stored.and_then(|v| v.as_i64()).unwrap_or(fallback);
    Ok(u32::try_from(hour.clamp(0, 23)).unwrap_or(7))
}

/// Shared sink handle so lib.rs can hand the loop one Arc.
pub type DigestHandle = Arc<DigestLoopConfig>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_item_cap_is_sane() {
        // Compile-time assertion: the cap must stay inside the sane band.
        const { assert!(MAX_DIGEST_ITEMS >= 10 && MAX_DIGEST_ITEMS <= 100) };
    }

    #[test]
    fn hour_clamps_into_a_real_clock_hour() {
        // The clamp helper is inline in digest_hour; verify the math used
        // there stays in range for absurd stored values.
        for raw in [-5_i64, 0, 7, 23, 99] {
            let hour = u32::try_from(raw.clamp(0, 23)).unwrap_or(7);
            assert!(hour <= 23);
        }
    }
}
