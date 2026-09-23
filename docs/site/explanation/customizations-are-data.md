# Customizations are data, not code

Everything an organization configures (custom fields, board statuses,
automations, webhook subscriptions, settings registry values) lives as
declarative rows bound to stable semantic primitives. This is the
[`docs/upgrade-compatibility-policy.md`](../../upgrade-compatibility-policy.md)
§5 position, and it is the load-bearing idea behind Meridian's upgrade
story.

## The problem it solves

Modular, customizable systems acquire a reputation for being painful to
upgrade exactly when customization is implemented as *forked logic*: code
the customer owns that the vendor must either preserve forever or break.
Forked logic is why "upgrades are boring" fails elsewhere.

## The defaults-first doctrine

Semantic status categories, the settings registry, and seeded system rows
are not UX conveniences — they are what keeps customization expressible
*without* forkable logic. When a board status is bound to a semantic
category, renaming it cannot change SLA, billing, automation, or reporting
behavior, because all of those bind to the category. Custom workflow
changes are therefore not breaking changes.

## The two tools this enables

1. **Customization manifest** — a canonical, versioned export describing
   everything an org has customized. It doubles as backup documentation and
   as the input to the upgrade preflight.
2. **Upgrade preflight** — `meridian upgrade --check` runs offline against
   a manifest + database copy and reports exactly which customizations touch
   constructs deprecated in the target version.

## What this costs, honestly

Data-bound customization is less expressive than arbitrary code. Meridian
deliberately trades the last few percent of "do literally anything" for the
property that every org can always leave, upgrade, or fork — which the
[manifesto](why-meridian.md) names as the point.
