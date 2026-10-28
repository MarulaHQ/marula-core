//! marula-core-cli — the operator entrypoint for Marula's background services.
//!
//! Purpose: the library half of the `marula` binary. The verbs (migrate, worker,
//! upgrade, data, install, doctor) live here so that a second host can call the
//! same code without re-running the command line: the installer app invokes
//! these modules directly, and `marula install --ui` serves them over loopback.
//! One engine, two hosts, no third implementation of what a plan means.
//!
//! Non-purpose: no server runtime of its own. `attachments`, `data`, `fixture`
//! and `upgrade` are operator verbs that dial out; the installer modules plan
//! and execute a deployment. Neither serves the product's API.

pub mod attachments;
pub mod data;
pub mod deploy;
pub mod deploy_check;
pub mod fixture;
pub mod install;
pub mod update;
pub mod upgrade;
