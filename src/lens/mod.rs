//! Lens module
//!
//! This module provides high-level "lens" abstractions that combine business logic
//! with output formatting. Lenses are designed to be reusable across different
//! interfaces (CLI, REST API, WebSocket, GUI).
//!
//! # Architecture
//!
//! Each lens module exposes:
//! - A **Lens struct** (e.g., `RpkiLens`, `TimeLens`) - the main entry point for all operations
//! - **Args structs** - input arguments for lens methods
//! - **Output types** - return types and format enums
//!
//! Internal implementation details (helper functions, data loading, API calls) are kept
//! private within each lens module. External users should only interact through the lens.
//!
//! ```text
//! lens/
//! ├── as2org/     # AS-to-Organization lookup lens
//! │   ├── args    # Reusable argument structs
//! │   ├── types   # Result types and enums
//! │   └── mod     # As2orgLens implementation
//! │
//! ├── as2rel/     # AS-level relationship lens
//! │   ├── args    # Reusable argument structs
//! │   ├── types   # Result types and enums
//! │   └── mod     # As2relLens implementation
//! │
//! ├── country     # CountryLens - country code/name lookup (in-memory)
//! │
//! ├── ip/         # IpLens - IP information lookup
//! │   └── mod     # Lens implementation with types and args
//! │
//! ├── parse/      # ParseLens - MRT file parsing
//! │   └── mod     # Lens implementation with filters
//! │
//! ├── pfx2as/     # Pfx2asLens - prefix-to-ASN mapping
//! │   └── mod     # Lens implementation with types and args
//! │
//! ├── rpki/       # RpkiLens - RPKI validation and data
//! │   ├── commons # (internal) bgpkit-commons integration
//! │   ├── validator # (internal) Cloudflare GraphQL API
//! │   └── mod     # Lens implementation
//! │
//! ├── search/     # SearchLens - BGP message search
//! │   └── mod     # Lens implementation with filters
//! │
//! └── time/       # TimeLens - time parsing and formatting
//!     └── mod     # Lens implementation with types and args
//! ```
//!
//! # Usage
//!
//! All lens operations should be performed through the lens struct. Import the
//! specific lens module you need:
//!
//! ```rust,ignore
//! // AS-to-Organization lookup
//! use monocle::lens::as2org::{As2orgLens, As2orgSearchArgs, As2orgOutputFormat};
//!
//! // Time parsing
//! use monocle::lens::time::{TimeLens, TimeParseArgs, TimeOutputFormat};
//!
//! // RPKI validation
//! use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiListArgs, RpkiRoaEntry};
//!
//! // IP information
//! use monocle::lens::ip::{IpLens, IpLookupArgs, IpInfo};
//!
//! // Prefix-to-ASN mapping
//! use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asLookupArgs};
//! ```
//!
//! # Examples
//!
//! ## RPKI Validation
//!
//! ```rust,ignore
//! use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiListArgs};
//!
//! let lens = RpkiLens::new();
//!
//! // Validate a prefix/ASN pair
//! let args = RpkiValidationArgs::new(13335, "1.1.1.0/24");
//! let (validity, covering_roas) = lens.validate(&args)?;
//!
//! // List ROAs for an ASN
//! let args = RpkiListArgs::for_asn(13335);
//! let roas = lens.list_roas(&args)?;
//! ```
//!
//! ## Time Parsing
//!
//! ```rust,ignore
//! use monocle::lens::time::{TimeLens, TimeParseArgs, TimeOutputFormat};
//!
//! let lens = TimeLens::new();
//!
//! // Parse various time formats
//! let args = TimeParseArgs::new(vec![
//!     "1697043600".to_string(),          // Unix timestamp
//!     "2023-10-11T00:00:00Z".to_string(), // RFC3339
//! ]);
//! let results = lens.parse(&args)?;
//!
//! // Format for display
//! let output = lens.format_results(&results, &TimeOutputFormat::Table);
//! ```
//!
//! ## IP Lookup
//!
//! ```rust,ignore
//! use monocle::lens::ip::{IpLens, IpLookupArgs};
//!
//! let lens = IpLens::new();
//!
//! // Look up a specific IP
//! let args = IpLookupArgs::new("1.1.1.1".parse().unwrap());
//! let info = lens.lookup(&args)?;
//!
//! // Get public IP info
//! let args = IpLookupArgs::public_ip();
//! let info = lens.lookup(&args)?;
//! ```

pub mod as2org;
pub mod as2rel;
pub mod country;
pub mod ip;
pub mod parse;
pub mod pfx2as;
pub mod rpki;
pub mod search;
pub mod time;
pub mod utils;
