//! Product/service catalog items (the private build tracker Phase 3, Wave 12 lane 2).
//!
//! An org-scoped sellable: name, optional SKU, unit of measure, unit
//! price, cost, taxable flag, category/group, and an active flag. Money
//! is integer minor units + ISO 4217 (§5: floats never touch amounts).
//! Archival is data (`active = false`) — quotes snapshot their lines, so
//! catalog history is never rewritten.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::Utc;

/// One catalog item. `unit_price_minor` and `cost_minor` share `currency`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogItem {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    /// Optional SKU; unique per org (case-insensitive) when present.
    pub sku: Option<String>,
    pub description: Option<String>,
    /// Unit of measure the price applies to ("each", "hour", "month").
    pub unit: String,
    pub currency: String,
    pub unit_price_minor: i64,
    pub cost_minor: i64,
    pub taxable: bool,
    pub category: Option<String>,
    pub group_name: Option<String>,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl CatalogItem {
    /// Gross margin in minor units (unit price − cost); may be negative.
    pub fn margin_minor(&self) -> i64 {
        self.unit_price_minor - self.cost_minor
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation(
                "catalog item name is required".into(),
            ));
        }
        if self.name.len() > 200 {
            return Err(DomainError::Validation(
                "catalog item name must be at most 200 characters".into(),
            ));
        }
        if self.unit.trim().is_empty() {
            return Err(DomainError::Validation(
                "catalog item unit is required".into(),
            ));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        if self.unit_price_minor < 0 {
            return Err(DomainError::Validation(
                "unit price must not be negative".into(),
            ));
        }
        if self.cost_minor < 0 {
            return Err(DomainError::Validation("cost must not be negative".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> CatalogItem {
        CatalogItem {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            name: "Managed workstation".into(),
            sku: Some("WS-MGD".into()),
            description: None,
            unit: "month".into(),
            currency: "USD".into(),
            unit_price_minor: 9_900,
            cost_minor: 4_000,
            taxable: true,
            category: Some("Managed services".into()),
            group_name: None,
            active: true,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn validates_and_computes_margin() {
        let it = item();
        assert!(it.validate().is_ok());
        assert_eq!(it.margin_minor(), 5_900);
    }

    #[test]
    fn rejects_bad_fields() {
        let mut it = item();
        it.name = "  ".into();
        assert!(it.validate().is_err());
        let mut it = item();
        it.currency = "doubloons".into();
        assert!(it.validate().is_err());
        let mut it = item();
        it.unit_price_minor = -1;
        assert!(it.validate().is_err());
        let mut it = item();
        it.cost_minor = -1;
        assert!(it.validate().is_err());
    }
}
