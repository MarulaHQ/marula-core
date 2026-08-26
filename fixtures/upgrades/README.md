# Upgrade-matrix fixtures (policy §8 / TASKLIST U7)

Purpose: seeded production-like snapshot fixtures per tagged release — data
exercising every customization surface (custom fields, board statuses,
automations, webhook filters, settings registry keys) plus installed-plugin
manifests. CI restores fixture N−2 → migrates to HEAD → runs conformance suites.

Format: `vX.Y.Z.seed.sql.gz` logical dumps + `manifest.json` enumerating covered
customization surfaces. First fixtures generated at v0.1.x line once tag anchors exist.
