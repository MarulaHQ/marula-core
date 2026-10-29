# Docs

Two bodies of documentation live here and they answer to different readers.

| Path | Reader | Published |
|---|---|---|
| [`site/`](site/) | Owners, admins, technicians, client portal users | yes, through the static publisher (ADR 0009) |
| everything else in this directory | people building and running Marula | no |

If you are writing for someone who uses the product, write in `site/` and start
from [`site/README.md`](site/README.md). If you are writing for an engineer, stay
here and add a row to the table for the task below. A new page that no table row
points at is a page nobody will find.

The public documentation index is [`site/index.md`](site/index.md). The task
entry points for each reader are [`site/start/`](site/start/index.md). Symptom
lookup is [`site/troubleshooting.md`](site/troubleshooting.md). Vocabulary is
[`site/reference/glossary.md`](site/reference/glossary.md).

## Understand how it works

| Doc | What it settles |
|---|---|
| [`tech-stack.md`](tech-stack.md) | Language and database choices, the package layout, and the reason for each |
| [`adr/`](adr/) | Every decision that was expensive to reverse, and what it beat |
| [`theme.md`](theme.md) | Visual theme tokens (binding) |
| [`instant-first.md`](instant-first.md) | Interaction rules for optimistic writes and undo (binding) |

## Run an instance

| Doc | Task |
|---|---|
| [`install.md`](install.md) | Install on a machine: the plan, `--apply`, and what stays manual |
| [`all-in-one.md`](all-in-one.md) | Run one process instead of two on a small install |
| [`deploy-descriptor.md`](deploy-descriptor.md) | The machine-readable deploy surface and every runtime parameter |
| [`capacity-model.md`](capacity-model.md) | How many agents one instance serves, from measurements |
| [`worker-cadence.md`](worker-cadence.md) | Which worker loops exist, how often they run, what a small install can trim |
| [`reference/data-lifecycle.md`](reference/data-lifecycle.md) | Retention windows per table and profile, object-store integrity, and what a tenant erasure means for each surface |
| [`reference/rpo-rto.md`](reference/rpo-rto.md) | The recovery numbers each deploy profile actually delivers, how they were derived, and the drill that measures them |
| [`reference/reliability-matrix.md`](reference/reliability-matrix.md) | What each subsystem guarantees, what it does not, how it fails, and which page shows it |
| [`reference/reliability-levers.md`](reference/reliability-levers.md) | Every descriptor lever and the retry budget each preset resolves to (generated) |
| [`perf.md`](perf.md) | Budgets, the measurement harness, the recorded baselines |
| [`upgrade-compatibility-policy.md`](upgrade-compatibility-policy.md) | The N−2 support window and the expand-then-contract migration rule |
| [`release.md`](release.md) | Cut a release, verify a download, and install one on a host with no network |
| [`release-images.md`](release-images.md) | The four application images: how they are named and pinned by digest, and what the release must publish |
| [`update.md`](update.md) | `marula update`: a signed manifest, the sentence a check prints, and what a rollback restores |
| [`desktop-release.md`](desktop-release.md) | Desktop packaging, signing and the auto-update path |
| [`security/staging-runbook.md`](security/staging-runbook.md) | Deploy, seed, rotate, collect evidence for, and tear down the alpha-tester and penetration-test staging instance |
| [`security/pentest-known-issues.md`](security/pentest-known-issues.md) | What the audit already owns, written for testers so they do not report it again |

Operator tasks also live in the site, because operators read the site:
[`site/start/operators.md`](site/start/operators.md) links stand-up, TLS, backup,
upgrade and observability.

## Change a surface

| Doc | Scope |
|---|---|
| [`notes-tasks-workbar.md`](notes-tasks-workbar.md) | The workbar note and task panel specification |
| [`routing-providers.md`](routing-providers.md) | The routing contract, provider shapes and the warning codes |
| [`10-agent-platform.md`](10-agent-platform.md) | The agent harness: decision model, grants, run ledger |
| [`auth-hardening.md`](auth-hardening.md) | MFA, SSO, refresh-token reuse detection and audit completion |
| [`reference/auth-provider-contract.md`](reference/auth-provider-contract.md) | What an auth provider may implement |
| [`reference/plugin-runtime.md`](reference/plugin-runtime.md) | The plugin host in one page: what runs, what refuses, and the map to the rest |
| [`reference/plugin-pdk.md`](reference/plugin-pdk.md) | Writing a plugin: the module contract, the handler, the manifest, build, sign, install |
| [`reference/plugin-host-functions.md`](reference/plugin-host-functions.md) | The nine ABI entries: signatures, gates, in-band refusals, limits and the 25-code vocabulary |
| [`reference/plugin-capabilities.md`](reference/plugin-capabilities.md) | All thirty-five capability keys, the permission each resolves to, and the surface each licenses |
| [`reference/plugin-security-model.md`](reference/plugin-security-model.md) | The trust boundaries, the limits, what a hostile guest can do, and what cannot happen |
| [`reference/plugin-operator-runbook.md`](reference/plugin-operator-runbook.md) | Install, grant, quarantine, roll back and revoke, then triage by symptom |
| [`reference/plugin-agent-guide.md`](reference/plugin-agent-guide.md) | What an agent sees of a plugin, what it may invoke, and what an author owes it |
| [`reference/connection-pooling.md`](reference/connection-pooling.md) | PgBouncer in transaction mode: the connection ceiling, the two settings that make it safe, and the cross-tenant probe that verifies RLS through it |
| [`reference/workflow-node-catalogue.md`](reference/workflow-node-catalogue.md) | Every workflow node the builder offers |
| [`reference/read-model.md`](reference/read-model.md) | The reporting view contract, with per-view pages in [`reference/read-model/`](reference/read-model/) |
| [`reference/engineering-glossary.md`](reference/engineering-glossary.md) | Wave, lane, merge group, gate, honest gap — the shorthand in tracker rows and evidence directories |
| [`reference/core-and-packs.md`](reference/core-and-packs.md) | Where the open core ends and a premium pack begins, and what enforces the line |
| [`agents/README.md`](agents/README.md) | What an agent may reach, what it may never do, and [`agents/manifest.json`](agents/manifest.json) as the machine-readable index |
| [`rn-groundwork.md`](rn-groundwork.md) | What the native app foundation shipped and what it reuses |
| [`e2e-event-type-registry.md`](e2e-event-type-registry.md) | The event-type registry the webhook admin surface reads |
| [`e2e-inbound-webhook-receiver.md`](e2e-inbound-webhook-receiver.md) | Signed inbound webhooks, and the deduplication rule behind the ingest event |

## Publish and check the docs

| Task | Command |
|---|---|
| Read the docs licence and publishing boundary | [`adr/0025-docs-content-license.md`](adr/0025-docs-content-license.md) |
| Read the site conventions and the generated-page rules | [`site/README.md`](site/README.md) |
| Verify links, generated pages, the help manifest and the publish path | `node docs/site/scripts/check-docs-site.mjs` |
| Verify the information architecture (orphans, index coverage, vocabulary) | `node docs/site/scripts/check-docs-ia.mjs` |
| Re-render the static publication | `node docs/site/scripts/publish.mjs` |
| Read the coverage audit the site was built from | [`site/DOCS-COVERAGE.md`](site/DOCS-COVERAGE.md) |

Both checkers run on a clean checkout with Node only. The site-checker invokes
the IA checker, so one command covers both.

## History: read only when tracing a decision

The per-stage build record and the original build-prompt verdict ledger live in
the private sibling repository, not here. Summaries of what shipped and when
belong in `CHANGELOG.md` and `ROADMAP.md`.

## Not for publication

Product and engineering strategy, commercial tiering, and the internal build
record live in the private sibling repository. Nothing under `site/` may link to
them, because `site/` is published.
