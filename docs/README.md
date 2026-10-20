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
| [`reference/architecture-review-2026-10.md`](reference/architecture-review-2026-10.md) | The reviewed shape of the system and the findings that came out of it |
| [`theme.md`](theme.md) | Visual theme tokens (binding) |
| [`instant-first.md`](instant-first.md) | Interaction rules for optimistic writes and undo (binding) |

## Run an instance

| Doc | Task |
|---|---|
| [`all-in-one.md`](all-in-one.md) | Run one process instead of two on a small install |
| [`deploy-descriptor.md`](deploy-descriptor.md) | The machine-readable deploy surface and every runtime parameter |
| [`capacity-model.md`](capacity-model.md) | How many agents one instance serves, from measurements |
| [`worker-cadence.md`](worker-cadence.md) | Which worker loops exist, how often they run, what a small install can trim |
| [`perf.md`](perf.md) | Budgets, the measurement harness, the recorded baselines |
| [`upgrade-compatibility-policy.md`](upgrade-compatibility-policy.md) | The N−2 support window and the expand-then-contract migration rule |
| [`desktop-release.md`](desktop-release.md) | Desktop packaging, signing and the auto-update path |

Operator tasks also live in the site, because operators read the site:
[`site/start/operators.md`](site/start/operators.md) links stand-up, TLS, backup,
upgrade and observability.

## Change a surface

| Doc | Scope |
|---|---|
| [`reference/parallel-lanes.md`](reference/parallel-lanes.md) | Merge groups, lane gates and the integration-owner mechanics |
| [`reference/marula-surface-brief.md`](reference/marula-surface-brief.md) | Roles, end-to-end paths and every web and native route grouped by surface |
| [`reference/wave41-marula-plan.md`](reference/wave41-marula-plan.md) | The rebrand plan of record and the decisions it locked |
| [`notes-tasks-workbar.md`](notes-tasks-workbar.md) | The workbar note and task panel specification |
| [`routing-providers.md`](routing-providers.md) | The routing contract, provider shapes and the warning codes |
| [`10-agent-platform.md`](10-agent-platform.md) | The agent harness: decision model, grants, run ledger |
| [`auth-hardening.md`](auth-hardening.md) | MFA, SSO, refresh-token reuse detection and audit completion |
| [`reference/auth-provider-contract.md`](reference/auth-provider-contract.md) | What an auth provider may implement |
| [`reference/auth-threat-model.md`](reference/auth-threat-model.md) | What the auth design is defending against |
| [`reference/plugin-runtime.md`](reference/plugin-runtime.md) | The plugin host and its capability model, plus the distribution contract |
| [`reference/plugin-signing-rotation-runbook.md`](reference/plugin-signing-rotation-runbook.md) | Rotating a plugin signing key without breaking installs |
| [`reference/workflow-node-catalogue.md`](reference/workflow-node-catalogue.md) | Every workflow node the builder offers |
| [`reference/read-model.md`](reference/read-model.md) | The reporting view contract, with per-view pages in [`reference/read-model/`](reference/read-model/) |
| [`reference/marula-name-map.md`](reference/marula-name-map.md) | Old name to new name, for reading anything written before the rebrand |
| [`reference/engineering-glossary.md`](reference/engineering-glossary.md) | Wave, lane, merge group, gate, honest gap — the shorthand in tracker rows and evidence directories |
| [`rn-groundwork.md`](rn-groundwork.md) | What the native app foundation shipped and what it reuses |
| [`11-web-presence.md`](11-web-presence.md) | The consumer website specification |
| [`e2e-event-type-registry.md`](e2e-event-type-registry.md) | The event-type registry the webhook admin surface reads |
| [`e2e-inbound-webhook-receiver.md`](e2e-inbound-webhook-receiver.md) | Signed inbound webhooks, and the deduplication rule behind the ingest event |

## Publish and check the docs

| Task | Command |
|---|---|
| Read the site conventions and the generated-page rules | [`site/README.md`](site/README.md) |
| Verify links, generated pages, the help manifest and the publish path | `node docs/site/scripts/check-docs-site.mjs` |
| Verify the information architecture (orphans, index coverage, vocabulary) | `node docs/site/scripts/check-docs-ia.mjs` |
| Re-render the static publication | `node docs/site/scripts/publish.mjs` |
| Read the coverage audit the site was built from | [`site/DOCS-COVERAGE.md`](site/DOCS-COVERAGE.md) |

Both checkers run on a clean checkout with Node only. The site-checker invokes
the IA checker, so one command covers both.

## History: read only when tracing a decision

[`STAGE-REPORT.md`](STAGE-REPORT.md) is one section per build stage.
[`BUILD-PROMPT-VERDICTS.md`](BUILD-PROMPT-VERDICTS.md) scores the original build
prompt item by item, and [`prompts/BUILD-PROMPT.md`](prompts/BUILD-PROMPT.md) is
the prompt itself.

## Not for publication

[`STRATEGY.md`](STRATEGY.md) holds product and engineering strategy. It stays in
this repository. Do not copy it, quote it, or link it from anything under
`site/`, which is published.
