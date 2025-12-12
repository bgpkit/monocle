//! File-based cache for RPKI and Pfx2as data
//!
//! This module provides a simple file-based caching system for data that requires
//! INET-level operations (prefix matching, containment queries) that SQLite doesn't
//! natively support. The cache stores JSON files with timestamps encoded in filenames.
//!
//! # Cache File Naming
//!
//! Files are named with the pattern: `{type}_{source}_{timestamp}.json`
//! - RPKI: `rpki_{source}_{date}_{timestamp}.json` (date optional, for historical data)
//! - Pfx2as: `pfx2as_{source_hash}_{timestamp}.json`
//!
//! The timestamp is in RFC 3339 format (URL-safe encoded).

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::info;

/// Default TTL for RPKI current data (1 hour)
pub const DEFAULT_RPKI_TTL: Duration = Duration::from_secs(60 * 60);

/// Default TTL for Pfx2as data (24 hours)
pub const DEFAULT_PFX2AS_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Default TTL for RPKI historical data (7 days - historical data doesn't change)
pub const DEFAULT_RPKI_HISTORICAL_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

// =============================================================================
// RPKI Cache
// =============================================================================

/// ROA (Route Origin Authorization) record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoaRecord {
    pub prefix: String,
    pub max_length: u8,
    pub origin_asn: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ta: Option<String>,
}

/// ASPA (Autonomous System Provider Authorization) record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AspaRecord {
    pub customer_asn: u32,
    pub provider_asns: Vec<u32>,
}

/// Cached RPKI data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiCacheData {
    /// Cache metadata
    pub meta: RpkiCacheMeta,
    /// ROA records
    pub roas: Vec<RoaRecord>,
    /// ASPA records
    pub aspas: Vec<AspaRecord>,
}

/// Metadata for RPKI cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiCacheMeta {
    /// Data source identifier (e.g., "cloudflare", "ripe", "rpkiviews")
    pub source: String,
    /// Date of the data (for historical data)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_date: Option<NaiveDate>,
    /// When the cache was created
    pub cached_at: DateTime<Utc>,
    /// Number of ROA records
    pub roa_count: usize,
    /// Number of ASPA records
    pub aspa_count: usize,
}

/// RPKI file cache manager
pub struct RpkiFileCache {
    cache_dir: PathBuf,
}

impl RpkiFileCache {
    /// Create a new RPKI file cache
    pub fn new(data_dir: &str) -> Result<Self> {
        let cache_dir = PathBuf::from(data_dir).join("cache").join("rpki");
        fs::create_dir_all(&cache_dir)
            .map_err(|e| anyhow!("Failed to create RPKI cache directory: {}", e))?;
        Ok(Self { cache_dir })
    }

    /// Generate cache filename for RPKI data
    fn cache_filename(&self, source: &str, data_date: Option<NaiveDate>) -> String {
        let safe_source = source.replace(['/', ':', '.'], "_");
        match data_date {
            Some(date) => format!("rpki_{}_{}.json", safe_source, date.format("%Y-%m-%d")),
            None => format!("rpki_{}_current.json", safe_source),
        }
    }

    /// Get cache file path
    fn cache_path(&self, source: &str, data_date: Option<NaiveDate>) -> PathBuf {
        self.cache_dir.join(self.cache_filename(source, data_date))
    }

    /// Check if cached data exists and is fresh
    pub fn is_fresh(&self, source: &str, data_date: Option<NaiveDate>, ttl: Duration) -> bool {
        let path = self.cache_path(source, data_date);
        if !path.exists() {
            return false;
        }

        // Read and check the cache metadata
        if let Ok(data) = self.load(source, data_date) {
            let age = Utc::now().signed_duration_since(data.meta.cached_at);
            return age.num_seconds() < ttl.as_secs() as i64;
        }

        false
    }

    /// Load cached RPKI data
    pub fn load(&self, source: &str, data_date: Option<NaiveDate>) -> Result<RpkiCacheData> {
        let path = self.cache_path(source, data_date);
        let content = fs::read_to_string(&path)
            .map_err(|e| anyhow!("Failed to read RPKI cache file {:?}: {}", path, e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse RPKI cache file {:?}: {}", path, e))
    }

    /// Store RPKI data to cache
    pub fn store(
        &self,
        source: &str,
        data_date: Option<NaiveDate>,
        roas: Vec<RoaRecord>,
        aspas: Vec<AspaRecord>,
    ) -> Result<()> {
        let path = self.cache_path(source, data_date);

        let data = RpkiCacheData {
            meta: RpkiCacheMeta {
                source: source.to_string(),
                data_date,
                cached_at: Utc::now(),
                roa_count: roas.len(),
                aspa_count: aspas.len(),
            },
            roas,
            aspas,
        };

        let content = serde_json::to_string_pretty(&data)
            .map_err(|e| anyhow!("Failed to serialize RPKI cache: {}", e))?;

        fs::write(&path, content)
            .map_err(|e| anyhow!("Failed to write RPKI cache file {:?}: {}", path, e))?;

        info!(
            "Cached {} ROAs and {} ASPAs to {:?}",
            data.meta.roa_count, data.meta.aspa_count, path
        );

        Ok(())
    }

    /// Get cache metadata without loading all data
    pub fn get_meta(&self, source: &str, data_date: Option<NaiveDate>) -> Option<RpkiCacheMeta> {
        self.load(source, data_date).ok().map(|d| d.meta)
    }

    /// List all cached RPKI data
    pub fn list_cached(&self) -> Result<Vec<RpkiCacheMeta>> {
        let mut results = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(data) = serde_json::from_str::<RpkiCacheData>(&content) {
                            results.push(data.meta);
                        }
                    }
                }
            }
        }

        results.sort_by(|a, b| b.cached_at.cmp(&a.cached_at));
        Ok(results)
    }

    /// Clear cache for a specific source/date
    pub fn clear(&self, source: &str, data_date: Option<NaiveDate>) -> Result<()> {
        let path = self.cache_path(source, data_date);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| anyhow!("Failed to remove cache file {:?}: {}", path, e))?;
        }
        Ok(())
    }

    /// Clear all RPKI cache
    pub fn clear_all(&self) -> Result<()> {
        if self.cache_dir.exists() {
            for entry in fs::read_dir(&self.cache_dir)?.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "json") {
                    fs::remove_file(&path)?;
                }
            }
        }
        Ok(())
    }
}

// =============================================================================
// Pfx2as Cache
// =============================================================================

/// Pfx2as record representing a prefix-to-origin mapping
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asRecord {
    pub prefix: String,
    pub origin_asns: Vec<u32>,
}

/// Cached Pfx2as data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asCacheData {
    /// Cache metadata
    pub meta: Pfx2asCacheMeta,
    /// Prefix-to-AS records
    pub records: Vec<Pfx2asRecord>,
}

/// Metadata for Pfx2as cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asCacheMeta {
    /// Data source URL or identifier
    pub source: String,
    /// When the cache was created
    pub cached_at: DateTime<Utc>,
    /// Number of records
    pub record_count: usize,
}

/// Pfx2as file cache manager
pub struct Pfx2asFileCache {
    cache_dir: PathBuf,
}

impl Pfx2asFileCache {
    /// Create a new Pfx2as file cache
    pub fn new(data_dir: &str) -> Result<Self> {
        let cache_dir = PathBuf::from(data_dir).join("cache").join("pfx2as");
        fs::create_dir_all(&cache_dir)
            .map_err(|e| anyhow!("Failed to create Pfx2as cache directory: {}", e))?;
        Ok(Self { cache_dir })
    }

    /// Generate a safe filename from the source URL
    fn source_to_filename(source: &str) -> String {
        // Create a simple hash-like identifier from the source
        let safe_name: String = source
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(50)
            .collect();

        if safe_name.is_empty() {
            "default".to_string()
        } else {
            safe_name
        }
    }

    /// Generate cache filename for Pfx2as data
    fn cache_filename(&self, source: &str) -> String {
        format!("pfx2as_{}.json", Self::source_to_filename(source))
    }

    /// Get cache file path
    fn cache_path(&self, source: &str) -> PathBuf {
        self.cache_dir.join(self.cache_filename(source))
    }

    /// Check if cached data exists and is fresh
    pub fn is_fresh(&self, source: &str, ttl: Duration) -> bool {
        let path = self.cache_path(source);
        if !path.exists() {
            return false;
        }

        // Read and check the cache metadata
        if let Ok(data) = self.load(source) {
            let age = Utc::now().signed_duration_since(data.meta.cached_at);
            return age.num_seconds() < ttl.as_secs() as i64;
        }

        false
    }

    /// Load cached Pfx2as data
    pub fn load(&self, source: &str) -> Result<Pfx2asCacheData> {
        let path = self.cache_path(source);
        let content = fs::read_to_string(&path)
            .map_err(|e| anyhow!("Failed to read Pfx2as cache file {:?}: {}", path, e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse Pfx2as cache file {:?}: {}", path, e))
    }

    /// Store Pfx2as data to cache
    pub fn store(&self, source: &str, records: Vec<Pfx2asRecord>) -> Result<()> {
        let path = self.cache_path(source);

        let data = Pfx2asCacheData {
            meta: Pfx2asCacheMeta {
                source: source.to_string(),
                cached_at: Utc::now(),
                record_count: records.len(),
            },
            records,
        };

        let content = serde_json::to_string(&data)
            .map_err(|e| anyhow!("Failed to serialize Pfx2as cache: {}", e))?;

        fs::write(&path, content)
            .map_err(|e| anyhow!("Failed to write Pfx2as cache file {:?}: {}", path, e))?;

        info!(
            "Cached {} Pfx2as records to {:?}",
            data.meta.record_count, path
        );

        Ok(())
    }

    /// Get cache metadata without loading all data
    pub fn get_meta(&self, source: &str) -> Option<Pfx2asCacheMeta> {
        self.load(source).ok().map(|d| d.meta)
    }

    /// List all cached Pfx2as data
    pub fn list_cached(&self) -> Result<Vec<Pfx2asCacheMeta>> {
        let mut results = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(data) = serde_json::from_str::<Pfx2asCacheData>(&content) {
                            results.push(data.meta);
                        }
                    }
                }
            }
        }

        results.sort_by(|a, b| b.cached_at.cmp(&a.cached_at));
        Ok(results)
    }

    /// Clear cache for a specific source
    pub fn clear(&self, source: &str) -> Result<()> {
        let path = self.cache_path(source);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| anyhow!("Failed to remove cache file {:?}: {}", path, e))?;
        }
        Ok(())
    }

    /// Clear all Pfx2as cache
    pub fn clear_all(&self) -> Result<()> {
        if self.cache_dir.exists() {
            for entry in fs::read_dir(&self.cache_dir)?.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "json") {
                    fs::remove_file(&path)?;
                }
            }
        }
        Ok(())
    }

    /// Build a prefix-to-ASN map from cached data for in-memory lookups
    pub fn build_prefix_map(&self, source: &str) -> Result<HashMap<String, Vec<u32>>> {
        let data = self.load(source)?;
        let mut map = HashMap::with_capacity(data.records.len());
        for record in data.records {
            map.insert(record.prefix, record.origin_asns);
        }
        Ok(map)
    }
}

// =============================================================================
// Cache Directory Management
// =============================================================================

/// Ensure the cache directory structure exists
pub fn ensure_cache_dirs(data_dir: &str) -> Result<()> {
    let cache_base = PathBuf::from(data_dir).join("cache");
    fs::create_dir_all(cache_base.join("rpki"))
        .map_err(|e| anyhow!("Failed to create RPKI cache directory: {}", e))?;
    fs::create_dir_all(cache_base.join("pfx2as"))
        .map_err(|e| anyhow!("Failed to create Pfx2as cache directory: {}", e))?;
    Ok(())
}

/// Get total cache size in bytes
pub fn cache_size(data_dir: &str) -> Result<u64> {
    let cache_base = PathBuf::from(data_dir).join("cache");
    let mut total = 0u64;

    fn dir_size(path: &Path) -> u64 {
        let mut size = 0u64;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    size += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                } else if path.is_dir() {
                    size += dir_size(&path);
                }
            }
        }
        size
    }

    if cache_base.exists() {
        total = dir_size(&cache_base);
    }

    Ok(total)
}

/// Clear all caches
pub fn clear_all_caches(data_dir: &str) -> Result<()> {
    let rpki_cache = RpkiFileCache::new(data_dir)?;
    rpki_cache.clear_all()?;

    let pfx2as_cache = Pfx2asFileCache::new(data_dir)?;
    pfx2as_cache.clear_all()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_rpki_cache_store_and_load() {
        let temp_dir = setup_test_dir();
        let cache = RpkiFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        let roas = vec![RoaRecord {
            prefix: "10.0.0.0/8".to_string(),
            max_length: 24,
            origin_asn: 65000,
            ta: Some("test".to_string()),
        }];
        let aspas = vec![AspaRecord {
            customer_asn: 65001,
            provider_asns: vec![65000, 65002],
        }];

        cache
            .store("test-source", None, roas.clone(), aspas.clone())
            .unwrap();

        let loaded = cache.load("test-source", None).unwrap();
        assert_eq!(loaded.meta.source, "test-source");
        assert_eq!(loaded.meta.roa_count, 1);
        assert_eq!(loaded.meta.aspa_count, 1);
        assert_eq!(loaded.roas.len(), 1);
        assert_eq!(loaded.aspas.len(), 1);
    }

    #[test]
    fn test_rpki_cache_freshness() {
        let temp_dir = setup_test_dir();
        let cache = RpkiFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        // No cache yet
        assert!(!cache.is_fresh("test", None, DEFAULT_RPKI_TTL));

        // Store some data
        cache.store("test", None, vec![], vec![]).unwrap();

        // Should be fresh
        assert!(cache.is_fresh("test", None, DEFAULT_RPKI_TTL));

        // Should be stale with 0 TTL
        assert!(!cache.is_fresh("test", None, Duration::ZERO));
    }

    #[test]
    fn test_rpki_cache_historical() {
        let temp_dir = setup_test_dir();
        let cache = RpkiFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        cache.store("ripe", Some(date), vec![], vec![]).unwrap();

        let loaded = cache.load("ripe", Some(date)).unwrap();
        assert_eq!(loaded.meta.data_date, Some(date));
    }

    #[test]
    fn test_pfx2as_cache_store_and_load() {
        let temp_dir = setup_test_dir();
        let cache = Pfx2asFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65001, 65002],
            },
        ];

        cache
            .store("https://example.com/data.json", records)
            .unwrap();

        let loaded = cache.load("https://example.com/data.json").unwrap();
        assert_eq!(loaded.meta.record_count, 2);
        assert_eq!(loaded.records.len(), 2);
    }

    #[test]
    fn test_pfx2as_cache_freshness() {
        let temp_dir = setup_test_dir();
        let cache = Pfx2asFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        assert!(!cache.is_fresh("test-source", DEFAULT_PFX2AS_TTL));

        cache.store("test-source", vec![]).unwrap();

        assert!(cache.is_fresh("test-source", DEFAULT_PFX2AS_TTL));
        assert!(!cache.is_fresh("test-source", Duration::ZERO));
    }

    #[test]
    fn test_pfx2as_build_prefix_map() {
        let temp_dir = setup_test_dir();
        let cache = Pfx2asFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65001, 65002],
            },
        ];

        cache.store("test", records).unwrap();

        let map = cache.build_prefix_map("test").unwrap();
        assert_eq!(map.get("10.0.0.0/8"), Some(&vec![65000]));
        assert_eq!(map.get("192.168.0.0/16"), Some(&vec![65001, 65002]));
    }

    #[test]
    fn test_cache_clear() {
        let temp_dir = setup_test_dir();
        let rpki_cache = RpkiFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();
        let pfx2as_cache = Pfx2asFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        rpki_cache.store("test", None, vec![], vec![]).unwrap();
        pfx2as_cache.store("test", vec![]).unwrap();

        assert!(rpki_cache.load("test", None).is_ok());
        assert!(pfx2as_cache.load("test").is_ok());

        rpki_cache.clear("test", None).unwrap();
        pfx2as_cache.clear("test").unwrap();

        assert!(rpki_cache.load("test", None).is_err());
        assert!(pfx2as_cache.load("test").is_err());
    }

    #[test]
    fn test_list_cached() {
        let temp_dir = setup_test_dir();
        let cache = RpkiFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();

        cache.store("source1", None, vec![], vec![]).unwrap();
        cache.store("source2", None, vec![], vec![]).unwrap();

        let list = cache.list_cached().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_ensure_cache_dirs() {
        let temp_dir = setup_test_dir();
        ensure_cache_dirs(temp_dir.path().to_str().unwrap()).unwrap();

        assert!(temp_dir.path().join("cache/rpki").exists());
        assert!(temp_dir.path().join("cache/pfx2as").exists());
    }
}
