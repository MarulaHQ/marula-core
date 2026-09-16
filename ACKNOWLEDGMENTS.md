# Acknowledgments — third-party software, fonts & vendored content

> Meridian would not exist without the open-source ecosystem. This file
> thanks the projects we build on and documents every third-party license
> we ship or vendor. **Meridian itself is AGPL-3.0-or-later** (see
> [`LICENSE`](LICENSE)); the SDK, plugin kit, first-party plugins &
> connectors are Apache-2.0 (see
> [`packages/sdk/LICENSE`](packages/sdk/LICENSE)).

## 1. Fonts (SIL Open Font License 1.1)

Loaded self-hosted at build time via `next/font/google` — the woff2 files
ship inside our bundles, so the license text must (and does) travel with
them:

| Font | Copyright | Used for |
|---|---|---|
| **Inter** | 2020 The Inter Project Authors (https://github.com/rsms/inter) | UI/body type |
| **JetBrains Mono** | 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono) | Code & data, tabular numerals |
| **Source Serif 4** | 2014 The Source Serif 4 Project Authors (https://github.com/adobe-fonts/source-serif) | Display/headline accents |

All three are licensed under the SIL Open Font License 1.1 (full text:
[`LEGAL/fonts/ofl-1.1.txt`](LEGAL/fonts/ofl-1.1.txt); per-family copyright
headers: `LEGAL/fonts/{inter,jetbrains-mono,source-serif-4}/OFL.txt`).
Redistributions of the font software (e.g. the packaged web bundle) carry
this license with them via this directory.

## 2. Embedded desktop stack (the double-clickable test app)

The local test app (ADR 0005) bundles real third-party binaries:

- **PostgreSQL 17** — licensed under the PostgreSQL License
  (https://www.postgresql.org/about/licence/).
- **pgvector** — same license family
  (https://github.com/pgvector/pgvector — PostgreSQL License).
- **Tauri** (shell/runtime) — MIT & Apache-2.0
  (https://github.com/tauri-apps/tauri).
- The embedded Node.js runtime and the Next.js standalone server with all
  npm dependencies (see §3). The Rust dependency tree is covered by §4.

Thank you to the PostgreSQL Global Development Group, the pgvector
contributors, the Tauri team, and the Node.js project.

## 3. Frameworks & libraries (summary)

The full machine inventories live here:

- **npm/web:** [`LEGAL/npm-licenses.json`](LEGAL/npm-licenses.json)
  (623 packages, regenerated with `pnpm licenses list --json` — see §6).
  Highlights: Next.js (MIT), React (MIT), Tailwind CSS (MIT), Recharts
  (MIT), TypeScript (Apache-2.0), Playwright (Apache-2.0), Vitest (MIT).
- **Rust:** the workspace license inventory is generated with
  [`cargo-about`](https://github.com/emboss-studios/cargo-about) or
  audited with `cargo-deny` (command see §6). Highlights: tokio/axum/tonic
  (MIT), sqlx (MIT/Apache-2.0), extism (BSD-3-Clause), clap (MIT/Apache-2.0).

**Notable non-MIT outliers in the web bundle** (all acceptable deps, kept
visible):

- `@img/sharp-libvips-*` — LGPL-3.0-or-later (bundled native lib; used as
  a library, compatible with our aggregate).
- `caniuse-lite` — CC-BY-4.0 (data, not code).
- `lightningcss` — MPL-2.0.
- `isexe`, `minimatch` — BlueOak-1.0.0 (npm's official copyleft-less
  post-IRL option).

## 4. Vendored agent skills (internal tooling, not shipped in product)

| Skill directory | Source | License |
|---|---|---|
| `.kilo/skills/postgres-best-practices` | https://github.com/supabase/agent-skills (v1.1.1; vendored 2026-09-13) | MIT |
| `.kilo/skills/rust-best-practices` | Apollo GraphQL handbook (apollographql) | MIT |
| `.kilo/skills/insecure-defaults` | https://github.com/trailofbits/skills (plugins/insecure-defaults) | MIT |
| `.kilo/skills/ui-skills-root` | https://ui-skills.com | MIT |
| `.kilo/skills/{better-ui,improve-ui,interface-design,fixing-accessibility,web-design-guidelines,rams}` | the ui-skills.com pack | documented as part of the ui-skills pack |
| `.kilo/skills/stripe-best-practices` | Stripe best-practices pattern pack | **provenance/licence not yet recorded in-file — follow-up: add a provenance header before it is redistributed anywhere** |

First-party skills written for this repo (rebuild-local-test-app) are ours.

## 5. Design sources

- **Tangerine theme** — generated from the tweakcn editor
  (https://tweakcn.com), vendored as `design/tangerine/tangerine.json` and
  documented in [`docs/theme.md`](docs/theme.md). tweakcn is MIT.
- Visual references in `design/mockups/` and markup drafts are first-party
  or internal research records (lazyweb citations live with each lane's
  `design/qa-*` evidence).

## 6. Keeping this honest

Regenerate the inventories whenever dependencies move:

```sh
pnpm licenses list --json   # → normalize into LEGAL/npm-licenses.json
cargo about generate about.hbs -o LEGAL/cargo-licenses.html   # one-time setup: cargo install cargo-about
cargo deny check licenses   # policy gates (recommended follow-up in a CI lane)
```

Any new vendored directory must carry a provenance header (source URL +
license + date). `AGENTS.md` requires this for skills; the stripe skill
needs one added (tracked as a follow-up above).
