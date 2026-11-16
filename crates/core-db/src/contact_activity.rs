//! Contact activity: the last time a person on a client account was actually
//! contacted, derived from the record this product already keeps.
//!
//! Sources, all scoped to the client's own tickets or identity:
//!   * inbound email from the contact's address,
//!   * outbound email to the contact's address,
//!   * a portal reply by the contact's portal user,
//!   * a call to or from the contact's number.
//!
//! Nothing is written here. A derived read is the honest shape: there is one
//! event ledger, and the column is a view of it rather than a second fact that
//! can drift.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

/// One dated contact event.
#[derive(Debug, Clone)]
pub struct ContactActivityEvent {
    pub contact_id: Uuid,
    pub at: DateTime<Utc>,
    /// `email_in`, `email_out`, `portal_reply`, `call_in`, `call_out`.
    pub kind: String,
    /// The subject, ticket number or call summary, when the source has one.
    pub detail: Option<String>,
}

/// The most recent events for every contact on one client, newest first, capped
/// per contact so a long correspondence cannot turn a table into a report.
pub async fn list_for_client(
    executor: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    per_contact: i64,
) -> Result<Vec<ContactActivityEvent>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        WITH address_book AS (
            SELECT
                c.id AS contact_id,
                lower(COALESCE(c.email, '')) AS email,
                regexp_replace(COALESCE(c.phone, ''), '\D', '', 'g') AS phone
            FROM contacts c
            WHERE c.org_id = $1
              AND c.client_id = $2
              AND c.merged_into_id IS NULL
        ),
        events AS (
            -- Email tied to one of this client's tickets, in either direction.
            SELECT
                a.contact_id,
                em.created_at AS at,
                CASE WHEN em.direction = 'inbound' THEN 'email_in' ELSE 'email_out' END AS kind,
                COALESCE(NULLIF(em.subject, ''), NULLIF(t.number, ''), NULL) AS detail
            FROM address_book a
            JOIN email_messages em
              ON em.org_id = $1
             AND a.email <> ''
             AND (
                  -- Inbound mail is from the contact; outbound mail is to them.
                  -- Pairing the address with the direction is what stops a
                  -- colleague copied on a customer's email from reading as a
                  -- contact that never happened.
                  (em.direction = 'inbound' AND lower(em.from_address) = a.email)
                  OR (em.direction = 'outbound' AND EXISTS (
                        SELECT 1 FROM unnest(em.to_addresses) AS addr
                        WHERE lower(addr) = a.email
                      ))
             )
            JOIN tickets t ON t.id = em.ticket_id AND t.org_id = $1 AND t.client_id = $2

            UNION ALL

            -- A portal reply by the contact's portal identity.
            SELECT
                a.contact_id,
                ae.occurred_at AS at,
                'portal_reply' AS kind,
                NULLIF(ae.detail ->> 'ticket_number', '') AS detail
            FROM address_book a
            JOIN portal_users pu
              ON pu.org_id = $1
             AND pu.client_id = $2
             AND a.email <> ''
             AND lower(pu.email) = a.email
            JOIN audit_events ae
              ON ae.org_id = $1
             AND ae.action = 'portal.ticket_reply'
             AND ae.actor_id = pu.id::text
             AND ae.actor_kind IN ('portal_contact', 'portal_approver', 'portal_it_manager')

            UNION ALL

            -- A call to or from the contact's number.
            SELECT
                a.contact_id,
                v.occurred_at AS at,
                CASE WHEN v.direction = 'in' THEN 'call_in' ELSE 'call_out' END AS kind,
                NULLIF(v.summary, '') AS detail
            FROM address_book a
            JOIN voip_call_logs v
              ON v.org_id = $1
             AND a.phone <> ''
             AND length(a.phone) >= 7
             AND regexp_replace(COALESCE(v.external_number, ''), '\D', '', 'g') = a.phone
        ),
        ranked AS (
            SELECT
                contact_id, at, kind, detail,
                row_number() OVER (PARTITION BY contact_id ORDER BY at DESC) AS rn
            FROM events
        )
        SELECT contact_id, at, kind, detail
        FROM ranked
        WHERE rn <= $3
        ORDER BY at DESC
        "#,
    )
    .bind(org_id)
    .bind(client_id)
    .bind(per_contact)
    .fetch_all(executor)
    .await?;

    use sqlx::Row;
    Ok(rows
        .into_iter()
        .map(|row| ContactActivityEvent {
            contact_id: row.get("contact_id"),
            at: row.get("at"),
            kind: row.get("kind"),
            detail: row.get("detail"),
        })
        .collect())
}
