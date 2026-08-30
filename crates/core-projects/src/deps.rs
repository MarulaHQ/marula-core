//! Finish-to-start dependency validation for project tasks.
//!
//! Pure graph logic — no I/O. The route layer loads the project's edge
//! list (core-db `list_project_dep_edges`) plus the candidate mutation and
//! hands it here; a rejected dependency set produces a typed 409
//! `task_dependencies_unsatisfied`, never a silent partial write.

use core_domain::DomainError;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A rejected dependency mutation: names the offending edge so the client
/// can flag the exact task without a second round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRejection {
    /// The task whose dependency set was rejected.
    pub task_id: Uuid,
    /// The dependency edge that caused the rejection.
    pub depends_on: Uuid,
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    /// The edge's two endpoints live in different projects (or one side
    /// does not exist / is invisible in scope).
    CrossProject,
    /// A task cannot depend on itself.
    SelfDependency,
    /// Adding the edge would create a cycle through the existing graph.
    Cycle,
}

impl std::fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CrossProject => f.write_str("cross_project"),
            Self::SelfDependency => f.write_str("self_dependency"),
            Self::Cycle => f.write_str("cycle"),
        }
    }
}

impl std::fmt::Display for DependencyRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "dependency {} -> {} rejected: {}",
            self.task_id, self.depends_on, self.reason
        )
    }
}

impl From<DependencyRejection> for DomainError {
    fn from(r: DependencyRejection) -> Self {
        DomainError::Validation(format!(
            "dependency {} -> {} rejected: {}",
            r.task_id, r.depends_on, r.reason
        ))
    }
}

/// Validates adding ONE edge (`task_id` waits for `depends_on`) against
/// the project's existing edges. `same_project` is the precomputed
/// membership answer (both endpoints exist in the caller's org-scope AND
/// the same project).
pub fn validate_add(
    existing: &[(Uuid, Uuid)],
    task_id: Uuid,
    depends_on: Uuid,
    same_project: bool,
) -> Result<(), DependencyRejection> {
    let reject = |reason: RejectionReason| DependencyRejection {
        task_id,
        depends_on,
        reason,
    };
    if !same_project {
        return Err(reject(RejectionReason::CrossProject));
    }
    if task_id == depends_on {
        return Err(reject(RejectionReason::SelfDependency));
    }
    let mut edges: Vec<(Uuid, Uuid)> = existing.to_vec();
    edges.push((task_id, depends_on));
    if has_cycle(&edges) {
        return Err(reject(RejectionReason::Cycle));
    }
    Ok(())
}

/// Validates REPLACING one task's dependency set (PUT semantics). The new
/// set must be same-project, self-free, and acyclic when grafted onto the
/// untouched edges of the other tasks. Returns the validated set in input
/// order (deduplicated).
pub fn validate_replace(
    existing: &[(Uuid, Uuid)],
    task_id: Uuid,
    new_deps: &[Uuid],
    members: &HashSet<Uuid>,
) -> Result<Vec<Uuid>, DependencyRejection> {
    let mut validated: Vec<Uuid> = Vec::with_capacity(new_deps.len());
    let mut seen: HashSet<Uuid> = HashSet::new();
    // Other tasks' edges survive; this task's own edges are replaced.
    let mut edges: Vec<(Uuid, Uuid)> = existing
        .iter()
        .copied()
        .filter(|(t, _)| *t != task_id)
        .collect();
    for dep in new_deps {
        let reject = |reason: RejectionReason| DependencyRejection {
            task_id,
            depends_on: *dep,
            reason,
        };
        if *dep == task_id {
            return Err(reject(RejectionReason::SelfDependency));
        }
        if !members.contains(dep) || !members.contains(&task_id) {
            return Err(reject(RejectionReason::CrossProject));
        }
        if seen.insert(*dep) {
            validated.push(*dep);
            edges.push((task_id, *dep));
        }
    }
    if has_cycle(&edges) {
        // Name the LAST edge that closed a cycle for a precise 409 body.
        let mut probe: Vec<(Uuid, Uuid)> = existing
            .iter()
            .copied()
            .filter(|(t, _)| *t != task_id)
            .collect();
        for dep in &validated {
            probe.push((task_id, *dep));
            if has_cycle(&probe) {
                return Err(DependencyRejection {
                    task_id,
                    depends_on: *dep,
                    reason: RejectionReason::Cycle,
                });
            }
        }
        // Unreachable: the full-set cycle must reproduce incrementally.
        return Err(DependencyRejection {
            task_id,
            depends_on: task_id,
            reason: RejectionReason::Cycle,
        });
    }
    Ok(validated)
}

/// Kahn's algorithm: `true` when the directed edge list contains a cycle.
pub fn has_cycle(edges: &[(Uuid, Uuid)]) -> bool {
    let mut indegree: HashMap<Uuid, usize> = HashMap::new();
    let mut adjacency: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    let mut nodes: HashSet<Uuid> = HashSet::new();
    for (from, to) in edges {
        nodes.insert(*from);
        nodes.insert(*to);
        adjacency.entry(*from).or_default().push(*to);
        *indegree.entry(*to).or_insert(0) += 1;
        indegree.entry(*from).or_insert(0);
    }
    let mut queue: Vec<Uuid> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut visited = 0_usize;
    while let Some(node) = queue.pop() {
        visited += 1;
        if let Some(neighbors) = adjacency.get(&node) {
            for next in neighbors {
                let d = indegree.get_mut(next).expect("node registered above");
                *d -= 1;
                if *d == 0 {
                    queue.push(*next);
                }
            }
        }
    }
    visited != nodes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> Uuid {
        Uuid::from_u128(u128::from(n))
    }

    #[test]
    fn self_dependency_is_rejected() {
        let t = id(1);
        let err = validate_add(&[], t, t, true).unwrap_err();
        assert_eq!(err.reason, RejectionReason::SelfDependency);
    }

    #[test]
    fn cross_project_edge_is_rejected() {
        let err = validate_add(&[], id(1), id(2), false).unwrap_err();
        assert_eq!(err.reason, RejectionReason::CrossProject);
    }

    #[test]
    fn direct_and_transitive_cycles_are_detected() {
        let (a, b, c) = (id(1), id(2), id(3));
        // Direct two-node cycle.
        let existing = vec![(b, a)];
        let err = validate_add(&existing, a, b, true).unwrap_err();
        assert_eq!(err.reason, RejectionReason::Cycle);
        // Transitive three-node cycle a→b→c→a.
        let existing = vec![(b, c), (c, a)];
        let err = validate_add(&existing, a, b, true).unwrap_err();
        assert_eq!(err.reason, RejectionReason::Cycle);
    }

    #[test]
    fn diamonds_and_chains_are_accepted() {
        let (a, b, c, d) = (id(1), id(2), id(3), id(4));
        // The full diamond d→b→a, d→c→a is acyclic.
        let diamond = vec![(b, a), (c, a), (d, b), (d, c)];
        assert!(!has_cycle(&diamond));
        // Adding the closing edge a→d cycles it.
        assert!(has_cycle(&[diamond.as_slice(), &[(a, d)]].concat()));
        // Extending the acyclic graph stays legal.
        assert!(validate_add(&diamond, a, d, true).is_err());
        let open: Vec<(Uuid, Uuid)> = diamond[..3].to_vec();
        assert!(validate_add(&open, d, c, true).is_ok());
    }

    #[test]
    fn replace_validates_and_dedupes() {
        let (a, b, c, d) = (id(1), id(2), id(3), id(4));
        let existing = vec![(b, a)];
        let members: HashSet<Uuid> = [a, b, c, d].into();
        // Replace b's set with {c, c} → dedupes to {c}, no cycle.
        let out = validate_replace(&existing, b, &[c, c], &members).unwrap();
        assert_eq!(out, vec![c]);
        // Replacing a's set with {b} would create a→b→a cycle.
        let err = validate_replace(&existing, a, &[b], &members).unwrap_err();
        assert_eq!(err.reason, RejectionReason::Cycle);
        assert_eq!(err.depends_on, b);
        // Foreign member rejected as cross-project.
        let err = validate_replace(&existing, a, &[id(99)], &members).unwrap_err();
        assert_eq!(err.reason, RejectionReason::CrossProject);
    }
}
