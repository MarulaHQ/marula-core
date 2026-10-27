//! The compose target (Wave 60 lane W60-2).
//!
//! The shipped topology. `--apply` writes one env file and runs
//! `docker compose up -d --wait`, which returns only when the healthchecks the
//! profile names are passing. Nothing else is written: the compose file, the
//! pooler config and the images come from the release, and this target's job is
//! the environment that binds them together.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::deploy::{DeployDescriptor, Profile};

use super::host::HostFacts;
use super::plan::{missing_for, render_env, Plan, PlanError, Selection, Step, StepKind};
use super::target::{Applied, DeploymentTarget, Status};

/// The compose target.
#[derive(Debug, Clone)]
pub struct ComposeTarget {
    /// The descriptor the plan explains itself from.
    pub descriptor: DeployDescriptor,
    /// The compose file this target runs.
    pub compose_file: PathBuf,
}

impl ComposeTarget {
    #[must_use]
    pub fn new(descriptor: DeployDescriptor, compose_file: impl Into<PathBuf>) -> Self {
        Self {
            descriptor,
            compose_file: compose_file.into(),
        }
    }

    /// The compose overlays a profile needs beyond the base file.
    ///
    /// `all-in-one` is one process rather than two, and the repository ships
    /// that shape as an override (`deploy/docker-compose.all-in-one.yml`). A
    /// plan that started only the base file would run the split topology under
    /// an `MARULA_ALL_IN_ONE=1` environment, which is neither shape.
    fn overlays(&self, profile: &Profile) -> Vec<PathBuf> {
        let base = self.compose_file.parent().unwrap_or_else(|| Path::new("."));
        match profile.name {
            "all-in-one" => vec![base.join("docker-compose.all-in-one.yml")],
            _ => Vec::new(),
        }
    }

    /// The services this profile starts, by name.
    ///
    /// The profile's own services, the infrastructure each one declares it
    /// needs, and the two compose services every install runs: `migrate` (the
    /// schema one-shot) and `web` (the browser client). Naming them is not a
    /// convenience: `all-in-one` replaces the split worker with loops inside
    /// core-api, so starting the merged file's `worker` as well gives two
    /// processes fighting for port 8082. The override's own header says to
    /// start only its listed services, and this is that list.
    fn service_list(&self, profile: &Profile) -> Vec<String> {
        let mut services: Vec<String> = Vec::new();
        let mut push = |name: &str| {
            if !services.iter().any(|kept| kept == name) {
                services.push(name.to_string());
            }
        };
        for service in &profile.services {
            push(service.name);
            for need in &service.needs {
                push(need);
            }
        }
        push("migrate");
        push("web");
        services
    }

    /// The profile the selection names, or a typed refusal.
    fn profile(&self, selection: &Selection) -> Result<&Profile, PlanError> {
        self.descriptor
            .profiles
            .iter()
            .find(|profile| profile.name == selection.profile)
            .ok_or_else(|| PlanError::UnknownProfile {
                asked: selection.profile.clone(),
                known: self
                    .descriptor
                    .profiles
                    .iter()
                    .map(|profile| profile.name.to_string())
                    .collect(),
            })
    }
}

impl DeploymentTarget for ComposeTarget {
    fn name(&self) -> &'static str {
        "compose"
    }

    fn available(&self, host: &HostFacts) -> Result<(), PlanError> {
        if host.docker.is_none() {
            return Err(PlanError::TargetUnavailable {
                target: "compose",
                reason: "the Docker server did not answer `docker version`".to_string(),
            });
        }
        if !self.compose_file.exists() {
            return Err(PlanError::TargetUnavailable {
                target: "compose",
                reason: format!("{} is not here", self.compose_file.display()),
            });
        }
        Ok(())
    }

    fn plan(&self, host: &HostFacts, selection: &Selection) -> Result<Plan, PlanError> {
        // The descriptor is the plan's only source of truth, so a build whose
        // descriptor contradicts itself cannot produce a plan. The check is the
        // same one `deploy describe --json` refuses on (Wave 60 lane W60-1).
        let violations = crate::deploy_check::check(&self.descriptor);
        if !violations.is_empty() {
            return Err(PlanError::Descriptor(
                violations
                    .iter()
                    .map(|violation| violation.sentence())
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        let profile = self.profile(selection)?;
        let missing = missing_for(profile, host);
        if !missing.is_empty() {
            return Err(PlanError::FloorsNotMet {
                profile: profile.name.to_string(),
                missing,
            });
        }
        for name in selection.overrides.keys() {
            if !self
                .descriptor
                .params
                .iter()
                .any(|param| param.name == name)
            {
                return Err(PlanError::UnknownLever { name: name.clone() });
            }
        }

        let env_path = selection.dir.join("marula.env");
        // Reuse what an earlier apply wrote. Rotating a signing key or the
        // database password on a re-run would be a different, worse operation
        // than the one the operator asked for.
        let existing = std::fs::read_to_string(&env_path)
            .map(|text| crate::install::doctor::parse_env_file(&text))
            .unwrap_or_default();
        let resolved =
            crate::install::plan::resolve_required(&self.descriptor, selection, &existing);
        let mut steps = vec![Step {
            kind: StepKind::WriteFile {
                path: env_path.clone(),
                contents: render_env(&self.descriptor, profile, selection, &resolved),
            },
            reason: "the environment every service reads: the profile's presets, your overrides, and the four values the process refuses to boot without".to_string(),
            source: format!("profiles.{}.sets, params.*", profile.name),
        }];
        let mut args = vec![
            "compose".to_string(),
            "--env-file".to_string(),
            env_path.display().to_string(),
            "-f".to_string(),
            self.compose_file.display().to_string(),
        ];
        for overlay in self.overlays(profile) {
            if overlay.exists() {
                args.push("-f".to_string());
                args.push(overlay.display().to_string());
            }
        }
        args.extend(["up".to_string(), "-d".to_string(), "--wait".to_string()]);
        args.extend(self.service_list(profile));
        steps.push(Step {
            kind: StepKind::Run {
                program: "docker".to_string(),
                args,
            },
            reason: "starts the profile's services and returns only when their healthchecks pass"
                .to_string(),
            source: format!("profiles.{}.services", profile.name),
        });

        Ok(Plan {
            target: self.name(),
            profile: profile.name.to_string(),
            facts: host.clone(),
            steps,
            manual: manual_items(profile),
            secret_names: secret_names(&self.descriptor),
        })
    }

    fn apply(&self, plan: &Plan) -> Applied {
        let mut done = Vec::new();
        for step in &plan.steps {
            match &step.kind {
                StepKind::WriteFile { path, contents } => {
                    if let Err(error) = write_file(path, contents) {
                        return Applied {
                            done,
                            failed: Some(format!("write {}: {error}", path.display())),
                            manual: plan.manual.clone(),
                        };
                    }
                    done.push(format!("wrote {}", path.display()));
                }
                StepKind::Run { program, args } => {
                    let output = Command::new(program).args(args).output();
                    match output {
                        Ok(output) if output.status.success() => {
                            done.push(format!("{program} {}", args.join(" ")));
                        }
                        Ok(output) => {
                            let tail = String::from_utf8_lossy(&output.stderr)
                                .lines()
                                .rev()
                                .take(3)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join(" / ");
                            return Applied {
                                done,
                                failed: Some(format!(
                                    "{program} {} exited {}: {tail}",
                                    args.join(" "),
                                    output.status
                                )),
                                manual: plan.manual.clone(),
                            };
                        }
                        Err(error) => {
                            return Applied {
                                done,
                                failed: Some(format!("could not run {program}: {error}")),
                                manual: plan.manual.clone(),
                            };
                        }
                    }
                }
            }
        }
        Applied {
            done,
            failed: None,
            manual: plan.manual.clone(),
        }
    }

    fn status(&self) -> Status {
        let output = Command::new("docker")
            .args([
                "compose",
                "-f",
                &self.compose_file.display().to_string(),
                "ps",
                "--format",
                "{{.Service}} {{.State}}",
            ])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let services: Vec<String> = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(str::to_string)
                    .collect();
                Status {
                    target: self.name(),
                    services,
                    error: None,
                }
            }
            Ok(output) => Status {
                target: self.name(),
                services: Vec::new(),
                error: Some(format!("docker compose ps exited {}", output.status)),
            },
            Err(error) => Status {
                target: self.name(),
                services: Vec::new(),
                error: Some(format!("could not run docker: {error}")),
            },
        }
    }
}

/// Every name whose value is a credential: the descriptor's secrets and the
/// values the process refuses to boot without, plus the four infrastructure
/// values the compose file requires.
fn secret_names(descriptor: &DeployDescriptor) -> Vec<String> {
    let mut names: Vec<String> = descriptor
        .params
        .iter()
        .filter(|param| param.secret || param.required)
        .map(|param| param.name.to_string())
        .collect();
    names.extend(
        crate::install::plan::infra_requirements()
            .iter()
            .map(|(name, _)| (*name).to_string()),
    );
    names
}

fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// What the installer deliberately leaves to a person, named rather than
/// implied (W60-3 renders the same list).
fn manual_items(profile: &Profile) -> Vec<String> {
    let mut items = vec![
        "a TLS certificate and the public hostname it is issued for".to_string(),
        "the backup destination and its retention window".to_string(),
        "an alert receiver, if you want one".to_string(),
    ];
    if profile.sets.contains_key("MARULA_ALL_IN_ONE") {
        items.push("a second machine before the box has to scale past one process".to_string());
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy;
    use crate::install::plan::recommend;

    fn facts(cores: u16, ram_mb: u32, disk_gb: u32, docker: bool) -> HostFacts {
        HostFacts {
            cores,
            ram_mb: Some(ram_mb),
            free_disk_gb: Some(disk_gb),
            docker: docker.then(|| "27.0.0".to_string()),
            gpu: None,
        }
    }

    fn target(dir: &Path) -> ComposeTarget {
        std::fs::create_dir_all(dir).expect("temp dir");
        let compose = dir.join("docker-compose.yml");
        std::fs::write(&compose, "services: {}\n").expect("compose file");
        ComposeTarget::new(deploy::descriptor(), compose)
    }

    #[test]
    fn a_small_host_is_refused_by_the_floors_it_misses() {
        let dir = std::env::temp_dir().join(format!("marula-plan-{}", uuid::Uuid::now_v7()));
        let target = target(&dir);
        let selection = Selection::new("split", &dir);
        let error = target
            .plan(&facts(2, 1024, 5, true), &selection)
            .expect_err("a two-core box cannot run split");
        let message = error.to_string();
        assert!(message.contains("cores"), "{message}");
        assert!(message.contains("memory"), "{message}");
        assert!(message.contains("disk"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_plan_writes_one_env_file_and_runs_compose_with_it() {
        let dir = std::env::temp_dir().join(format!("marula-plan-{}", uuid::Uuid::now_v7()));
        let target = target(&dir);
        let selection = Selection::new("all-in-one", &dir);
        let plan = target
            .plan(&facts(8, 16384, 100, true), &selection)
            .expect("an eight-core box fits");

        let writes = plan.writes();
        assert_eq!(writes.len(), 1, "one file, not a tree of them");
        assert!(writes[0].0.ends_with("marula.env"));
        assert!(
            writes[0].1.contains("MARULA_ALL_IN_ONE=1"),
            "the profile's presets are in the file"
        );
        assert!(
            writes[0].1.contains("# "),
            "every written lever carries its consequence"
        );

        let run = plan
            .steps
            .iter()
            .find_map(|step| match &step.kind {
                StepKind::Run { args, .. } => Some(args.clone()),
                StepKind::WriteFile { .. } => None,
            })
            .expect("a run step");
        assert!(
            run.contains(&"--wait".to_string()),
            "apply waits for health"
        );
        assert!(run.contains(&"--env-file".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_all_in_one_plan_names_its_overlay_and_a_split_plan_does_not() {
        let dir = std::env::temp_dir().join(format!("marula-plan-{}", uuid::Uuid::now_v7()));
        let target = target(&dir);
        // The overlay ships beside the base file; the test creates the same
        // shape so the plan's existence rule is what decides.
        std::fs::write(dir.join("docker-compose.all-in-one.yml"), "services: {}\n")
            .expect("overlay");

        let all_in_one = target
            .plan(
                &facts(8, 16384, 100, true),
                &Selection::new("all-in-one", &dir),
            )
            .expect("a plan");
        let args = all_in_one
            .steps
            .iter()
            .find_map(|step| match &step.kind {
                StepKind::Run { args, .. } => Some(args.clone()),
                StepKind::WriteFile { .. } => None,
            })
            .expect("a run step");
        assert!(
            args.iter()
                .any(|arg| arg.ends_with("docker-compose.all-in-one.yml")),
            "the all-in-one plan must run its overlay: {args:?}"
        );

        let split = target
            .plan(&facts(8, 16384, 100, true), &Selection::new("split", &dir))
            .expect("a plan");
        let split_args = split
            .steps
            .iter()
            .find_map(|step| match &step.kind {
                StepKind::Run { args, .. } => Some(args.clone()),
                StepKind::WriteFile { .. } => None,
            })
            .expect("a run step");
        assert_eq!(
            split_args.iter().filter(|arg| *arg == "-f").count(),
            1,
            "split runs the base file only: {split_args:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_all_in_one_plan_starts_the_services_that_override_names() {
        let dir = std::env::temp_dir().join(format!("marula-plan-{}", uuid::Uuid::now_v7()));
        let target = target(&dir);
        let plan = target
            .plan(
                &facts(8, 16384, 100, true),
                &Selection::new("all-in-one", &dir),
            )
            .expect("a plan");
        let args = plan
            .steps
            .iter()
            .find_map(|step| match &step.kind {
                StepKind::Run { args, .. } => Some(args.clone()),
                StepKind::WriteFile { .. } => None,
            })
            .expect("a run step");
        assert!(
            !args.iter().any(|arg| arg == "worker"),
            "all-in-one must not start the split worker: the loops live in core-api: {args:?}"
        );
        for named in ["core-api", "postgres", "redis", "migrate", "web"] {
            assert!(
                args.iter().any(|arg| arg == named),
                "{named} is missing: {args:?}"
            );
        }

        let split = target
            .plan(&facts(8, 16384, 100, true), &Selection::new("split", &dir))
            .expect("a plan");
        let split_args = split
            .steps
            .iter()
            .find_map(|step| match &step.kind {
                StepKind::Run { args, .. } => Some(args.clone()),
                StepKind::WriteFile { .. } => None,
            })
            .expect("a run step");
        for named in ["core-api", "worker", "pgbouncer", "migrate", "web"] {
            assert!(
                split_args.iter().any(|arg| arg == named),
                "{named} is missing from the split plan: {split_args:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_lever_is_refused_before_anything_is_written() {
        let dir = std::env::temp_dir().join(format!("marula-plan-{}", uuid::Uuid::now_v7()));
        let target = target(&dir);
        let mut selection = Selection::new("all-in-one", &dir);
        selection.set("MARULA_NOT_A_LEVER=1").expect("parsed");
        let error = target
            .plan(&facts(8, 16384, 100, true), &selection)
            .expect_err("an unpublished lever is refused");
        assert!(matches!(error, PlanError::UnknownLever { .. }));
        assert!(!dir.exists() || !dir.join("marula.env").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_without_docker_is_refused_by_the_target_not_the_plan() {
        let dir = std::env::temp_dir().join(format!("marula-plan-{}", uuid::Uuid::now_v7()));
        let target = target(&dir);
        let error = target
            .available(&facts(8, 16384, 100, false))
            .expect_err("no docker, no compose target");
        assert!(error.to_string().contains("Docker"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_recommendation_prefers_split_and_names_what_all_in_one_saves() {
        let desc = deploy::descriptor();
        let big = recommend(&desc, &facts(8, 16384, 100, true));
        assert_eq!(big.profile.as_deref(), Some("split"));
        assert!(big.embedding_fits);

        // Eight cores and 16 GB clear both profiles; the smaller profile exists
        // for a box that cannot host the split shape's services independently.
        let small = recommend(&desc, &facts(4, 8192, 100, true));
        assert_eq!(
            small.profile.as_deref(),
            Some("split"),
            "split's floors are met, so the shipped topology wins"
        );

        // Thirty gigabytes is below split's disk floor, so the one-process
        // shape is the answer even though split's cores and memory are met.
        let low_disk = recommend(&desc, &facts(4, 8192, 30, true));
        assert_eq!(low_disk.profile.as_deref(), Some("all-in-one"));

        let none = recommend(&desc, &facts(1, 512, 2, true));
        assert_eq!(none.profile, None);
        let shortfall = none.shortfall.expect("a shortfall is reported");
        assert!(shortfall.needs.iter().any(|need| need.contains("cores")));
    }
}
