//! RPKI data integration using bgpkit-commons.
//!
//! This module provides functions to load and query RPKI data (ROAs and ASPAs)
//! from bgpkit-commons, supporting both current (Cloudflare) and historical
//! (RIPE NCC, RPKIviews) data sources.

use anyhow::{anyhow, Result};
use bgpkit_commons::rpki::{HistoricalRpkiSource, RpkiTrie, RpkiViewsCollector};
use chrono::NaiveDate;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tabled::Tabled;

/// ROA entry for display
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct RoaEntry {
    pub prefix: String,
    pub max_length: u8,
    pub origin_asn: u32,
    pub ta: String,
}

/// ASPA entry for display (grouped by customer ASN)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AspaEntry {
    pub customer_asn: u32,
    pub providers: Vec<u32>,
}

/// ASPA entry for table display
#[derive(Debug, Clone, Tabled)]
pub struct AspaTableEntry {
    pub customer_asn: u32,
    pub providers: String,
}

impl From<&AspaEntry> for AspaTableEntry {
    fn from(entry: &AspaEntry) -> Self {
        AspaTableEntry {
            customer_asn: entry.customer_asn,
            providers: entry
                .providers
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// Parse RPKIviews collector from string
pub fn parse_rpkiviews_collector(collector: &str) -> Result<RpkiViewsCollector> {
    match collector.to_lowercase().as_str() {
        "soborost" | "soborostnet" => Ok(RpkiViewsCollector::SoborostNet),
        "massars" | "massarsnet" => Ok(RpkiViewsCollector::MassarsNet),
        "attn" | "attnjp" => Ok(RpkiViewsCollector::AttnJp),
        "kerfuffle" | "kerfufflenet" => Ok(RpkiViewsCollector::KerfuffleNet),
        _ => Err(anyhow!(
            "Unknown RPKIviews collector: {}. Valid options: soborost, massars, attn, kerfuffle",
            collector
        )),
    }
}

/// Parse historical RPKI source from strings
pub fn parse_historical_source(
    source: &str,
    collector: Option<&str>,
) -> Result<HistoricalRpkiSource> {
    match source.to_lowercase().as_str() {
        "ripe" => Ok(HistoricalRpkiSource::Ripe),
        "rpkiviews" => {
            let collector = collector.unwrap_or("soborost");
            let rpkiviews_collector = parse_rpkiviews_collector(collector)?;
            Ok(HistoricalRpkiSource::RpkiViews(rpkiviews_collector))
        }
        _ => Err(anyhow!(
            "Unknown RPKI source: {}. Valid options: ripe, rpkiviews",
            source
        )),
    }
}

/// Load current RPKI data from Cloudflare
pub fn load_current_rpki() -> Result<RpkiTrie> {
    RpkiTrie::from_cloudflare().map_err(|e| anyhow!("Failed to load current RPKI data: {}", e))
}

/// Load historical RPKI data for a specific date
pub fn load_historical_rpki(date: NaiveDate, source: HistoricalRpkiSource) -> Result<RpkiTrie> {
    match source {
        HistoricalRpkiSource::Ripe => RpkiTrie::from_ripe_historical(date)
            .map_err(|e| anyhow!("Failed to load RIPE historical RPKI data: {}", e)),
        HistoricalRpkiSource::RpkiViews(collector) => RpkiTrie::from_rpkiviews(collector, date)
            .map_err(|e| anyhow!("Failed to load RPKIviews RPKI data: {}", e)),
    }
}

/// Load RPKI data - current if no date provided, historical otherwise
pub fn load_rpki_data(
    date: Option<NaiveDate>,
    source: Option<&str>,
    collector: Option<&str>,
) -> Result<RpkiTrie> {
    match date {
        None => load_current_rpki(),
        Some(d) => {
            let source_str = source.unwrap_or("ripe");
            let historical_source = parse_historical_source(source_str, collector)?;
            load_historical_rpki(d, historical_source)
        }
    }
}

/// Get all ROAs, optionally filtered by prefix and/or origin ASN
pub fn get_roas(
    trie: &RpkiTrie,
    prefix_filter: Option<&str>,
    asn_filter: Option<u32>,
) -> Result<Vec<RoaEntry>> {
    let mut results: Vec<RoaEntry> = Vec::new();

    // If prefix filter is provided, look up ROAs for that prefix
    if let Some(prefix_str) = prefix_filter {
        let prefix = IpNet::from_str(prefix_str)
            .map_err(|e| anyhow!("Invalid prefix '{}': {}", prefix_str, e))?;

        let roas = trie.lookup_by_prefix(&prefix);
        for roa in roas {
            // Apply ASN filter if provided
            if let Some(asn) = asn_filter {
                if roa.asn != asn {
                    continue;
                }
            }
            results.push(RoaEntry {
                prefix: roa.prefix.to_string(),
                max_length: roa.max_length,
                origin_asn: roa.asn,
                ta: roa.rir.map(|r| format!("{:?}", r)).unwrap_or_default(),
            });
        }
    } else {
        // No prefix filter - iterate through all ROAs in the trie
        for (prefix, roas) in trie.trie.iter() {
            for roa in roas {
                // Apply ASN filter if provided
                if let Some(asn) = asn_filter {
                    if roa.asn != asn {
                        continue;
                    }
                }
                // Create RoaEntry with correct prefix from iteration
                results.push(RoaEntry {
                    prefix: prefix.to_string(),
                    max_length: roa.max_length,
                    origin_asn: roa.asn,
                    ta: roa.rir.map(|r| format!("{:?}", r)).unwrap_or_default(),
                });
            }
        }
    }

    // Sort by prefix for consistent output
    results.sort_by(|a, b| a.prefix.cmp(&b.prefix));

    Ok(results)
}

/// Get all ASPAs, optionally filtered by customer and/or provider ASN
/// Results are grouped by customer ASN with providers as a comma-separated list
pub fn get_aspas(
    trie: &RpkiTrie,
    customer_asn: Option<u32>,
    provider_asn: Option<u32>,
) -> Result<Vec<AspaEntry>> {
    let mut results: Vec<AspaEntry> = Vec::new();

    for aspa in &trie.aspas {
        // Apply customer ASN filter
        if let Some(customer) = customer_asn {
            if aspa.customer_asn != customer {
                continue;
            }
        }

        // Filter providers if provider filter is specified
        let filtered_providers: Vec<u32> = if let Some(prov_filter) = provider_asn {
            aspa.providers
                .iter()
                .copied()
                .filter(|&p| p == prov_filter)
                .collect()
        } else {
            aspa.providers.clone()
        };

        // Skip if no providers match the filter
        if filtered_providers.is_empty() {
            continue;
        }

        // Sort providers for consistent output
        let mut sorted_providers = filtered_providers;
        sorted_providers.sort();

        results.push(AspaEntry {
            customer_asn: aspa.customer_asn,
            providers: sorted_providers,
        });
    }

    // Sort by customer ASN
    results.sort_by_key(|a| a.customer_asn);

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rpkiviews_collector() {
        assert!(matches!(
            parse_rpkiviews_collector("soborost").unwrap(),
            RpkiViewsCollector::SoborostNet
        ));
        assert!(matches!(
            parse_rpkiviews_collector("kerfuffle").unwrap(),
            RpkiViewsCollector::KerfuffleNet
        ));
        assert!(parse_rpkiviews_collector("invalid").is_err());
    }

    #[test]
    fn test_parse_historical_source() {
        assert!(matches!(
            parse_historical_source("ripe", None).unwrap(),
            HistoricalRpkiSource::Ripe
        ));
        assert!(matches!(
            parse_historical_source("rpkiviews", Some("soborost")).unwrap(),
            HistoricalRpkiSource::RpkiViews(RpkiViewsCollector::SoborostNet)
        ));
    }
}
