# How to customize without code

> **Audience:** both — admins declare the customization; staff experience the result.

Everything an organization configures lives as declarative rows bound to
stable semantic primitives — customizations are data, not code. That is what
makes them migratable and fork-safe.

## Prerequisites

- A signed-in admin (settings access) or an API client with the customization scopes.


## Add a custom field

Custom fields are declared per entity (client, contact, asset, …) with a
typed schema (`text | number | date | bool | select | url | email`).
Use the console (Settings) or the API under `/api/v1/…custom-fields` — see
the [API reference](../reference/api.md) for the exact routes.

## Reshape a board safely

Board statuses are always bound to semantic categories. Renaming boards,
statuses, or workflows never alters SLA, billing, automation, or reporting
behavior, because those bind to categories, not labels. Custom workflow
changes are therefore **not** breaking changes.

Rules of thumb:

- Statuses map to semantic categories; behavior follows the category.
- A board move (`POST /tickets/{id}/move-board`) remaps status through the
  semantic categories and is audited.

## Automate

Automations are declarative trigger/action rows, migrated as data under the
same rules as any other customization. Configure them in the console or via
the API.

## What this buys you at upgrade time

Because customizations are rows bound to stable primitives:

1. Migrations that touch customized constructs carry the transform for those
   rows in the same transaction.
2. The [upgrade preflight](upgrade.md) can tell you — before you upgrade —
   exactly which of your customizations touch anything deprecated in the
   target version.

## What is NOT a customization surface

Anything past the four stable boundaries (REST + WebSocket API, domain
events & webhooks, plugin API, declared customizations) — physical tables,
internal crate APIs, worker payloads — is explicitly not stable, ever.
