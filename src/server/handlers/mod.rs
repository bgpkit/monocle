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
//! - `as2org` - AS-to-Organization mappings (as2org.search, as2org.bootstrap)
//! - `as2rel` - AS-level relationships (as2rel.search, as2rel.relationship, as2rel.update)
//! - `pfx2as` - Prefix-to-ASN mappings (pfx2as.lookup)
//! - `database` - Database management (database.status, database.refresh)

pub mod as2org;
pub mod as2rel;
pub mod country;
pub mod database;
pub mod ip;
pub mod pfx2as;
pub mod rpki;
pub mod system;
pub mod time;

// Re-export all handlers for convenience
pub use as2org::{As2orgBootstrapHandler, As2orgSearchHandler};
pub use as2rel::{As2relRelationshipHandler, As2relSearchHandler, As2relUpdateHandler};
pub use country::CountryLookupHandler;
pub use database::{DatabaseRefreshHandler, DatabaseStatusHandler};
pub use ip::{IpLookupHandler, IpPublicHandler};
pub use pfx2as::Pfx2asLookupHandler;
pub use rpki::{RpkiAspasHandler, RpkiRoasHandler, RpkiValidateHandler};
pub use system::SystemInfoHandler;
pub use time::TimeParseHandler;
