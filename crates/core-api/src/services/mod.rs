//! Service functions shared by an HTTP route and an internal seam.
//!
//! A route that owns an effect the workflow executor also needs puts the
//! effect here, so both callers run the SAME code instead of a copy. The
//! HTTP route keeps the session principal and passes it in; the executor's
//! node endpoint materialises a principal for the run's author and passes
//! the same shape. Wave 43 lane 20 (`p43-workflow-foundation-gaps`) item E
//! established the pattern with the ticket status transition.

pub mod tickets;
