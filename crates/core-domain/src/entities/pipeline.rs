//! Sales pipeline: stages, deals (opportunities), and the audited
//! stage-transition ledger (the private build tracker Phase 3, Wave 12 lane 2).
//!
//! A pipeline is an org-scoped ordered list of [`PipelineStage`]s, each
//! with a kind — `open` (pipeline), `won` / `lost` (terminal). A
//! [`PipelineDeal`] links one client and OPTIONALLY a quote, carries a
//! value in one currency, and always sits on exactly one stage; moving it
//! appends an immutable [`PipelineDealTransition`] row (from → to, actor,
//! note, time). The ledger is the audit trail — there is no mutable
//! "current stage" history to rewrite.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{NaiveDate, Utc};
use std::str::FromStr;

/// The kind of a pipeline stage. `won` / `lost` are terminal outcomes;
/// deals on such a stage are closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum StageType {
    Open,
    Won,
    Lost,
}

impl StageType {
    pub const ALL: [StageType; 3] = [StageType::Open, StageType::Won, StageType::Lost];

    pub const fn as_str(self) -> &'static str {
        match self {
            StageType::Open => "open",
            StageType::Won => "won",
            StageType::Lost => "lost",
        }
    }

    pub const fn is_terminal(self) -> bool {
        !matches!(self, StageType::Open)
    }
}

impl FromStr for StageType {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "won" => Ok(Self::Won),
            "lost" => Ok(Self::Lost),
            other => Err(DomainError::Validation(format!(
                "invalid pipeline stage type: {other}"
            ))),
        }
    }
}

/// A deal's posture, mirroring its stage's kind. Kept as a distinct enum
/// so consumers filter deals without resolving the stage row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DealStatus {
    Open,
    Won,
    Lost,
}

impl DealStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            DealStatus::Open => "open",
            DealStatus::Won => "won",
            DealStatus::Lost => "lost",
        }
    }

    /// The posture a deal takes when placed on a stage of this kind.
    pub const fn for_stage(stage: StageType) -> Self {
        match stage {
            StageType::Open => DealStatus::Open,
            StageType::Won => DealStatus::Won,
            StageType::Lost => DealStatus::Lost,
        }
    }
}

impl FromStr for DealStatus {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "won" => Ok(Self::Won),
            "lost" => Ok(Self::Lost),
            other => Err(DomainError::Validation(format!(
                "invalid pipeline deal status: {other}"
            ))),
        }
    }
}

/// One ordered stage of the org's pipeline.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineStage {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub position: i32,
    pub stage_type: StageType,
    pub active: bool,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl PipelineStage {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation(
                "pipeline stage name is required".into(),
            ));
        }
        if self.name.len() > 100 {
            return Err(DomainError::Validation(
                "pipeline stage name must be at most 100 characters".into(),
            ));
        }
        Ok(())
    }
}

/// An opportunity: one client, optionally one quote, one current stage.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineDeal {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub quote_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stage_id: uuid::Uuid,
    pub name: String,
    pub amount_minor: i64,
    pub currency: String,
    pub status: DealStatus,
    pub expected_close_on: Option<NaiveDate>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub owner_user_id: Option<uuid::Uuid>,
    pub notes: Option<String>,
    /// Set when the deal lands on a won/lost stage.
    pub closed_at: Option<chrono::DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl PipelineDeal {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation(
                "pipeline deal name is required".into(),
            ));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        if self.amount_minor < 0 {
            return Err(DomainError::Validation(
                "pipeline deal amount must not be negative".into(),
            ));
        }
        Ok(())
    }
}

/// One immutable stage move (or the initial placement, with a NULL
/// `from_stage_id`). Appended in the same transaction as the move.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineDealTransition {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub deal_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub from_stage_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_stage_id: uuid::Uuid,
    pub note: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub actor_id: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_types_round_trip() {
        for t in StageType::ALL {
            assert_eq!(StageType::from_str(t.as_str()).unwrap(), t);
            assert_eq!(
                serde_json::to_value(t).unwrap(),
                serde_json::json!(t.as_str())
            );
        }
        assert!(StageType::from_str("maybe").is_err());
        assert!(!StageType::Open.is_terminal());
        assert!(StageType::Won.is_terminal());
        assert_eq!(DealStatus::for_stage(StageType::Lost), DealStatus::Lost);
    }

    #[test]
    fn deal_validation_basics() {
        let mut deal = PipelineDeal {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            client_id: uuid::Uuid::now_v7(),
            quote_id: None,
            stage_id: uuid::Uuid::now_v7(),
            name: "Workstation rollout".into(),
            amount_minor: 12_000,
            currency: "USD".into(),
            status: DealStatus::Open,
            expected_close_on: None,
            owner_user_id: None,
            notes: None,
            closed_at: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(deal.validate().is_ok());
        deal.amount_minor = -1;
        assert!(deal.validate().is_err());
        deal.amount_minor = 0;
        deal.currency = "doubloons".into();
        assert!(deal.validate().is_err());
    }
}
