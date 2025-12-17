//! WebSocket method handlers
//!
//! This module provides all the individual method handlers for the WebSocket API.
//! Each handler implements the `WsMethod` trait and provides a specific operation.
//!
//! # Handler Organization
//!
//! Handlers are organized by namespace:
//!
//! - `system` - Introspection methods (system.info, system.methods)
//! - `time` - Time parsing and formatting (time.parse)
//! - `country` - Country lookup (country.lookup)
//! - `ip` - IP information lookup (ip.lookup, ip.public)
//! - `rpki` - RPKI validation and data (rpki.validate, rpki.roas, rpki.aspas)
//! - `as2rel` - AS-level relationships (as2rel.search, as2rel.relationship, as2rel.update)
//! - `pfx2as` - Prefix-to-ASN mappings (pfx2as.lookup)
//! - `inspect` - Unified AS/prefix information (inspect.query, inspect.refresh)
//! - `database` - Database management (database.status, database.refresh)

pub mod as2rel;
pub mod country;
pub mod database;
pub mod inspect;
pub mod ip;
pub mod pfx2as;
pub mod rpki;
pub mod system;
pub mod time;

// Re-export all handlers for convenience
pub use as2rel::{As2relRelationshipHandler, As2relSearchHandler, As2relUpdateHandler};
pub use country::CountryLookupHandler;
pub use database::{DatabaseRefreshHandler, DatabaseStatusHandler};
pub use inspect::{InspectQueryHandler, InspectRefreshHandler};
pub use ip::{IpLookupHandler, IpPublicHandler};
pub use pfx2as::Pfx2asLookupHandler;
pub use rpki::{RpkiAspasHandler, RpkiRoasHandler, RpkiValidateHandler};
pub use system::SystemInfoHandler;
pub use time::TimeParseHandler;
