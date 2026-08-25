//! In-memory presence: which users currently have live hub connections,
//! tracked per org with connection refcounting.
//!
//! Purpose: answer "who is online in this org right now" for the presence
//! protocol message. Multi-tab users stay present until their last
//! connection drops.
//!
//! Non-purpose: no persistence, no cross-process sharing — a hub restart
//! clears presence, and horizontal scaling of hubs would need a shared
//! store (deliberately deferred).

use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Org → user → open-connection count.
#[derive(Debug, Default)]
pub struct PresenceMap {
    orgs: Mutex<HashMap<Uuid, HashMap<Uuid, usize>>>,
}

impl PresenceMap {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one more connection for `user` in `org`, returning the
    /// user's new total connection count in that org.
    pub fn join(&self, org: Uuid, user: Uuid) -> usize {
        let mut orgs = self.lock();
        let count = orgs.entry(org).or_default().entry(user).or_insert(0);
        *count += 1;
        *count
    }

    /// Records one closed connection for `user` in `org`, returning the
    /// remaining count (zero once the user is fully offline there).
    pub fn leave(&self, org: Uuid, user: Uuid) -> usize {
        let mut orgs = self.lock();
        let mut remaining = 0;
        if let Some(users) = orgs.get_mut(&org) {
            if let Some(count) = users.get_mut(&user) {
                *count = count.saturating_sub(1);
                remaining = *count;
                if remaining == 0 {
                    users.remove(&user);
                }
            }
            if users.is_empty() {
                orgs.remove(&org);
            }
        }
        remaining
    }

    /// Sorted ids of users with at least one live connection in `org`.
    pub fn users_in_org(&self, org: Uuid) -> Vec<Uuid> {
        self.lock()
            .get(&org)
            .map(|users| {
                let mut ids: Vec<Uuid> = users.keys().copied().collect();
                ids.sort_unstable();
                ids
            })
            .unwrap_or_default()
    }

    /// Lock helper that recovers from poisoning: presence counts are always
    /// internally consistent, so a poisoned mutex carries no bad state.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, HashMap<Uuid, usize>>> {
        self.orgs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refcounts_multiple_connections_per_user() {
        let presence = PresenceMap::new();
        let (org, user) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(presence.join(org, user), 1);
        assert_eq!(presence.join(org, user), 2);
        assert_eq!(presence.users_in_org(org), vec![user]);
        assert_eq!(presence.leave(org, user), 1);
        assert_eq!(presence.leave(org, user), 0);
        assert!(presence.users_in_org(org).is_empty());
    }

    #[test]
    fn leave_is_safe_without_a_prior_join() {
        let presence = PresenceMap::new();
        assert_eq!(presence.leave(Uuid::now_v7(), Uuid::now_v7()), 0);
    }

    #[test]
    fn orgs_are_isolated() {
        let presence = PresenceMap::new();
        let (org_a, org_b, shared_user) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        presence.join(org_a, shared_user);
        assert_eq!(presence.leave(org_b, shared_user), 0);
        assert_eq!(presence.users_in_org(org_b).len(), 0);
        assert_eq!(presence.leave(org_a, shared_user), 0);
    }

    #[test]
    fn users_are_listed_sorted() {
        let presence = PresenceMap::new();
        let org = Uuid::now_v7();
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        presence.join(org, second);
        presence.join(org, first);
        let expected = if first < second {
            vec![first, second]
        } else {
            vec![second, first]
        };
        assert_eq!(presence.users_in_org(org), expected);
    }
}
