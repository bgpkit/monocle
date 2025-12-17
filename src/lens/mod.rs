//! Lens module
//!
//! This module provides high-level "lens" abstractions that combine business logic
//! with output formatting. Lenses are designed to be reusable across different
//! interfaces (CLI, REST API, WebSocket, GUI).
//!
//! # Feature Requirements
//!
//! Lenses are organized by the features they require:
//!
//! | Lens | Feature Required | Dependencies |
//! |------|-----------------|--------------|
//! | `TimeLens` | `lens-core` | chrono, dateparser |
//! | `CountryLens` | `lens-bgpkit` | bgpkit-commons |
//! | `IpLens` | `lens-bgpkit` | ureq, radar-rs |
//! | `ParseLens` | `lens-bgpkit` | bgpkit-parser |
//! | `SearchLens` | `lens-bgpkit` | bgpkit-broker, bgpkit-parser, rayon |
//! | `RpkiLens` | `lens-bgpkit` | bgpkit-commons |
//! | `Pfx2asLens` | `lens-bgpkit` | bgpkit-commons, oneio |
//! | `As2relLens` | `lens-bgpkit` | (database only) |
//! | `InspectLens` | `lens-full` | All above |
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
//! // Time parsing (lens-core)
//! use monocle::lens::time::{TimeLens, TimeParseArgs, TimeOutputFormat};
//!
//! // RPKI validation (lens-bgpkit)
//! use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiListArgs, RpkiRoaEntry};
//!
//! // IP information (lens-bgpkit)
//! use monocle::lens::ip::{IpLens, IpLookupArgs, IpInfo};
//!
//! // Unified AS/prefix inspection (lens-full)
//! use monocle::lens::inspect::{InspectLens, InspectQueryOptions};
//! ```

// =============================================================================
// Utility module (always available when any lens feature is enabled)
// =============================================================================
pub mod utils;

// =============================================================================
// Core lenses (lens-core feature)
// =============================================================================

// TimeLens - time parsing and formatting
#[cfg(feature = "lens-core")]
pub mod time;

// =============================================================================
// BGPKIT lenses (lens-bgpkit feature)
// =============================================================================

// CountryLens - country code/name lookup using bgpkit-commons
#[cfg(feature = "lens-bgpkit")]
pub mod country;

// IpLens - IP information lookup
#[cfg(feature = "lens-bgpkit")]
pub mod ip;

// ParseLens - MRT file parsing with bgpkit-parser
#[cfg(feature = "lens-bgpkit")]
pub mod parse;

// SearchLens - BGP message search across MRT files
#[cfg(feature = "lens-bgpkit")]
pub mod search;

// RpkiLens - RPKI validation and data
#[cfg(feature = "lens-bgpkit")]
pub mod rpki;

// Pfx2asLens - prefix-to-ASN mapping
#[cfg(feature = "lens-bgpkit")]
pub mod pfx2as;

// As2relLens - AS-level relationships (uses database, but grouped with bgpkit for convenience)
#[cfg(feature = "lens-bgpkit")]
pub mod as2rel;

// =============================================================================
// Full lenses (lens-full feature)
// =============================================================================

// InspectLens - unified AS and prefix information lookup
#[cfg(feature = "lens-full")]
pub mod inspect;
