//! Local HTTP node over one ledger.
//!
//! ```text
//! Node      one Ledger behind a lock, an optional signing key, typed answers
//! routes    thin axum handlers over Node, plus hosting of the built explorer
//! guard     Host and Origin checks against browser-borne requests
//! demo      labeled example records for a first look
//! ```
//!
//! The API has no authentication and is meant for one machine. Records are
//! checked exactly as the CLI checks them; the node adds nothing to what a
//! signature or an inclusion proof establishes.

pub mod demo;
mod error;
mod guard;
mod node;
mod routes;

pub use error::NodeError;
pub use node::{
    ArtifactStatus, Draft, Node, Proof, Published, RecordDetail, RecordFilter, RecordPage, Status,
    Summary, Tree, VerifyReport,
};
pub use routes::{router, Settings};
