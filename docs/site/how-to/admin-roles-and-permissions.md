---
description: "What each built-in Marula role can do: the module matrix, the special grants, board-level roles, and how to fork a preset into a custom role."
---

# How to understand role permissions

> **Audience:** admin — you assign roles, and someone will ask why a dispatcher
> cannot open an invoice.

Marula ships eight preset roles. Each one is a stored permission set on the same
matrix custom roles use, so what you read here is what the server checks, not a
summary of it.

## Prerequisites

- **Manage roles** (`special:manage_roles`) to change a role or assign one.
- **Manage users** (`special:manage_users`) to invite or edit a person. A
  dispatcher holds neither, but holds **manage boards**, which is enough to read
  the staff directory without changing it.

## 1. Pick the preset that matches the job

| Preset | What it is for |
|---|---|
| **Owner** | Full control, including billing and the organization itself. |
| **Admin** | Everything an Owner holds, except actions that would leave the org without an active Owner. |
| **Dispatcher** | Schedules and assigns work, and keeps boards in shape. |
| **Senior Tech** | Works tickets and approves time. No financial visibility. |
| **Technician** | Works tickets, uses the vaults, logs time. |
| **Bookkeeper** | Invoicing and financial visibility. Does not work tickets. |
| **Auditor** | Reads everything and reads the audit trail. Writes nothing. |
| **Read only** | Views every module. No writes, no audit trail. |

## 2. Read the module matrix

Each module grants five actions: view, create, edit, delete and export.

- **F** — all five actions.
- **V+E** — view and edit only.
- **V+X** — view and export only.
- **V** — view only.
- **—** means no access at all.

| Module | Owner | Admin | Dispatcher | Senior Tech | Technician | Bookkeeper | Auditor | Read only |
|---|---|---|---|---|---|---|---|---|
| Tickets | F | F | F | F | F | — | V | V |
| Clients | F | F | F | V | V | V | V | V |
| Time | F | F | V+E | V+E | V+E | V | V | V |
| Scheduling | F | F | F | V+E | V+E | — | V | V |
| Projects | F | F | V+E | — | — | — | V | V |
| Commercial | F | F | V+E | — | — | V | V | V |
| Invoicing | F | F | — | — | — | F | V | V |
| Knowledge base | F | F | V+E | F | F | — | V | V |
| Documents | F | F | — | — | — | — | V | V |
| Passwords | F | F | V | V+E | V+E | — | V | V |
| Secrets | F | F | V | V+E | V+E | — | V | V |
| Reporting | F | F | V+E | V | V | V+X | V | V |
| Admin | F | F | — | — | — | — | V | V |
| Plugins | F | F | — | — | — | — | V | V |

Two reads that surprise people:

- **Dispatcher holds the full Tickets and Scheduling matrix plus Clients**, and
  view-and-edit on Time, Projects, Commercial, Knowledge base and Reporting. That
  is a scheduling role with enough client context to place work, not a read-only
  board.
- **Senior Tech and Technician differ by one grant.** Both hold the full ticket
  and knowledge-base matrix. Senior Tech adds **approve time**. Cost visibility is
  not a seniority perk, so neither one can see rates.

## 3. Add the special grants

Some operations are not module-scoped. They are granted by name:

| Preset | Special grants |
|---|---|
| **Owner**, **Admin** | All ten: manage users, manage roles, manage boards, approve time, financial read, manage billing, manage plugins, manage API keys, view audit log, portal admin |
| **Dispatcher** | Manage boards |
| **Senior Tech** | Approve time |
| **Technician** | None |
| **Bookkeeper** | Financial read, manage billing |
| **Auditor** | View audit log |
| **Read only** | None |

A module grant never implies a special grant. A dispatcher who can edit a ticket
still cannot approve the hours logged against it.

## 4. Know what narrows a grant

Three things sit on top of the role, and each can only take access away:

- **Board roles.** Membership on a board carries **viewer**, **member** or
  **admin**. A viewer reads the board and nothing else. A member also opens and
  works tickets but does not close them. A board admin configures the board and
  its members. A board role never widens what the org role allows. See
  [Set workflow categories](admin-workflow-categories.md).
- **Secret categories.** Viewing the secrets module does not reveal a folder
  marked restricted. Category grants decide which folders a person sees, and
  revealing a value needs edit, not view. See [Manage API keys](admin-api-keys.md)
  for the pattern of showing a value once.
- **The organization itself.** Owner and Admin hold the same permission set. The
  difference is the last active Owner: that account cannot be deactivated or
  demoted, so the org always has one person who can reach billing and the org
  settings.

## 5. Change a role

Open **Settings → Team**. The preset roster sits above the directory.

- A preset row whose stored permissions no longer match the shipped matrix is
  marked, and offers **Restore**. Pressing it puts the canonical matrix back.
- **Fork** copies a preset into a custom role you can edit. Presets stay
  canonical. Forks are where local variation belongs.
- Custom roles open in the matrix editor, where you name the role and tick
  module and action pairs.

Give a person the narrowest preset that does their job, and fork when a single
person needs one extra grant. Widening a whole preset for one person makes every
future answer about that role wrong.

## What this page does not do

- It does not list AI surface grants. Those are separate from role permissions
  and are set per role in AI settings. See [Manage AI access](admin-ai-access.md).
- It does not cover portal accounts. Client contacts use a separate authorization
  model with tiers per client. See [Set up the portal](admin-portal-setup.md).
- It does not cover plugin capabilities. A plugin holds only what its consent
  record grants. See [Configure the plugin consent flow](admin-plugin-consent.md).

## Related

- [Manage users and invites](admin-users-invites.md) — putting a person in a role.
- [Dispatcher tasks](../start/dispatchers.md) — the job this matrix is shaped for.
- [Manage API keys](admin-api-keys.md) — shows the value-once pattern that the secret vaults reuse.
