//! The descriptor's self-check (Wave 60 lane W60-1).
//!
//! Purpose: the deploy descriptor is where an operator learns what a lever
//! does, so every explanation in it has to be present, long enough to say
//! something, and internally consistent. [`check`] reads the descriptor the way
//! a consumer would and reports each contradiction as a sentence a build can
//! print.
//!
//! The three failures the row names are here:
//!
//! - a parameter with no plain-language consequence ([`Violation::NoConsequence`]),
//! - a `requires` (or `conflicts`) naming a lever this catalogue does not have
//!   ([`Violation::UnknownLever`]),
//! - a profile whose presets ask for more machine than its own floors offer
//!   ([`Violation::FloorsForbidProfile`]).
//!
//! Four more rules came with the same shape and cost nothing to hold: a default
//! with no stated reason, a lever that names itself, a conflict one side only
//! declares, and a lever that is both required and forbidden.
//!
//! Non-purpose: this decides nothing about whether a default is a good choice.
//! It proves the descriptor does not argue with itself, which is what a consumer
//! pinning the JSON needs and what a wizard cannot check on its own.

use crate::deploy::{DeployDescriptor, Param, Profile};

/// The shortest consequence that can still carry information. Eight words is a
/// sentence with a subject and an effect; anything shorter is a label, and the
/// descriptor already has a label in `description`.
const MIN_CONSEQUENCE_WORDS: usize = 8;

/// One way the descriptor contradicts itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// A lever with no consequence at all.
    NoConsequence { param: String },
    /// A consequence too short to say what happens.
    ShallowConsequence { param: String, words: usize },
    /// A consequence that repeats the description, so it explains nothing.
    ConsequenceEchoesDescription { param: String },
    /// A default with no stated reason.
    NoDefaultRationale { param: String },
    /// `requires` or `conflicts` names something that is not a lever.
    UnknownLever {
        param: String,
        field: &'static str,
        lever: String,
    },
    /// A lever that requires or conflicts with itself.
    SelfReference { param: String, field: &'static str },
    /// One side declares a conflict the other does not.
    AsymmetricConflict { param: String, lever: String },
    /// A lever that is both required and forbidden.
    ContradictoryEdge { param: String, lever: String },
    /// A profile that declares no machine at all.
    EmptyFloors { profile: String },
    /// A profile whose presets select a local model its floors cannot host.
    FloorsForbidProfile {
        profile: String,
        model: String,
        needs_cores: u16,
        needs_ram_mb: u32,
        has_cores: u16,
        has_ram_mb: u32,
    },
}

impl Violation {
    /// The sentence a build prints. One line, naming the lever and the number.
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            Self::NoConsequence { param } => {
                format!(
                    "{param} has no consequence: an operator cannot tell what setting it changes"
                )
            }
            Self::ShallowConsequence { param, words } => format!(
                "{param} has a {words}-word consequence; it needs at least \
                 {MIN_CONSEQUENCE_WORDS} to name an effect"
            ),
            Self::ConsequenceEchoesDescription { param } => format!(
                "{param} restates its description as its consequence, so it explains nothing new"
            ),
            Self::NoDefaultRationale { param } => {
                format!("{param} has a default and no default_rationale saying why")
            }
            Self::UnknownLever {
                param,
                field,
                lever,
            } => format!("{param} names {lever} in `{field}`, but no such lever exists"),
            Self::SelfReference { param, field } => {
                format!("{param} names itself in `{field}`")
            }
            Self::AsymmetricConflict { param, lever } => format!(
                "{param} conflicts with {lever}, but {lever} does not declare the conflict back"
            ),
            Self::ContradictoryEdge { param, lever } => {
                format!("{param} both requires and conflicts with {lever}")
            }
            Self::EmptyFloors { profile } => format!(
                "profile {profile} declares no cores or memory, so no host can be scored against it"
            ),
            Self::FloorsForbidProfile {
                profile,
                model,
                needs_cores,
                needs_ram_mb,
                has_cores,
                has_ram_mb,
            } => format!(
                "profile {profile} presets the local model {model}, which needs {needs_cores} \
                 cores and {needs_ram_mb} MB, but the profile declares {has_cores} cores and \
                 {has_ram_mb} MB"
            ),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence())
    }
}

/// Every contradiction in `d`, in parameter order and then profile order.
///
/// Empty means the descriptor agrees with itself. Callers that publish the
/// descriptor refuse when this is not empty: a consumer that pins a
/// contradicting contract inherits the contradiction.
#[must_use]
pub fn check(d: &DeployDescriptor) -> Vec<Violation> {
    let mut out = Vec::new();
    let known: std::collections::HashSet<&str> = d.params.iter().map(|p| p.name).collect();

    for param in &d.params {
        check_consequence(param, &mut out);
        if param.default.is_some() && param.default_rationale.is_none() {
            out.push(Violation::NoDefaultRationale {
                param: param.name.to_string(),
            });
        }
        for (field, levers) in [
            ("requires", param.requires.as_slice()),
            ("conflicts", param.conflicts.as_slice()),
        ] {
            for lever in levers {
                if *lever == param.name {
                    out.push(Violation::SelfReference {
                        param: param.name.to_string(),
                        field,
                    });
                } else if !known.contains(lever) {
                    out.push(Violation::UnknownLever {
                        param: param.name.to_string(),
                        field,
                        lever: (*lever).to_string(),
                    });
                }
            }
        }
        for lever in &param.requires {
            if param.conflicts.contains(lever) {
                out.push(Violation::ContradictoryEdge {
                    param: param.name.to_string(),
                    lever: (*lever).to_string(),
                });
            }
        }
        for lever in &param.conflicts {
            let back = d
                .params
                .iter()
                .find(|p| p.name == *lever)
                .is_some_and(|other| other.conflicts.contains(&param.name));
            if !back && known.contains(lever) {
                out.push(Violation::AsymmetricConflict {
                    param: param.name.to_string(),
                    lever: (*lever).to_string(),
                });
            }
        }
    }

    for profile in &d.profiles {
        if profile.floors.cores == 0 || profile.floors.ram_mb == 0 {
            out.push(Violation::EmptyFloors {
                profile: profile.name.to_string(),
            });
            continue;
        }
        if let Some(violation) = profile_floor_violation(profile) {
            out.push(violation);
        }
    }

    out
}

fn check_consequence(param: &Param, out: &mut Vec<Violation>) {
    let consequence = param.consequence.trim();
    if consequence.is_empty() {
        out.push(Violation::NoConsequence {
            param: param.name.to_string(),
        });
        return;
    }
    if consequence.eq_ignore_ascii_case(param.description.trim()) {
        out.push(Violation::ConsequenceEchoesDescription {
            param: param.name.to_string(),
        });
        return;
    }
    let words = consequence.split_whitespace().count();
    if words < MIN_CONSEQUENCE_WORDS {
        out.push(Violation::ShallowConsequence {
            param: param.name.to_string(),
            words,
        });
    }
}

/// Whether the embedding selection a profile presets fits inside the machine
/// the same profile declares.
///
/// The selection is resolved exactly as the boot path resolves it: the model
/// lever when the profile sets one, otherwise the tier's entry for the
/// provider. A cloud model carries no local floor, so it never contradicts a
/// profile — that is the escape hatch a small box uses, and the reason this
/// rule does not read as "every profile needs a GPU".
fn profile_floor_violation(profile: &Profile) -> Option<Violation> {
    use core_domain::embedding::{Catalogue, EmbeddingTier, MODEL_ENV, PROVIDER_ENV, TIER_ENV};

    let tier = profile
        .sets
        .get(TIER_ENV)
        .and_then(|value| EmbeddingTier::parse(value))
        .unwrap_or(EmbeddingTier::Standard);
    let provider = profile.sets.get(PROVIDER_ENV).copied().unwrap_or("ollama");
    let catalogue = Catalogue::builtin();
    let entry = match profile.sets.get(MODEL_ENV) {
        Some(id) => catalogue.find(id),
        None => catalogue.for_tier(tier, provider),
    }?;
    if !entry.floor.is_local() {
        return None;
    }
    let short =
        entry.floor.cores > profile.floors.cores || entry.floor.ram_mb > profile.floors.ram_mb;
    if !short {
        return None;
    }
    Some(Violation::FloorsForbidProfile {
        profile: profile.name.to_string(),
        model: entry.id.to_string(),
        needs_cores: entry.floor.cores,
        needs_ram_mb: entry.floor.ram_mb,
        has_cores: profile.floors.cores,
        has_ram_mb: profile.floors.ram_mb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy;

    #[test]
    fn the_shipped_descriptor_passes() {
        let violations = check(&deploy::descriptor());
        assert!(
            violations.is_empty(),
            "the shipped descriptor contradicts itself:\n{}",
            violations
                .iter()
                .map(|v| format!("  - {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn a_parameter_with_no_consequence_fails() {
        let mut d = deploy::descriptor();
        d.params[0].consequence = "";
        assert_eq!(
            check(&d),
            vec![Violation::NoConsequence {
                param: d.params[0].name.to_string()
            }]
        );
    }

    #[test]
    fn a_consequence_too_short_to_say_anything_fails() {
        let mut d = deploy::descriptor();
        d.params[0].consequence = "Faster.";
        assert_eq!(
            check(&d),
            vec![Violation::ShallowConsequence {
                param: d.params[0].name.to_string(),
                words: 1
            }]
        );
    }

    #[test]
    fn a_consequence_that_repeats_the_description_fails() {
        let mut d = deploy::descriptor();
        let description = d.params[1].description;
        d.params[1].consequence = description;
        assert_eq!(
            check(&d),
            vec![Violation::ConsequenceEchoesDescription {
                param: d.params[1].name.to_string()
            }]
        );
    }

    #[test]
    fn a_requires_naming_a_lever_that_does_not_exist_fails() {
        let mut d = deploy::descriptor();
        d.params[0].requires = vec!["MARULA_NOT_A_LEVER"];
        let violations = check(&d);
        assert_eq!(
            violations,
            vec![Violation::UnknownLever {
                param: d.params[0].name.to_string(),
                field: "requires",
                lever: "MARULA_NOT_A_LEVER".to_string()
            }]
        );
        assert!(violations[0].sentence().contains("no such lever"));
    }

    #[test]
    fn a_conflict_only_one_side_declares_fails() {
        let mut d = deploy::descriptor();
        let other = d.params[2].name;
        d.params[1].conflicts = vec![other];
        let violations = check(&d);
        assert!(
            violations.iter().any(|v| matches!(
                v,
                Violation::AsymmetricConflict { lever, .. } if lever == other
            )),
            "expected an asymmetric-conflict report, got {violations:?}"
        );
    }

    #[test]
    fn a_default_with_no_rationale_fails() {
        let mut d = deploy::descriptor();
        let index = d
            .params
            .iter()
            .position(|p| p.default.is_some())
            .expect("a parameter with a default");
        d.params[index].default_rationale = None;
        assert_eq!(
            check(&d),
            vec![Violation::NoDefaultRationale {
                param: d.params[index].name.to_string()
            }]
        );
    }

    #[test]
    fn a_profile_its_own_floors_forbid_fails() {
        let mut d = deploy::descriptor();
        let profile = d.profiles.first_mut().expect("a profile");
        profile.floors = deploy::ResourceFloors {
            cores: 2,
            ram_mb: 1024,
            disk_gb: 10,
        };
        let violations = check(&d);
        assert_eq!(violations.len(), 1, "{violations:?}");
        match &violations[0] {
            Violation::FloorsForbidProfile {
                needs_cores,
                needs_ram_mb,
                has_cores,
                has_ram_mb,
                ..
            } => {
                assert_eq!((*needs_cores, *needs_ram_mb), (4, 768));
                assert_eq!((*has_cores, *has_ram_mb), (2, 1024));
            }
            other => panic!("expected a floors refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_profile_with_no_floors_fails() {
        let mut d = deploy::descriptor();
        d.profiles[0].floors = deploy::ResourceFloors {
            cores: 0,
            ram_mb: 0,
            disk_gb: 0,
        };
        assert_eq!(
            check(&d),
            vec![Violation::EmptyFloors {
                profile: d.profiles[0].name.to_string()
            }]
        );
    }

    /// A cloud model has no local floor, so the smallest profile may still
    /// select it: that is the path R14 says a host below every local floor
    /// takes, and the check must not close it.
    #[test]
    fn a_cloud_model_fits_a_small_profile() {
        let mut d = deploy::descriptor();
        let profile = d.profiles.first_mut().expect("a profile");
        profile.floors = deploy::ResourceFloors {
            cores: 2,
            ram_mb: 2048,
            disk_gb: 10,
        };
        profile.sets.insert("MARULA_EMBEDDING_PROVIDER", "openai");
        profile
            .sets
            .insert("MARULA_EMBEDDING_MODEL", "text-embedding-3-small");
        let violations = check(&d);
        assert!(violations.is_empty(), "{violations:?}");
    }
}
