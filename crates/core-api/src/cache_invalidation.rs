//! Turns contract events into cache invalidations.
//!
//! Called from `realtime::listener_session` for every `marula_events`
//! notification, before the frame reaches the hub. Every replica runs its own
//! listener and its own cache, and `LISTEN`/`NOTIFY` is broadcast to all of
//! them, so one mutation invalidates every replica without a shared store.
//!
//! The mapping belongs here rather than inside the hub because it is a policy
//! statement about what each event means for cached state, and it should be
//! readable in one place with its tests.
//!
//! A frame carries `event_type`, `org_id`, `entity` and `entity_id`. That is
//! enough for every case below. It is *not* enough to map a role to the users
//! holding it, which is why `role.*` flushes the org rather than a key.

use crate::cache::{self, Caches};
use crate::realtime::StreamFrame;
use uuid::Uuid;

/// Applies one frame's invalidations. Safe to call for every frame, including
/// presence frames and events with no cached state.
pub fn invalidate_for(frame: &StreamFrame) {
    let caches = cache::caches();
    match frame.event_type.as_str() {
        // --- identity ---------------------------------------------------------
        // The principal is keyed by `(org, user)`, and these events name the
        // user in `entity_id`, so the flush is exactly one entry.
        "user.created" | "user.invited" | "user.activated" | "user.deactivated"
        | "user.role_changed" => {
            match frame
                .entity_id
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok())
            {
                Some(user_id) => caches.principal.invalidate(&(frame.org_id, user_id)),
                // A malformed or absent id must not leave a stale principal
                // behind, so fall back to the org-scoped flush.
                None => caches.invalidate_org(frame.org_id),
            }
        }
        // A role event names a role, and this process has no role-to-users map.
        // The org flush is bounded, correct, and cheaper than the query these
        // caches exist to remove.
        other if other.starts_with("role.") => caches.invalidate_org(frame.org_id),

        // --- reference families ----------------------------------------------
        // Each family caches under `"<namespace>|<org>"`, so one event clears
        // exactly one family in exactly one org.
        other if other.starts_with("org.") || other == "setting.updated" => {
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_ORG_SETTINGS, frame.org_id));
            // A settings write can also move values cached under the AI family
            // (`ai.features_enabled` and its kin are settings-backed), and the
            // write path that owns them does not say so on the frame. Clearing
            // both is cheap here and removes the two-writer hole entirely.
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_AI, frame.org_id));
        }
        other if other.starts_with("board.") => {
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_BOARD, frame.org_id));
        }
        other if other.starts_with("custom_field.") => {
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_CUSTOM_FIELD, frame.org_id));
        }
        "sla.policy_updated"
        | "sla.calendar_updated"
        | "calendar_exclusion.updated"
        | "on_call.updated"
        | "availability.updated" => {
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_SLA, frame.org_id));
        }
        // Written by the decision-features path, which stores an ORG SETTING
        // (`ai.features_enabled`) rather than AI reference data. Its event is
        // still `ai.*`, so without this arm the org-settings projection would
        // serve the old value until its TTL. Both namespaces clear, because the
        // same write can move cached AI state too.
        "ai.decision_state_changed" => {
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_ORG_SETTINGS, frame.org_id));
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_AI, frame.org_id));
        }
        other if other.starts_with("ai.") => {
            caches.invalidate_refs(&Caches::ref_key(Caches::NS_AI, frame.org_id));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// The cache is process-wide, so cases in this module that insert into it
    /// must not interleave. Locking here keeps them serial inside the binary.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn only_one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn frame(event_type: &str, org_id: Uuid, entity_id: Option<&str>) -> StreamFrame {
        StreamFrame {
            event_type: event_type.to_string(),
            org_id,
            entity: "user".to_string(),
            entity_id: entity_id.map(|s| s.to_string()),
            seq: Some(1),
            at: Utc::now(),
            viewers: None,
            id: Some(Uuid::new_v4()),
        }
    }

    fn sample_principal(org: Uuid, user: Uuid) -> core_domain::entities::Principal {
        let now = Utc::now();
        core_domain::entities::Principal {
            user: core_domain::entities::User {
                id: user,
                org_id: org,
                email: "cache-test@example.test".into(),
                display_name: "Cache Test".into(),
                timezone: "UTC".into(),
                is_active: true,
                mfa_totp_secret: None,
                created_at: now,
                updated_at: now,
            },
            roles: vec![],
            permissions: vec![],
        }
    }

    #[test]
    fn a_user_event_drops_only_that_principal() {
        let _guard = only_one_at_a_time();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let (mine, theirs) = (Uuid::new_v4(), Uuid::new_v4());
        let caches = cache::caches();
        caches
            .principal
            .insert((org, mine), sample_principal(org, mine));
        caches
            .principal
            .insert((org, theirs), sample_principal(org, theirs));

        invalidate_for(&frame("user.role_changed", org, Some(&mine.to_string())));

        assert!(
            caches.principal.get(&(org, mine)).is_none(),
            "the named user's principal is gone"
        );
        assert!(
            caches.principal.get(&(org, theirs)).is_some(),
            "an unrelated user in the same org is untouched"
        );
    }

    #[test]
    fn a_role_event_flushes_the_whole_org() {
        let _guard = only_one_at_a_time();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let other_org = Uuid::new_v4();
        let (in_org, in_other) = (Uuid::new_v4(), Uuid::new_v4());
        let caches = cache::caches();
        caches
            .principal
            .insert((org, in_org), sample_principal(org, in_org));
        caches
            .principal
            .insert((other_org, in_other), sample_principal(other_org, in_other));

        invalidate_for(&frame("role.restored", org, None));

        assert!(
            caches.principal.get(&(org, in_org)).is_none(),
            "org cleared"
        );
        assert!(
            caches.principal.get(&(other_org, in_other)).is_some(),
            "another org's cache survives"
        );
    }

    #[test]
    fn a_user_event_with_no_usable_id_falls_back_to_the_org_flush() {
        let _guard = only_one_at_a_time();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let user = Uuid::new_v4();
        let caches = cache::caches();
        caches
            .principal
            .insert((org, user), sample_principal(org, user));

        invalidate_for(&frame("user.deactivated", org, Some("not-a-uuid")));

        assert!(caches.principal.get(&(org, user)).is_none());
    }

    #[test]
    fn an_unrelated_event_touches_nothing() {
        let _guard = only_one_at_a_time();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let user = Uuid::new_v4();
        let caches = cache::caches();
        caches
            .principal
            .insert((org, user), sample_principal(org, user));

        invalidate_for(&frame("ticket.updated", org, None));

        assert!(
            caches.principal.get(&(org, user)).is_some(),
            "a ticket event is not ours to invalidate"
        );
    }

    #[test]
    fn a_family_event_clears_only_its_own_namespace() {
        let _guard = only_one_at_a_time();
        cache::__enable_for_tests();
        let org = Uuid::new_v4();
        let other_org = Uuid::new_v4();
        let caches = cache::caches();
        let board_key = Caches::ref_key(Caches::NS_BOARD, org);
        let sla_key = Caches::ref_key(Caches::NS_SLA, org);
        let other_board_key = Caches::ref_key(Caches::NS_BOARD, other_org);
        caches.put_ref(board_key.clone(), "statuses".to_string());
        caches.put_ref(sla_key.clone(), "policies".to_string());
        caches.put_ref(other_board_key.clone(), "statuses".to_string());

        invalidate_for(&frame("board.status_updated", org, Some("board-1")));

        assert!(caches.get_ref::<String>(&board_key).is_none(), "board gone");
        assert!(
            caches.get_ref::<String>(&sla_key).is_some(),
            "a sibling family in the same org survives"
        );
        assert!(
            caches.get_ref::<String>(&other_board_key).is_some(),
            "another org's boards survive"
        );
    }

    #[test]
    fn the_sla_family_ignores_its_firing_events() {
        let _guard = only_one_at_a_time();
        cache::__enable_for_tests();
        let org = Uuid::new_v4();
        let caches = cache::caches();
        let key = Caches::ref_key(Caches::NS_SLA, org);
        caches.put_ref(key.clone(), "policies".to_string());

        // `sla.escalation_fired` is an outcome, not a definition change, so the
        // cached policy set is still correct afterwards.
        invalidate_for(&frame("sla.escalation_fired", org, None));

        assert!(caches.get_ref::<String>(&key).is_some());
    }

    #[test]
    fn a_decision_features_write_clears_the_org_settings_projection() {
        let _guard = only_one_at_a_time();
        cache::__enable_for_tests();
        let org = Uuid::new_v4();
        let caches = cache::caches();
        let settings_key = Caches::ref_key(Caches::NS_ORG_SETTINGS, org);
        let ai_key = Caches::ref_key(Caches::NS_AI, org);
        caches.put_ref(settings_key.clone(), "effective-settings".to_string());
        caches.put_ref(ai_key.clone(), "catalog".to_string());

        // The decision-features write stores `ai.features_enabled`, which the
        // org-settings projection serves, but its event is `ai.*`. The
        // projection must not be left stale by that mismatch.
        invalidate_for(&frame("ai.decision_state_changed", org, None));

        assert!(
            caches.get_ref::<String>(&settings_key).is_none(),
            "the org-settings projection is cleared by an org-setting write"
        );
        assert!(
            caches.get_ref::<String>(&ai_key).is_none(),
            "and the AI family"
        );
    }

    #[test]
    fn a_settings_write_clears_both_the_projection_and_the_ai_family() {
        let _guard = only_one_at_a_time();
        cache::__enable_for_tests();
        let org = Uuid::new_v4();
        let caches = cache::caches();
        let settings_key = Caches::ref_key(Caches::NS_ORG_SETTINGS, org);
        let ai_key = Caches::ref_key(Caches::NS_AI, org);
        caches.put_ref(settings_key.clone(), "effective-settings".to_string());
        caches.put_ref(ai_key.clone(), "catalog".to_string());

        invalidate_for(&frame("setting.updated", org, Some("ai.features_enabled")));

        assert!(caches.get_ref::<String>(&settings_key).is_none());
        assert!(
            caches.get_ref::<String>(&ai_key).is_none(),
            "settings-backed AI values share the write path that owns them"
        );
    }
}
