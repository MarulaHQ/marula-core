//! Project delivery extensions (change orders, templates, portfolio, pods,
//! client updates, ticket conversion and the portal project view).
//!
//! This module is registered from `routes::mod` and owns the project-side
//! handlers so the existing `projects` module stays the record read/write
//! seam.
//!
//! Surface-pack rules held here:
//! * A change order is the document; a billing release is the act. Approval
//!   never applies, apply never bills (S13.6), and a failed release is its own
//!   state rather than a half-applied row (S13.8).
//! * A client update is proposed by records, approved by a person and sent by
//!   a person. Nothing here sends itself (S2.11).
//! * Portfolio risk is a deterministic rule with named reasons; a project with
//!   no data is "not enough movement to assess", never healthy (S7.4).
//! * Promotion is idempotent: a task has one ticket, and asking twice returns
//!   the first (S10.7).

use axum::routing::{get, patch, post, put};
use axum::Router;

use crate::routes::{route_path, ApiGeneration};
use crate::state::AppState;
use core_domain::policy::{Action, Module, Permission};

fn projects(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Projects,
        action,
    }
}

pub mod change_orders;
pub mod client_updates;
pub mod pods;
pub mod portal_projects;
pub mod portfolio;
pub mod templates;

/// The project-delivery staff slice. Mounted beside the existing `projects`
/// routes; the paths here are static or nested under a project id that the
/// existing record routes already own.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let rp = |tail: &str| route_path(gen, tail);
    Router::new()
        // Templates: gallery, catalogue facts, preview, validation and the
        // lessons loop. Document CRUD stays in `projects::templates`.
        .route(&rp("/projects/template-gallery"), get(templates::gallery))
        .route(
            &rp("/project-templates/{id}/gallery-card"),
            get(templates::card),
        )
        .route(
            &rp("/project-templates/{id}/archive"),
            post(templates::archive),
        )
        .route(
            &rp("/project-templates/{id}/meta"),
            put(templates::set_meta),
        )
        .route(
            &rp("/project-templates/{id}/preview"),
            get(templates::preview),
        )
        .route(
            &rp("/project-templates/{id}/validate"),
            get(templates::validate),
        )
        .route(
            &rp("/project-templates/{id}/lessons"),
            get(templates::list_lessons).post(templates::create_lesson),
        )
        .route(
            &rp("/project-templates/{id}/lessons/{lesson_id}/decide"),
            post(templates::decide_lesson),
        )
        .route(
            &rp("/project-templates/{id}/create"),
            post(templates::create_from_template),
        )
        // Change orders.
        .route(
            &rp("/projects/{id}/change-orders"),
            get(change_orders::list).post(change_orders::create),
        )
        .route(
            &rp("/projects/{id}/change-orders/{change_order_id}"),
            get(change_orders::get_one).patch(change_orders::patch),
        )
        .route(
            &rp("/projects/{id}/change-orders/{change_order_id}/submit"),
            post(change_orders::submit),
        )
        .route(
            &rp("/projects/{id}/change-orders/{change_order_id}/approve"),
            post(change_orders::approve),
        )
        .route(
            &rp("/projects/{id}/change-orders/{change_order_id}/reject"),
            post(change_orders::reject),
        )
        .route(
            &rp("/projects/{id}/change-orders/{change_order_id}/apply"),
            post(change_orders::apply),
        )
        .route(
            &rp("/projects/{id}/change-orders/{change_order_id}/bill"),
            post(change_orders::bill),
        )
        // Client updates.
        .route(
            &rp("/projects/{id}/client-updates"),
            get(client_updates::list).post(client_updates::draft),
        )
        .route(
            &rp("/projects/{id}/client-updates/{update_id}"),
            patch(client_updates::edit),
        )
        .route(
            &rp("/projects/{id}/client-updates/{update_id}/approve"),
            post(client_updates::approve),
        )
        .route(
            &rp("/projects/{id}/client-updates/{update_id}/send"),
            post(client_updates::send),
        )
        .route(
            &rp("/projects/{id}/client-updates/{update_id}/withhold"),
            post(client_updates::withhold),
        )
        // Task pods: blockers, promotion and the personal feed.
        .route(
            &rp("/projects/{id}/tasks/{task_id}/blocker"),
            put(pods::set_blocker).delete(pods::clear_blocker),
        )
        .route(
            &rp("/projects/{id}/tasks/{task_id}/promote"),
            post(pods::promote_task),
        )
        .route(
            &rp("/project-pods/settings"),
            get(pods::settings).put(pods::put_settings),
        )
        .route(
            &rp("/project-pods/prefs"),
            get(pods::prefs).put(pods::put_prefs),
        )
        .route(&rp("/projects/my-work"), get(pods::my_work))
        // Portfolio quarter and its shareable view.
        .route(&rp("/projects/portfolio"), get(portfolio::quarter))
        .route(&rp("/projects/portfolio/share"), post(portfolio::share))
        .route(
            &rp("/projects/portfolio/shared/{token}"),
            get(portfolio::shared),
        )
}

/// The portal half. Registered separately because portal tokens never pass the
/// staff auth middleware (see `routes::mod`); the integration owner merges this
/// into `portal_protected`.
pub fn portal_routes(gen: ApiGeneration) -> Router<AppState> {
    let rp = |tail: &str| route_path(gen, tail);
    Router::new()
        .route(&rp("/portal/projects"), get(portal_projects::list))
        .route(&rp("/portal/projects/{id}"), get(portal_projects::get_one))
        .route(
            &rp("/portal/projects/{id}/change-orders/{change_order_id}/acknowledge"),
            post(portal_projects::acknowledge),
        )
}

#[cfg(test)]
mod tests {
    use super::change_orders::validate_scope_changes;
    use super::client_updates::compose_body;
    use super::portfolio::risk_reasons;
    use serde_json::json;

    #[test]
    fn scope_changes_accept_the_documented_shape() {
        let value = json!({
            "tasks": [
                { "title": "Extra FortiGate staging", "duration_minutes": 240 }
            ],
            "shift_days": [{ "task_id": "00000000-0000-4000-8000-000000000001", "days": 2 }],
            "effort_minutes": 480
        });
        assert!(validate_scope_changes(&value).is_ok());
    }

    #[test]
    fn scope_changes_refuse_a_task_with_no_title() {
        let value = json!({ "tasks": [{ "title": "   " }] });
        assert!(validate_scope_changes(&value).is_err());
    }

    #[test]
    fn scope_changes_refuse_a_malformed_shift() {
        let value = json!({ "shift_days": [{ "task_id": "not-a-uuid", "days": 2 }] });
        assert!(validate_scope_changes(&value).is_err());
    }

    #[test]
    fn compose_body_names_what_moved_and_stays_plain() {
        let body = compose_body(
            "Office move",
            &[
                ("Phase complete", "Procurement"),
                ("Milestone reached", "Network installed"),
            ],
            None,
        );
        assert!(body.contains("Office move"));
        assert!(body.contains("Procurement"));
        assert!(body.contains("Network installed"));
        assert!(!body.contains("budget"));
    }

    #[test]
    fn compose_body_is_honest_when_nothing_moved() {
        let body = compose_body("Office move", &[], None);
        assert!(body.contains("No movement"));
    }

    #[test]
    fn risk_names_the_reason_in_words() {
        let reasons = risk_reasons("active", Some("2020-01-01"), false, true);
        assert_eq!(reasons.len(), 2);
        assert!(reasons.iter().any(|r| r.contains("due date")));
        assert!(reasons.iter().any(|r| r.contains("nothing is scheduled")));
    }

    #[test]
    fn a_healthy_project_has_no_risk_reason() {
        assert!(risk_reasons("active", Some("2999-01-01"), true, true).is_empty());
    }
}
