//! IP information lookup lens
//!
//! This module provides IP address information lookup functionality,
//! including ASN, prefix, RPKI validation state, and geolocation data.

use anyhow::Result;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

// =============================================================================
// Types
// =============================================================================

/// RPKI validation state for a route
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpRpkiValidationState {
    #[serde(rename = "valid")]
    Valid,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "unknown")]
    NotFound,
}

impl std::fmt::Display for IpRpkiValidationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpRpkiValidationState::Valid => write!(f, "valid"),
            IpRpkiValidationState::Invalid => write!(f, "invalid"),
            IpRpkiValidationState::NotFound => write!(f, "unknown"),
        }
    }
}

/// ASN and route information associated with an IP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpAsnRouteInfo {
    /// Autonomous System Number
    pub asn: i64,
    /// Network prefix covering this IP
    #[serde(rename(serialize = "prefix"))]
    pub prefix: IpNet,
    /// RPKI validation state
    pub rpki: IpRpkiValidationState,
    /// AS name/description
    pub name: String,
    /// Country code where the AS is registered
    pub country: Option<String>,
}

/// Complete IP information including location and network details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpInfo {
    /// The IP address
    pub ip: String,
    /// Country/location of the IP
    #[serde(rename(serialize = "location"))]
    pub country: Option<String>,
    /// Network/ASN information
    #[serde(rename(serialize = "network"))]
    pub asn: Option<IpAsnRouteInfo>,
}

/// Output format for IP lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum IpOutputFormat {
    /// JSON format (default)
    #[default]
    Json,
    /// Pretty-printed JSON
    Pretty,
    /// Simple text format
    Text,
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for IP lookup operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct IpLookupArgs {
    /// IP address to look up (if not provided, uses public IP)
    #[cfg_attr(feature = "cli", clap(value_name = "IP"))]
    pub ip: Option<IpAddr>,

    /// Return simplified response (less detail)
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub simple: bool,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "json"))]
    #[serde(default)]
    pub format: IpOutputFormat,
}

impl IpLookupArgs {
    /// Create new args for a specific IP
    pub fn new(ip: IpAddr) -> Self {
        Self {
            ip: Some(ip),
            simple: false,
            format: IpOutputFormat::default(),
        }
    }

    /// Create args for public IP lookup
    pub fn public_ip() -> Self {
        Self::default()
    }

    /// Set simple mode
    pub fn with_simple(mut self, simple: bool) -> Self {
        self.simple = simple;
        self
    }

    /// Set output format
    pub fn with_format(mut self, format: IpOutputFormat) -> Self {
        self.format = format;
        self
    }
}

// =============================================================================
// Lens
// =============================================================================

const IP_INFO_API: &str = "https://api.bgpkit.com/v3/utils/ip";

/// IP information lookup lens
///
/// Provides methods for looking up IP address information including
/// ASN, prefix, RPKI validation, and geolocation data.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::ip::{IpLens, IpLookupArgs};
/// use std::net::IpAddr;
///
/// let lens = IpLens::new();
///
/// // Look up a specific IP
/// let args = IpLookupArgs::new("1.1.1.1".parse().unwrap());
/// let info = lens.lookup(&args)?;
///
/// println!("IP: {}", info.ip);
/// if let Some(asn) = &info.asn {
///     println!("ASN: {}", asn.asn);
/// }
/// ```
pub struct IpLens;

impl IpLens {
    /// Create a new IP lens
    pub fn new() -> Self {
        Self
    }

    /// Look up IP information
    pub fn lookup(&self, args: &IpLookupArgs) -> Result<IpInfo> {
        let mut params = vec![];
        if let Some(ip) = args.ip {
            params.push(format!("ip={}", ip));
        }
        if args.simple {
            params.push("simple=true".to_string());
        }

        let url = if params.is_empty() {
            IP_INFO_API.to_string()
        } else {
            format!("{}?{}", IP_INFO_API, params.join("&"))
        };

        let resp = ureq::get(&url).call()?.body_mut().read_json::<IpInfo>()?;
        Ok(resp)
    }

    /// Format IP info for display
    pub fn format_result(&self, info: &IpInfo, format: &IpOutputFormat) -> String {
        match format {
            IpOutputFormat::Json => serde_json::to_string(info).unwrap_or_default(),
            IpOutputFormat::Pretty => serde_json::to_string_pretty(info).unwrap_or_default(),
            IpOutputFormat::Text => {
                let mut lines = vec![format!("IP: {}", info.ip)];
                if let Some(country) = &info.country {
                    lines.push(format!("Location: {}", country));
                }
                if let Some(asn) = &info.asn {
                    lines.push(format!("ASN: {}", asn.asn));
                    lines.push(format!("Prefix: {}", asn.prefix));
                    lines.push(format!("Name: {}", asn.name));
                    lines.push(format!("RPKI: {}", asn.rpki));
                    if let Some(country) = &asn.country {
                        lines.push(format!("AS Country: {}", country));
                    }
                }
                lines.join("\n")
            }
        }
    }
}

impl Default for IpLens {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_ip_info() {
        let lens = IpLens::new();
        let args = IpLookupArgs::public_ip();
        let my_public_ip_info = lens.lookup(&args).unwrap();
        dbg!(my_public_ip_info);
    }

    #[test]
    fn test_format_text() {
        let lens = IpLens::new();
        let info = IpInfo {
            ip: "1.1.1.1".to_string(),
            country: Some("US".to_string()),
            asn: Some(IpAsnRouteInfo {
                asn: 13335,
                prefix: "1.1.1.0/24".parse().unwrap(),
                rpki: IpRpkiValidationState::Valid,
                name: "CLOUDFLARENET".to_string(),
                country: Some("US".to_string()),
            }),
        };

        let output = lens.format_result(&info, &IpOutputFormat::Text);
        assert!(output.contains("1.1.1.1"));
        assert!(output.contains("13335"));
    }
}
