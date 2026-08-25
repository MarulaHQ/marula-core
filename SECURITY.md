# Security Policy

## Reporting a vulnerability

**Do not open a public issue.** Use GitHub's private vulnerability reporting (Security tab → Report a vulnerability), or email `security@meridian.dev` with `[meridian]` in the subject.

Include: affected version/commit, reproduction steps or PoC, impact assessment, and any logs — never real tenant data.

## What to expect

- Acknowledgment within 48 hours
- Triage + severity within 7 days (CVSS-based)
- Fix timeline: critical ≤7 days, high ≤30 days, others by regular release train
- Coordinated disclosure; you'll be credited in the advisory unless you prefer otherwise

## Supported versions

| Version | Support |
|---|---|
| Latest stable release | Security fixes |
| Previous minor release | Critical fixes only |
| Anything older | Upgrade |

## Scope notes

- Self-hosted deployment hardening is the operator's responsibility for anything outside our documented `deploy/` configurations.
- RLS is a backstop, not the authz system — permission bypass reports are in scope even where RLS would have contained them.
