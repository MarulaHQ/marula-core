//! Cross-entity relationship links (docs-platform v1, migration 0053):
//! any document or password can link to a document, password, client,
//! ticket, or KB article. Links to vanished targets are surfaced as
//! broken (typed read model) rather than silently dropped.
//!
//! Authority (Wave 55 decision 3): a resolved LABEL is a read of the
//! target, so this resolver returns one only when the caller's gate for the
//! target's module allows it — the same `Action::View` the target's own
//! route requires, plus the record-scoped layer that gate already consults
//! (vault folder grants, board membership). A denied target keeps its link
//! row and comes back with `target_denied`, never a title. The gate lives
//! here because the document, password and asset panels all read through
//! this statement: no surface can resolve a label while asking a weaker
//! question, and a caller that forgets the scope does not compile.

use crate::{secret_vault, DbError, DbResult};
use core_domain::entities::{DocumentLink, LinkKind, ResolvedLink};
use core_domain::policy::{
    authorize_board_scoped, BoardOp, BoardRole, LinkViewScope, PolicyDecision,
};
use sqlx::{PgConnection, Postgres, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// A resolved row plus the coordinates its record-scoped gate needs. The
/// vault folder id and the ticket's board id never leave the resolver: they
/// are how the caller's own gate is consulted, not part of the read model.
struct ResolvedRow {
    link: ResolvedLink,
    /// The kind whose label this row carries — the target for `list_links`,
    /// the source for `list_links_to`.
    label_kind: LinkKind,
    secret_folder_id: Option<Uuid>,
    ticket_board_id: Option<Uuid>,
}

pub async fn list_links(
    conn: &mut PgConnection,
    org_id: Uuid,
    from_kind: LinkKind,
    from_id: Uuid,
    scope: &LinkViewScope,
) -> DbResult<Vec<ResolvedLink>> {
    let rows = sqlx::query(
        r#"SELECT l.id, l.org_id, l.from_kind, l.from_id, l.to_kind, l.to_id,
                  l.created_by, l.created_at,
                  CASE l.to_kind
                    WHEN 'document' THEN (SELECT title FROM documents d WHERE d.org_id = l.org_id AND d.id = l.to_id)
                    WHEN 'password' THEN (SELECT title FROM passwords p WHERE p.org_id = l.org_id AND p.id = l.to_id)
                    WHEN 'client'   THEN (SELECT name FROM clients c WHERE c.org_id = l.org_id AND c.id = l.to_id)
                    WHEN 'ticket'   THEN (SELECT t.number || ' ' || t.title FROM tickets t WHERE t.org_id = l.org_id AND t.id = l.to_id)
                    WHEN 'kb_article' THEN (SELECT title FROM kb_articles k WHERE k.org_id = l.org_id AND k.id = l.to_id)
                    WHEN 'asset' THEN (SELECT a.tag || ' ' || a.name FROM device_assets a WHERE a.org_id = l.org_id AND a.id = l.to_id)
                    WHEN 'secret' THEN (SELECT sr.title FROM secret_records sr
                        WHERE sr.id = l.to_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                  END AS target_label,
                  CASE l.to_kind
                    WHEN 'document' THEN (SELECT client_id FROM documents d WHERE d.org_id = l.org_id AND d.id = l.to_id)
                    WHEN 'password' THEN (SELECT client_id FROM passwords p WHERE p.org_id = l.org_id AND p.id = l.to_id)
                    WHEN 'ticket'   THEN (SELECT client_id FROM tickets t WHERE t.org_id = l.org_id AND t.id = l.to_id)
                    WHEN 'client'   THEN l.to_id
                    WHEN 'asset'    THEN (SELECT a.client_id FROM device_assets a WHERE a.org_id = l.org_id AND a.id = l.to_id)
                    WHEN 'secret'   THEN (SELECT sr.client_id FROM secret_records sr
                        WHERE sr.id = l.to_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                    ELSE NULL
                  END AS target_client_id,
                  CASE l.to_kind
                    WHEN 'secret' THEN (SELECT sr.folder_id FROM secret_records sr
                        WHERE sr.id = l.to_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                  END AS target_secret_folder_id,
                  CASE l.to_kind
                    WHEN 'ticket' THEN (SELECT t.board_id FROM tickets t WHERE t.org_id = l.org_id AND t.id = l.to_id)
                  END AS target_ticket_board_id
           FROM document_links l
           WHERE l.org_id = $1 AND l.from_kind = $2 AND l.from_id = $3
           ORDER BY l.created_at DESC"#,
    )
    .bind(org_id)
    .bind(from_kind.as_str())
    .bind(from_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut resolved: Vec<ResolvedRow> = rows
        .into_iter()
        .map(|r| {
            let to_kind = LinkKind::parse(r.get::<String, _>("to_kind").as_str())
                .unwrap_or(LinkKind::Document);
            let label: Option<String> = r.get("target_label");
            let deleted = label.is_none();
            ResolvedRow {
                link: ResolvedLink {
                    link: DocumentLink {
                        id: r.get("id"),
                        org_id: r.get("org_id"),
                        from_kind,
                        from_id: r.get("from_id"),
                        to_kind,
                        to_id: r.get("to_id"),
                        created_by: r.get("created_by"),
                        created_at: r.get("created_at"),
                    },
                    target_label: label,
                    target_client_id: r.get("target_client_id"),
                    target_deleted: deleted,
                    target_denied: false,
                },
                label_kind: to_kind,
                secret_folder_id: r.get("target_secret_folder_id"),
                ticket_board_id: r.get("target_ticket_board_id"),
            }
        })
        .collect();

    redact_unreadable(conn, org_id, &mut resolved, scope).await?;
    Ok(resolved.into_iter().map(|row| row.link).collect())
}

/// Drops every label the caller's own gates deny, keeping the rows. The
/// module flags come from `LinkViewScope`; the record-scoped passes below
/// ask the gates themselves (`secret_vault::folder_access` for a vault
/// record, `policy::authorize_board_scoped` plus `board_members` for a
/// ticket) rather than re-expressing either in this statement, which is how
/// a copy would drift from the gate it copies.
async fn redact_unreadable(
    conn: &mut PgConnection,
    org_id: Uuid,
    rows: &mut [ResolvedRow],
    scope: &LinkViewScope,
) -> DbResult<()> {
    let mut secret_checks: Vec<(usize, Uuid)> = Vec::new();
    let mut ticket_checks: Vec<(usize, Uuid)> = Vec::new();

    for (i, row) in rows.iter_mut().enumerate() {
        if !scope.may_view_module(row.label_kind) {
            deny(&mut row.link);
            continue;
        }
        // A null label past the module gate is a vanished target, not a
        // hidden one: there is nothing further to consult.
        if row.link.target_label.is_none() {
            continue;
        }
        match row.label_kind {
            LinkKind::Secret => {
                if let Some(folder_id) = row.secret_folder_id {
                    secret_checks.push((i, folder_id));
                }
            }
            LinkKind::Ticket => {
                if let Some(board_id) = row.ticket_board_id {
                    ticket_checks.push((i, board_id));
                }
            }
            _ => {}
        }
    }

    // Vault: a restricted folder chain denies without a grant row for one
    // of the caller's roles (the same per-record gate the vault's own list
    // runs).
    for (i, folder_id) in secret_checks {
        let visible =
            secret_vault::folder_access(&mut *conn, org_id, &scope.role_ids, folder_id, false)
                .await?;
        if !visible {
            deny(&mut rows[i].link);
        }
    }

    // Tickets: the board layer composes with the org verdict, exactly as
    // the ticket's own reads compose it (a membership row never expands
    // access). One membership read for every board on the panel.
    if !ticket_checks.is_empty() {
        let boards: Vec<Uuid> = ticket_checks.iter().map(|(_, board)| *board).collect();
        let membership = sqlx::query(
            "SELECT board_id, role::text AS role FROM board_members
             WHERE org_id = $1 AND user_id = $2 AND board_id = ANY($3)",
        )
        .bind(org_id)
        .bind(scope.user_id)
        .bind(&boards)
        .fetch_all(&mut *conn)
        .await?;
        let roles: HashMap<Uuid, BoardRole> = membership
            .iter()
            .filter_map(|r| {
                let board_id: Uuid = r.get("board_id");
                BoardRole::parse(r.get::<String, _>("role").as_str()).map(|role| (board_id, role))
            })
            .collect();
        for (i, board_id) in ticket_checks {
            let decision = authorize_board_scoped(
                PolicyDecision::Allowed,
                roles.get(&board_id).copied(),
                BoardOp::View,
            );
            if !decision.is_allowed() {
                deny(&mut rows[i].link);
            }
        }
    }

    Ok(())
}

/// The typed "not yours" shape: no title, no client association, and never
/// confused with a vanished target.
fn deny(link: &mut ResolvedLink) {
    link.target_label = None;
    link.target_client_id = None;
    link.target_deleted = false;
    link.target_denied = true;
}

/// Reverse-direction link list: links that POINT AT the given target
/// (e.g. the asset detail page's "vault entries" panel reads
/// password→asset links here). Same resolved-label read model, same gate:
/// the label this reads belongs to the FROM entity, so its module's view
/// gate decides it.
pub async fn list_links_to(
    conn: &mut PgConnection,
    org_id: Uuid,
    to_kind: LinkKind,
    to_id: Uuid,
    scope: &LinkViewScope,
) -> DbResult<Vec<ResolvedLink>> {
    let rows = sqlx::query(
        r#"SELECT l.id, l.org_id, l.from_kind, l.from_id, l.to_kind, l.to_id,
                  l.created_by, l.created_at,
                  CASE l.from_kind
                    WHEN 'document' THEN (SELECT title FROM documents d WHERE d.org_id = l.org_id AND d.id = l.from_id)
                    WHEN 'password' THEN (SELECT title FROM passwords p WHERE p.org_id = l.org_id AND p.id = l.from_id)
                    WHEN 'asset'    THEN (SELECT a.tag || ' ' || a.name FROM device_assets a WHERE a.org_id = l.org_id AND a.id = l.from_id)
                    WHEN 'secret'   THEN (SELECT sr.title FROM secret_records sr
                        WHERE sr.id = l.from_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                  END AS target_label,
                  NULL::uuid AS target_client_id,
                  CASE l.from_kind
                    WHEN 'secret' THEN (SELECT sr.folder_id FROM secret_records sr
                        WHERE sr.id = l.from_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                  END AS target_secret_folder_id
           FROM document_links l
           WHERE l.org_id = $1 AND l.to_kind = $2 AND l.to_id = $3
           ORDER BY l.created_at DESC"#,
    )
    .bind(org_id)
    .bind(to_kind.as_str())
    .bind(to_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut resolved: Vec<ResolvedRow> = rows
        .into_iter()
        .map(|r| {
            let from_kind = LinkKind::parse(r.get::<String, _>("from_kind").as_str())
                .unwrap_or(LinkKind::Document);
            let label: Option<String> = r.get("target_label");
            let deleted = label.is_none();
            ResolvedRow {
                link: ResolvedLink {
                    link: DocumentLink {
                        id: r.get("id"),
                        org_id: r.get("org_id"),
                        from_kind,
                        from_id: r.get("from_id"),
                        to_kind,
                        to_id: r.get("to_id"),
                        created_by: r.get("created_by"),
                        created_at: r.get("created_at"),
                    },
                    target_label: label,
                    target_client_id: r.get("target_client_id"),
                    target_deleted: deleted,
                    target_denied: false,
                },
                label_kind: from_kind,
                secret_folder_id: r.get("target_secret_folder_id"),
                // A from-side label is never a ticket in this read model.
                ticket_board_id: None,
            }
        })
        .collect();

    redact_unreadable(conn, org_id, &mut resolved, scope).await?;
    Ok(resolved.into_iter().map(|row| row.link).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_link<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    from_kind: LinkKind,
    from_id: Uuid,
    to_kind: LinkKind,
    to_id: Uuid,
    created_by: Uuid,
) -> DbResult<DocumentLink>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(
        r#"INSERT INTO document_links (id, org_id, from_kind, from_id, to_kind, to_id, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, org_id, from_kind, from_id, to_kind, to_id, created_by, created_at"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(from_kind.as_str())
    .bind(from_id)
    .bind(to_kind.as_str())
    .bind(to_id)
    .bind(created_by)
    .fetch_one(executor)
    .await?;
    Ok(DocumentLink {
        id: r.get("id"),
        org_id: r.get("org_id"),
        from_kind,
        from_id: r.get("from_id"),
        to_kind,
        to_id: r.get("to_id"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
    })
}

/// Removes one link, or both directions when `bidirectional` (the UI's
/// unlink acts on the link row it sees; the API also accepts the reverse
/// direction to make symmetric unlinking a single call).
pub async fn delete_link<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM document_links WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
