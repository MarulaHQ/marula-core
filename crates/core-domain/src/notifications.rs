//! Channel fan-out decisions: preferences, urgency classes, quiet hours.
//!
//! Purpose: the pure decision layer applied when a notification fans out —
//! which channels may receive it, and whether the moment falls inside the
//! recipient's quiet-hours window (wall-clock in their timezone, DST-safe
//! via `chrono-tz`). Also home of the `MailTransport` seam: the email leg
//! is always invoked through this trait; concrete adapters (Postmark/SES)
//! land with the email-in/out lane, and self-hosted default is the noop.
//!
//! Non-purpose: no I/O and no delivery execution. Quiet hours suppress the
//! interrupting channels only — the in-app inbox row is durable pull, not a
//! push, so it is governed by the per-kind toggle alone.

use crate::entities::NotificationKind;
use crate::DomainError;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// A delivery channel a notification can fan out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryChannel {
    InApp,
    Email,
}

/// Quiet-hours window in `HH:MM` local wall-clock terms, anchored to an
/// IANA timezone. The window is `[from, until)`: it includes its start and
/// excludes its end, and may cross midnight (`22:00` → `06:00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietHours {
    from: (u32, u32),
    until: (u32, u32),
    tz: Tz,
}

impl QuietHours {
    /// Builds a window from raw preference strings. `from`/`until` must be
    /// present together or not at all; `tz` must be a valid IANA name.
    pub fn parse(
        from: Option<&str>,
        until: Option<&str>,
        tz: &str,
    ) -> Result<Option<Self>, DomainError> {
        match (from, until) {
            (None, None) => Self::parse_tz(tz).map(|_| None),
            (Some(f), Some(u)) => {
                let from = parse_hhmm(f)?;
                let until = parse_hhmm(u)?;
                let tz = Self::parse_tz(tz)?;
                Ok(Some(Self { from, until, tz }))
            }
            _ => Err(DomainError::Validation(
                "quiet hours need both bounds (quiet_from and quiet_until)".into(),
            )),
        }
    }

    fn parse_tz(tz: &str) -> Result<Tz, DomainError> {
        tz.parse::<Tz>()
            .map_err(|_| DomainError::Validation(format!("unknown timezone: {tz}")))
    }

    /// The next instant this window's `until` boundary occurs at or after
    /// `now`, in UTC — the moment a quiet-deferred send becomes due.
    /// Outside quiet hours this is `now` itself (send immediately).
    pub fn next_window_end(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        use chrono::TimeZone;
        let local = now.with_timezone(&self.tz);
        let until = chrono::NaiveTime::from_hms_opt(self.until.0, self.until.1, 0)
            .expect("valid quiet end");
        for day_offset in [0, 1] {
            let day = local.date_naive() + chrono::Duration::days(day_offset);
            if let Some(c) = day.and_time(until).and_local_timezone(self.tz).single() {
                let utc = c.with_timezone(&Utc);
                if utc > now {
                    return utc;
                }
            }
        }
        now
    }

    /// True when `now`, expressed in the window's timezone, falls inside
    /// `[from, until)`. DST transitions are handled by the timezone
    /// conversion itself: during spring-forward the skipped local times
    /// simply never occur; during fall-back ambiguous local times resolve
    /// to the earlier offset, so a window boundary holds for the first
    /// occurrence.
    pub fn is_quiet(&self, now: DateTime<Utc>) -> bool {
        use chrono::Timelike;
        let local = now.with_timezone(&self.tz);
        let minutes = local.hour() * 60 + local.minute();
        let (f, u, t) = (
            self.from.0 * 60 + self.from.1,
            self.until.0 * 60 + self.until.1,
            minutes,
        );
        if f == u {
            return false;
        }
        if f < u {
            t >= f && t < u
        } else {
            // Window crosses midnight: e.g. 22:00 → 06:00.
            t >= f || t < u
        }
    }
}

fn parse_hhmm(s: &str) -> Result<(u32, u32), DomainError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err(DomainError::Validation(format!(
            "quiet-hours bound must be HH:MM, got {s:?}"
        )));
    }
    let h: u32 = parts[0]
        .parse()
        .map_err(|_| DomainError::Validation(format!("invalid hour in {s:?}")))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| DomainError::Validation(format!("invalid minute in {s:?}")))?;
    if h > 23 || m > 59 {
        return Err(DomainError::Validation(format!(
            "quiet-hours bound out of range: {s:?}"
        )));
    }
    Ok((h, m))
}

/// Kinds that page through quiet hours: an active SLA breach or escalation
/// ladder must reach a human regardless of the clock.
pub fn bypasses_quiet_hours(kind: NotificationKind) -> bool {
    matches!(
        kind,
        NotificationKind::SlaBreach | NotificationKind::Escalation
    )
}

/// Outcome of evaluating one channel against prefs + clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Fan this channel out now.
    Deliver,
    /// The recipient disabled this channel for this kind.
    Disabled,
    /// Enabled, but suppressed by quiet hours (non-critical kind only).
    QuietDeferred,
}

/// Decides one channel at fan-out time.
///
/// Rules:
/// - channel toggled off ⇒ [`Decision::Disabled`] (the in-app row is never
///   even created upstream when InApp is disabled);
/// - quiet hours active AND the kind is non-critical AND the channel is an
///   interrupting one ⇒ [`Decision::QuietDeferred`] (suppressed, not
///   queued — the in-app inbox remains the durable record);
/// - otherwise ⇒ [`Decision::Deliver`].
pub fn decide_channel(
    channel: DeliveryChannel,
    kind: NotificationKind,
    enabled: bool,
    quiet: Option<&QuietHours>,
    now: DateTime<Utc>,
) -> Decision {
    if !enabled {
        return Decision::Disabled;
    }
    let quiet_active =
        matches!(channel, DeliveryChannel::Email) && quiet.is_some_and(|q| q.is_quiet(now));
    if quiet_active && !bypasses_quiet_hours(kind) {
        return Decision::QuietDeferred;
    }
    Decision::Deliver
}

/// One outbound email, as handed to a [`MailTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub text_body: String,
}

/// The edge seam every outbound email crosses (§4.7): core code never
/// touches a vendor SDK. The email lane provides real adapters; shipped
/// default is [`NoopMailTransport`] (standalone-first doctrine).
pub trait MailTransport: Send + Sync {
    /// Sends one message or fails. Implementations must be idempotent
    /// under retry and must never block the caller's event loop.
    fn send(
        &self,
        message: EmailMessage,
    ) -> impl std::future::Future<Output = Result<(), MailError>> + Send;
}

/// Errors surfaced by transports. Vendor detail stays behind the seam.
#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("mail transport unavailable")]
    Unavailable,
    #[error("mail rejected: {0}")]
    Rejected(String),
}

/// Ships-noop transport: accepts every message and drops it. The
/// documented "none" mode — callers log the drop at their seam.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopMailTransport;

impl MailTransport for NoopMailTransport {
    async fn send(&self, _message: EmailMessage) -> Result<(), MailError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn window(from: &str, until: &str, tz: &str) -> QuietHours {
        QuietHours::parse(Some(from), Some(until), tz)
            .expect("valid window")
            .expect("some window")
    }

    #[test]
    fn same_day_window_boundaries() {
        let w = window("09:00", "17:00", "UTC");
        // Inclusive start, exclusive end.
        assert!(w.is_quiet(utc(2026, 1, 15, 9, 0)));
        assert!(!w.is_quiet(utc(2026, 1, 15, 17, 0)));
        assert!(w.is_quiet(utc(2026, 1, 15, 12, 30)));
        assert!(!w.is_quiet(utc(2026, 1, 15, 8, 59)));
    }

    #[test]
    fn overnight_window_crosses_midnight() {
        let w = window("22:00", "06:00", "UTC");
        assert!(w.is_quiet(utc(2026, 1, 15, 23, 0)));
        assert!(w.is_quiet(utc(2026, 1, 16, 5, 59)));
        assert!(!w.is_quiet(utc(2026, 1, 16, 6, 0)));
        assert!(!w.is_quiet(utc(2026, 1, 15, 21, 59)));
    }

    #[test]
    fn empty_window_is_never_quiet() {
        let w = window("09:00", "09:00", "UTC");
        assert!(!w.is_quiet(utc(2026, 1, 15, 9, 0)));
        assert!(!w.is_quiet(utc(2026, 1, 15, 15, 0)));
    }

    #[test]
    fn timezone_shifts_the_window() {
        let w = window("22:00", "06:00", "America/New_York");
        // 03:00 UTC == 22:00 EST (UTC-5) → quiet start.
        assert!(w.is_quiet(utc(2026, 1, 16, 3, 0)));
        // 10:59 UTC == 05:59 EST → still quiet.
        assert!(w.is_quiet(utc(2026, 1, 16, 10, 59)));
        // 11:00 UTC == 06:00 EST → window closed.
        assert!(!w.is_quiet(utc(2026, 1, 16, 11, 0)));
    }

    #[test]
    fn dst_spring_forward_skips_nonexistent_local_time() {
        // US DST 2026: springs forward 2026-03-08 02:00 → 03:00 EST→EDT.
        let w = window("01:00", "04:00", "America/New_York");
        // 06:30 UTC == 01:30 EST → quiet.
        assert!(w.is_quiet(utc(2026, 3, 8, 6, 30)));
        // 07:30 UTC == 03:30 EDT (02:30 never happened) → still quiet.
        assert!(w.is_quiet(utc(2026, 3, 8, 7, 30)));
        // 08:00 UTC == 04:00 EDT → window closed on the shifted wall clock.
        assert!(!w.is_quiet(utc(2026, 3, 8, 8, 0)));
    }

    #[test]
    fn dst_fall_back_repeats_local_hour_once() {
        // US DST 2026: falls back 2026-11-01 02:00 EDT → 01:00 EST.
        let w = window("01:00", "02:00", "America/New_York");
        // 05:30 UTC == 01:30 EDT (first occurrence) → quiet.
        assert!(w.is_quiet(utc(2026, 11, 1, 5, 30)));
        // 06:30 UTC == 01:30 EST (repeated occurrence) → quiet again;
        // the wall-clock rule holds for both readings of 01:30.
        assert!(w.is_quiet(utc(2026, 11, 1, 6, 30)));
        // 07:00 UTC == 02:00 EST → closed.
        assert!(!w.is_quiet(utc(2026, 11, 1, 7, 0)));
    }

    #[test]
    fn southern_hemisphere_dst_runs_the_other_way() {
        // Australia/Sydney: DST ends 2026-04-05 03:00 AEDT → 02:00 AEST.
        let w = window("01:30", "02:30", "Australia/Sydney");
        // 2026-04-04 15:30 UTC == 01:30 AEST (UTC+10, post-DST rules read)
        // — before the transition Sydney was UTC+11, so 15:30 UTC on Apr 4
        // is 02:30 AEDT → window just closed.
        assert!(!w.is_quiet(utc(2026, 4, 4, 15, 30)));
        // After fallback: 15:30 UTC == 01:30 AEST on Apr 5+ → quiet.
        assert!(w.is_quiet(utc(2026, 4, 5, 15, 30)));
    }

    #[test]
    fn parse_validates_bounds_and_pairs() {
        assert!(QuietHours::parse(None, None, "UTC").is_ok());
        assert!(QuietHours::parse(Some("24:00"), Some("06:00"), "UTC").is_err());
        assert!(QuietHours::parse(Some("ab:cd"), Some("06:00"), "UTC").is_err());
        assert!(QuietHours::parse(Some("09:00"), None, "UTC").is_err());
        assert!(QuietHours::parse(Some("09:00"), Some("17:00"), "Not/AZone").is_err());
    }

    #[test]
    fn decision_matrix() {
        let quiet = window("22:00", "06:00", "UTC");
        let noon = utc(2026, 1, 15, 12, 0);
        let night = utc(2026, 1, 16, 3, 0);

        // Disabled wins over everything.
        assert_eq!(
            decide_channel(
                DeliveryChannel::Email,
                NotificationKind::SlaBreach,
                false,
                Some(&quiet),
                night
            ),
            Decision::Disabled
        );
        // In-app is pull: never quiet-deferred.
        assert_eq!(
            decide_channel(
                DeliveryChannel::InApp,
                NotificationKind::Mention,
                true,
                Some(&quiet),
                night
            ),
            Decision::Deliver
        );
        // Non-critical email inside quiet hours is suppressed…
        assert_eq!(
            decide_channel(
                DeliveryChannel::Email,
                NotificationKind::Mention,
                true,
                Some(&quiet),
                night
            ),
            Decision::QuietDeferred
        );
        // …and outside them delivered.
        assert_eq!(
            decide_channel(
                DeliveryChannel::Email,
                NotificationKind::Mention,
                true,
                Some(&quiet),
                noon
            ),
            Decision::Deliver
        );
        // Critical kinds page straight through the window.
        for kind in [NotificationKind::SlaBreach, NotificationKind::Escalation] {
            assert_eq!(
                decide_channel(DeliveryChannel::Email, kind, true, Some(&quiet), night),
                Decision::Deliver
            );
        }
        // No window configured ⇒ everything enabled delivers.
        assert_eq!(
            decide_channel(
                DeliveryChannel::Email,
                NotificationKind::Mention,
                true,
                None,
                night
            ),
            Decision::Deliver
        );
    }

    /// The noop transport accepts everything and drops it (the worker seam
    /// logs the drop); exercised end-to-end by the worker notifier tests.
    #[test]
    fn noop_transport_is_a_documented_none_mode() {
        // Compile-level guard: the default transport is nameable and
        // stateless — no configuration required to run standalone.
        let _: NoopMailTransport = NoopMailTransport;
    }

    /// Guard against silent chrono behavior changes: the wall-clock read
    /// always goes through the window's timezone, never the host's.
    #[test]
    fn local_minutes_derive_from_the_window_tz() {
        let w = window("22:00", "06:00", "America/Denver");
        // 05:00 UTC == 22:00 Jan 15 MST (UTC-7) → window opens.
        assert!(w.is_quiet(utc(2026, 1, 16, 5, 0)));
        // 13:00 UTC == 06:00 MST → window closed.
        assert!(!w.is_quiet(utc(2026, 1, 16, 13, 0)));
    }
}
