//! FX entities (ADR 0002, Wave 12 lane 10): the append-only rate book
//! entry and the reconciliation read-model row. The conversion operation
//! itself lives in [`crate::fx`] — these are the wire shapes.

/// One `fx_rates` row: append-only; corrections add rows, never rewrite.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FxRateEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    /// 1 base major unit buys `rate` quote major units.
    pub base_currency: String,
    pub quote_currency: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub as_of_date: chrono::NaiveDate,
    /// Stored-precision decimal string ("17.850000000000").
    pub rate: String,
    /// 'manual' for the built-in provider; managed feeds stamp their own.
    pub source: String,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub created_by: Option<uuid::Uuid>,
}

/// One `ops.v1_fx_reconciliation` row — the FX deliverable to the books
/// (FX7): original amount, rate, source/date, base equivalent, sync
/// target. The core records conversions that happened; realized/unrealized
/// gain-loss stays in the accounting system.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FxReconciliationRow {
    /// 'expense' | 'cost_line' | 'invoice_issue' | 'labor_pricing' |
    /// 'inventory_receipt' | 'payment'.
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub fact_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: Option<uuid::Uuid>,
    pub original_currency: Option<String>,
    pub original_amount_minor: Option<i64>,
    pub fx_rate_used: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub fx_rate_date: Option<chrono::NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
    /// The accounting export target ('none' until a deployment configures
    /// one — qbo-export-sync / xero-export-sync are per-deployment).
    pub sync_target: String,
    pub fact_at: Option<chrono::DateTime<chrono::Utc>>,
}
