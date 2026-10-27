# ADR 0026: The installer's deployment target, and what `--apply` writes

Date: 2026-10-27 · Status: Accepted · Lane: p60-descriptor-consequences (W60-1)

## Context

Wave 60 builds an installer: host detection, a plan, `--apply`, `marula doctor`,
a wizard over the same data, a released artifact, an update engine and a
verification pass. Five later lanes branch from the descriptor this lane
defines, and the wave names five questions that each of them would otherwise
answer differently. They are settled here because each one changes an interface
or a release path rather than a single screen, and a wrong answer is expensive
to unwind once an artifact exists.

The descriptor already states every lever with its default, and it now states
what each lever does to an operator. That makes it the only input an installer
needs for its explanations, and the only place a target can read what a
deployment must contain.

## Decision

### 1. The target interface is four verbs over a data plan

A `DeploymentTarget` in the core CLI crate, implemented once per shape:

```
detect()    -> HostFacts      // cores, RAM, free disk, Docker, GPU
plan(&Selection) -> Plan      // what would be written and run, with a reason per step
apply(&Plan) -> Applied       // perform the steps, report each outcome
status()    -> Status         // what is running now, and what is missing
```

`Selection` is a profile plus parameter overrides, resolved against the
descriptor: the profile's `sets` flattened over the catalogue defaults. A
selection that names a lever the descriptor does not publish is refused.

`Plan` is data: a list of file writes (path and bytes) and command steps, each
carrying the reason it exists and the descriptor field that justifies it.
`--dry-run` prints that object and writes nothing. `--apply` builds the same
object and executes it. One code path, so a dry run cannot describe something
the apply would not do, and a negative test can assert the plan itself rather
than a screenshot.

The compose target is the first implementation. The binary and systemd target
is the second, behind the same trait, and may land later. A Kubernetes target
implements `detect`, `plan` and `status`; its `apply` is described below.

The trait lives in marula-core, not in the commercial control plane. The
installer ships to people who clone this repository, and an interface defined
outside it would make the core depend on a consumer.

### 2. `--apply` executes one target, never two

Compose is the shipped shape, so on a host where compose is the detected target
`--apply` writes the environment file and runs `docker compose up -d`, then
waits for the health endpoints the profile names.

On a Kubernetes host, `--apply` writes the values file and prints the helm
command. It does not run `helm upgrade`, because the cluster's release name,
namespace, RBAC and rollback policy belong to whoever operates the cluster, and
a tool that guesses them turns a failed upgrade into an ambiguous one.

A host is one target. There is no mode that writes compose files and helm values
in the same run: two writes under one command leave a partial failure with no
obvious state, and the second write is never needed on the same machine.

### 3. A host between two profiles is scored by floors, not by a guess

A profile fits when measured cores, RAM and free disk each meet or exceed its
`floors`. Among the fitting profiles the installer prefers `split`, because it
is the shipped topology and the one a growing install wants; `all-in-one` is
chosen when `split` does not fit.

When no profile fits, the installer reports the shortfall in numbers, names the
profile that came closest, and continues with `all-in-one` if the only missing
component is RAM for the embedding model (see 4). It never rounds a host up, and
it never presents a profile whose floors the host misses as the recommendation.

The numbers are a lower bound rather than a measurement of capacity: floors
answer "can this shape run here", and the capacity model answers "how many
users". W60-6 measures whether the shipped floors are right and raises them with
evidence if they are not.

### 4. When no local embedding model fits, cloud is the offered default

R14 ships a local default and R13 measured it, so a host that cannot run the
local model must be told what it missed, not handed a smaller model. The
embedding step preselects a cloud provider with the smallest cloud entry, prints
the local floor in cores and megabytes beside the host's measured figures, and
requires an instance key before the wizard can continue.

The descriptor already permits this combination: a cloud model carries no local
floor, so the floors check passes for a small host that selects one. Picking the
unmeasured smaller local model instead would quietly trade recall for memory,
and the operator would never see the trade.

### 5. The desktop client keeps its own release pipeline

The core release workflow publishes images and the chart for a version tag, and
W60-4's installer artifact joins that workflow. The desktop bundle does not. It
needs a Tauri updater key and Apple notarisation credentials
(docs/desktop-release.md), which are a different release ceremony from an image
digest and a chart, and mixing them would put a notarisation secret in the path
of every core release.

Both consume the same version tag, so a release still has one number. W61-1
settles the wrapper and the desktop update channel.

## Consequences

- W60-2 builds the trait, `detect`, `plan` and the compose target, and its
  `--dry-run` output is the plan object rather than prose.
- W60-3 renders `consequence`, `default_rationale`, `requires` and `conflicts`
  from the descriptor. A screen that needs a sentence the descriptor lacks is a
  descriptor bug, and the check is where it is caught.
- W60-4 packages the installer into the existing release workflow when that
  workflow is switched on; nothing about the desktop pipeline changes.
- W60-5's update engine reuses `plan` and `apply`, so an update and an install
  produce the same kind of artifact and the same kind of failure report.
- W60-6's negative cases are the descriptor check's violation classes plus the
  apply refusals: a selection naming an unknown lever, a profile under its
  floors, a cloud provider with no key, and a second target on one host.
- `schema_version` is 2 from this landing. A consumer that validated 1 strictly
  must accept the four new parameter fields and `floors` on each profile.
