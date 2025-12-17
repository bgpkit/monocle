//! Session-based database storage
//!
//! This module provides database storage for one-time/session use cases,
//! such as storing BGP search results during a search operation.
//!
//! Unlike the shared database, session databases are typically:
//! - Created per-operation (e.g., per search)
//! - Stored in user-specified locations
//! - Not shared across monocle instances
//!
//! # Feature Requirements
//!
//! The `MsgStore` type requires the `lens-bgpkit` feature because it depends
//! on `bgpkit_parser::BgpElem` for storing BGP elements.

#[cfg(feature = "lens-bgpkit")]
mod msg_store;

#[cfg(feature = "lens-bgpkit")]
pub use msg_store::MsgStore;
