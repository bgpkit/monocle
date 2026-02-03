//! Lens module
//!
//! This module provides high-level "lens" abstractions that combine business logic
//! with output formatting. Lenses are designed to be reusable across different
//! interfaces (CLI, REST API, WebSocket, GUI).
//!
//! # Feature Requirements
//!
//! All lenses require the `lib` feature to be enabled:
//!
//! | Lens | Description | Dependencies |
//! |------|-------------|--------------|
//! | `TimeLens` | Time parsing and formatting | chrono, dateparser |
//! | `CountryLens` | Country code/name lookup | bgpkit-commons |
//! | `IpLens` | IP information lookup | ureq, radar-rs |
//! | `ParseLens` | MRT file parsing | bgpkit-parser |
//! | `SearchLens` | BGP message search | bgpkit-broker, bgpkit-parser, rayon |
//! | `RpkiLens` | RPKI validation and data | bgpkit-commons |
//! | `Pfx2asLens` | Prefix-to-ASN mapping | bgpkit-commons, oneio |
//! | `As2relLens` | AS-level relationships | database |
//! | `InspectLens` | Unified AS/prefix lookup | All above |
//!
//! # Architecture
//!
//! Each lens module exports:
//! - A **Lens struct** (e.g., `RpkiLens`, `TimeLens`) - the main entry point for all operations
//! - **Args structs** - input arguments for lens methods
//! - **Output types** - return types and format enums
//!
//! Internal implementation details (helper functions, data loading, API calls) are kept
//! private within each lens module. External users should only interact through the lens.
//!
//! # Usage
//!
//! All lens operations should be performed through the lens struct. Import the
//! specific lens module you need:
//!
//! ```rust,ignore
//! // Time parsing
//! use monocle::lens::time::{TimeLens, TimeParseArgs, TimeOutputFormat};
//!
//! // RPKI validation
//! use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiListArgs, RpkiRoaEntry};
//!
//! // IP information
//! use monocle::lens::ip::{IpLens, IpLookupArgs, IpInfo};
//!
//! // Unified AS/prefix inspection
//! use monocle::lens::inspect::{InspectLens, InspectQueryOptions};
//! ```

// =============================================================================
// Utility module (always available when lib feature is enabled)
// =============================================================================
pub mod utils;

// =============================================================================
// All lenses (require lib feature)
// =============================================================================

// TimeLens - time parsing and formatting
#[cfg(feature = "lib")]
pub mod time;

// CountryLens - country code/name lookup using bgpkit-commons
#[cfg(feature = "lib")]
pub mod country;

// IpLens - IP information lookup
#[cfg(feature = "lib")]
pub mod ip;

// ParseLens - MRT file parsing with bgpkit-parser
#[cfg(feature = "lib")]
pub mod parse;

// SearchLens - BGP message search across MRT files
#[cfg(feature = "lib")]
pub mod search;

// RpkiLens - RPKI validation and data
#[cfg(feature = "lib")]
pub mod rpki;

// Pfx2asLens - prefix-to-ASN mapping
#[cfg(feature = "lib")]
pub mod pfx2as;

// As2relLens - AS-level relationships
#[cfg(feature = "lib")]
pub mod as2rel;

// InspectLens - unified AS and prefix information lookup
#[cfg(feature = "lib")]
pub mod inspect;
