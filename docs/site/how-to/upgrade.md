# How to upgrade to the latest release

> **Audience:** both — self-host operators run the upgrade, admins own release timing.

Meridian guarantees you can always upgrade from either of the two previous
tagged releases (the **N−2 rule**, policy §1/§3). This guide gets you there
safely.

## Prerequisites

- Shell access to the deployment or the `meridian` CLI.
- A current backup — the [backup/restore runbook](../reference/backup-restore.md) first.
- Still on Postgres 17? Do the [one-time PG18 volume cut](../reference/pg18-upgrade.md)
  before or right after this upgrade. New installs are already on 18.


## 1. Check your version and the support window

Your version may be up to two releases behind. Further jumps apply the same
machinery sequentially — upgrade one release at a time.

## 2. Run the upgrade preflight

```sh
meridian upgrade --check
```

The preflight runs offline against your [customization
manifest](../explanation/customizations-are-data.md) + a database copy and
reports exactly which customizations touch constructs deprecated in the
target version. Deprecated constructs are listed in
[`deprecations.json`](../../../deprecations.json) with their
`removal_target`.

## 3. Back up

Take a full `pg_dump`. The [upgrade
fixtures](../reference/upgrade-matrix.md) CI restores are production-like
snapshots — your backup is your own equivalent.

## 4. Read "Breaking & deprecated"

Every release carries a release-notes section **"Breaking & deprecated"**,
even when it is empty. Read it before applying anything.

## 5. Apply migrations

Migrations run automatically on container start, or manually:

```sh
meridian migrate
```

The chain is forward-only and append-only. History is never edited, so any
deployed database replays it identically. Destructive steps are flagged
`--breaking` and only permitted in a major cycle after the deprecation
window has elapsed.

When your release ships new container images, pull the new tags **and**
their registry digests together — every image reference in the compose file
and the Helm chart is pinned to both. The digest is what actually runs, so
a tag bump without the digest bump changes nothing (and the Helm chart
refuses to render until you set both). Image uids did not change across
releases, so volumes written by an earlier release stay writable.

## 6. Verify

The same conformance suites CI runs against the [upgrade
fixtures](../reference/upgrade-matrix.md) (API/event conformance) are what
you should sanity-check against your deployment. If something behaves
differently, check the read model first: behavior binds to semantic
categories, not labels, so renamed boards/statuses never change SLA,
billing, automation, or reporting behavior.

## If you are stuck

Roll back to your backup, then open an issue with your customization
manifest attached. The manifest is a canonical, versioned export of
everything your org customized — it doubles as backup documentation.
