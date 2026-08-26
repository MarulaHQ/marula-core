//! Domain entity structs — the serde shapes shared by DB mapping and the API.

pub mod attachment;
pub mod audit;
pub mod board;
pub mod checklist;
pub mod client;
pub mod email;
pub mod notification;
pub mod org;
pub mod portal;
pub mod scheduling;
pub mod sla;
pub mod ticket;
pub mod time_tracking;

pub use attachment::*;
pub use audit::*;
pub use board::*;
pub use checklist::*;
pub use client::*;
pub use email::*;
pub use notification::*;
pub use org::*;
pub use portal::*;
pub use scheduling::*;
pub use sla::*;
pub use ticket::*;
pub use time_tracking::*;
