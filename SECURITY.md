# Security policy

## Report a vulnerability

Do not open a public issue. Use GitHub's private vulnerability reporting (Security → Report a vulnerability), or email `security@getmarula.com` with `[marula]` in the subject.

Include the affected version or commit, the request that reproduces it, what an attacker gains from it, and any logs. Strip tenant data before you send: there should be none in a report.

## Test the staging instance

Marula keeps a disposable staging instance for alpha testers and independent security researchers. It is rebuilt from `main` and holds no customer data; every account in it was issued to a tester.

- **Target:** the operator sends you the URL with your credentials. Credentials are issued out of band and never published here.
- **Accounts:** one per tester, with a role the operator chose. Owner accounts stay with the operator. Do not share credentials or make accounts for other people.
- **Before you report:** ask the operator for the list of known issues. Those items are already owned, and a fresh report on one of them costs everyone a day.
- **Disclosure window:** 90 days from the report, or earlier when a fix ships. We tell you when it does, and credit you in the advisory unless you prefer otherwise.

### In scope

- The web console at the target host, and the API under `/api` (also mounted at `/api/v1`).
- Sign-in, session, MFA, invite and password-reset flows.
- Tenant isolation: reaching another organization's data, or another person's data inside your own org.
- Authorization on the surfaces the console calls, the client portal included.
- Uploads and downloads, including the attachment and document objects behind them.
- Workflow, automation and webhook effects that a permitted user can trigger.
- The manifests in `deploy/`: a default that weakens a deployment is a finding.

### Out of scope

- Denial of service and load testing. The instance is a small single node and the app rate-limits per principal, so a flood takes the target away from every other tester. A handful of requests that show a limit is missing is fine.
- Social engineering, physical access, and anything aimed at Marula staff or at infrastructure other than the staging target.
- Third-party services (mail, payment, model providers) and their dashboards.
- Scanner output with no working request behind it.
- The known issues list. Send new information about one of those items instead of opening a report for it.
- Self-hosted deployments other than the staging instance. Hardening a deployment is the operator's job; the defaults in `deploy/` are in scope, a running operator's configuration is not.

### Rules of engagement

- Use only the accounts issued to you. If a finding needs a second account to prove, ask the operator for one.
- Prove the minimum. Read one row, not the table. Do not download or keep data that is not yours.
- Do not delete or corrupt data outside your own org, and do not leave persistence behind (admin users, API keys, webhooks, scheduled jobs). If proving a finding required creating one of those, say so in the report and remove it.
- Do not pivot into the Kubernetes cluster, the database or the host. Access to them is out of scope even when the app makes it reachable.
- If you find evidence of a real breach rather than a test result, email the operator before doing anything else.
- Research that stays inside this policy is authorised. We will not pursue or report you for it.

## What to expect

- Acknowledgment within 48 hours
- Triage and severity within 7 days (CVSS-based)
- Fix timeline: critical ≤7 days, high ≤30 days, others on the regular release train
- Coordinated disclosure as described above

## Supported versions

| Version | Support |
| --- | --- |
| Latest stable release | Security fixes |
| Previous minor release | Critical fixes only |
| Anything older | Upgrade |

## Scope notes

- RLS is a backstop, not the authorization system. A permission bypass is in scope even where RLS would have contained it.
- The staging instance can be rebuilt or wiped without notice, including mid-test. A finding has to survive that, so include the request and the response, not only a URL.
- Reports against the local demo stack (`scripts/demo-up.sh`) are useful, but it runs with development defaults on purpose and those defaults are not findings.
