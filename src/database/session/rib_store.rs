//! Working-state storage and SQLite export for reconstructed RIB snapshots.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use bgpkit_parser::models::ElemType;
use bgpkit_parser::BgpElem;
use rusqlite::params;

use crate::database::core::DatabaseConn;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RibRouteKey {
    pub collector: Arc<str>,
    pub peer_ip: IpAddr,
    pub peer_asn: u32,
    pub prefix: Arc<str>,
    pub path_id: Option<u32>,
}

impl RibRouteKey {
    pub fn from_elem(collector: Arc<str>, elem: &BgpElem) -> Self {
        Self {
            collector,
            peer_ip: elem.peer_ip,
            peer_asn: elem.peer_asn.to_u32(),
            prefix: Arc::from(elem.prefix.prefix.to_string().into_boxed_str()),
            path_id: elem.prefix.path_id,
        }
    }

    pub fn from_entry(entry: &StoredRibEntry) -> Self {
        Self {
            collector: Arc::clone(&entry.collector),
            peer_ip: entry.peer_ip,
            peer_asn: entry.peer_asn,
            prefix: Arc::clone(&entry.prefix),
            path_id: entry.path_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredRibEntry {
    pub collector: Arc<str>,
    pub timestamp: f64,
    pub peer_ip: IpAddr,
    pub peer_asn: u32,
    pub prefix: Arc<str>,
    pub path_id: Option<u32>,
    pub as_path: Option<String>,
    pub origin_asns: Option<Vec<u32>>,
}

impl StoredRibEntry {
    pub fn from_elem(collector: Arc<str>, elem: BgpElem) -> Self {
        Self {
            collector,
            timestamp: elem.timestamp,
            peer_ip: elem.peer_ip,
            peer_asn: elem.peer_asn.to_u32(),
            prefix: Arc::from(elem.prefix.prefix.to_string().into_boxed_str()),
            path_id: elem.prefix.path_id,
            as_path: elem.as_path.map(|path| path.to_string()),
            origin_asns: elem
                .origin_asns
                .map(|asns| asns.into_iter().map(|asn| asn.to_u32()).collect::<Vec<_>>()),
        }
    }

    pub fn route_key(&self) -> RibRouteKey {
        RibRouteKey::from_entry(self)
    }

    pub fn origin_asns_string(&self) -> Option<String> {
        self.origin_asns.as_ref().map(|asns| {
            asns.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        })
    }
}

/// Represents a filtered BGP update message that contributed to RIB reconstruction.
/// Stored in the `updates` table for 2nd and later RIB snapshots.
#[derive(Debug, Clone)]
pub struct StoredRibUpdate {
    /// The target RIB timestamp this update contributed to
    pub rib_ts: i64,
    /// When the update message was received
    pub timestamp: f64,
    pub collector: Arc<str>,
    pub peer_ip: IpAddr,
    pub peer_asn: u32,
    pub prefix: Arc<str>,
    pub path_id: Option<u32>,
    pub as_path: Option<String>,
    pub origin_asns: Option<Vec<u32>>,
    /// The type of BGP message (ANNOUNCE or WITHDRAW)
    pub elem_type: ElemType,
}

impl StoredRibUpdate {
    pub fn from_elem(rib_ts: i64, collector: Arc<str>, elem: BgpElem, elem_type: ElemType) -> Self {
        Self {
            rib_ts,
            collector,
            timestamp: elem.timestamp,
            peer_ip: elem.peer_ip,
            peer_asn: elem.peer_asn.to_u32(),
            prefix: Arc::from(elem.prefix.prefix.to_string().into_boxed_str()),
            path_id: elem.prefix.path_id,
            as_path: elem.as_path.map(|path| path.to_string()),
            origin_asns: elem
                .origin_asns
                .map(|asns| asns.into_iter().map(|asn| asn.to_u32()).collect::<Vec<_>>()),
            elem_type,
        }
    }

    pub fn origin_asns_string(&self) -> Option<String> {
        self.origin_asns.as_ref().map(|asns| {
            asns.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        })
    }
}

pub struct RibStateStore {
    entries: HashMap<RibRouteKey, StoredRibEntry>,
}

impl RibStateStore {
    pub fn new_temp() -> Result<Self> {
        Ok(Self {
            entries: HashMap::new(),
        })
    }

    pub fn count(&self) -> Result<u64> {
        Ok(self.entries.len() as u64)
    }

    pub fn route_exists(&self, key: &RibRouteKey) -> Result<bool> {
        Ok(self.entries.contains_key(key))
    }

    pub fn upsert_entry(&mut self, entry: StoredRibEntry) -> Result<()> {
        self.upsert_entries(vec![entry])
    }

    pub fn upsert_entries<I>(&mut self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = StoredRibEntry>,
    {
        for entry in entries {
            self.entries.insert(entry.route_key(), entry);
        }
        Ok(())
    }

    pub fn delete_key(&mut self, key: &RibRouteKey) -> Result<()> {
        self.entries.remove(key);
        Ok(())
    }

    pub fn delete_keys<I>(&mut self, keys: I) -> Result<()>
    where
        I: IntoIterator<Item = RibRouteKey>,
    {
        for key in keys {
            self.entries.remove(&key);
        }
        Ok(())
    }

    pub fn visit_entries<F>(&self, mut visitor: F) -> Result<()>
    where
        F: FnMut(&StoredRibEntry) -> Result<()>,
    {
        for entry in self.entries.values() {
            visitor(entry)?;
        }
        Ok(())
    }
}

/// SQLite storage for RIB reconstruction output with two tables:
///
/// - `ribs`: Stores final reconstructed RIB states at each target timestamp.
///   Contains one row per (rib_ts, route) showing the routing table state.
///
/// - `updates`: Stores filtered BGP update messages that were applied to build
///   subsequent RIB snapshots. Only populated for 2nd and later RIBs.
///   Shows the incremental changes between snapshots.
pub struct RibSqliteStore {
    db: DatabaseConn,
    /// Tracks which snapshot index we're processing (0 = first RIB)
    snapshot_index: usize,
}

impl RibSqliteStore {
    pub fn new(db_path: &str, reset: bool) -> Result<Self> {
        let db = DatabaseConn::open_path(db_path)?;
        let store = Self {
            db,
            snapshot_index: 0,
        };
        store.initialize(reset)?;
        Ok(store)
    }

    fn initialize(&self, reset: bool) -> Result<()> {
        if reset {
            self.db
                .conn
                .execute("DROP TABLE IF EXISTS ribs", [])
                .map_err(|e| anyhow!("Failed to drop existing ribs table: {}", e))?;
            self.db
                .conn
                .execute("DROP TABLE IF EXISTS updates", [])
                .map_err(|e| anyhow!("Failed to drop existing updates table: {}", e))?;
        }

        self.db
            .conn
            .execute_batch(
                r#"
                -- Final reconstructed RIB states at each target timestamp
                -- One row per (rib_ts, route_key) showing the routing table state
                CREATE TABLE IF NOT EXISTS ribs (
                    rib_ts INTEGER NOT NULL,
                    timestamp REAL NOT NULL,
                    collector TEXT NOT NULL,
                    peer_ip TEXT NOT NULL,
                    peer_asn INTEGER NOT NULL,
                    prefix TEXT NOT NULL,
                    path_id INTEGER,
                    as_path TEXT,
                    origin_asns TEXT
                );

                -- Filtered BGP updates used to build 2nd and later RIB snapshots
                -- Only populated when multiple rib_ts are requested
                -- Shows incremental changes between consecutive RIB states
                CREATE TABLE IF NOT EXISTS updates (
                    rib_ts INTEGER NOT NULL,
                    timestamp REAL NOT NULL,
                    collector TEXT NOT NULL,
                    peer_ip TEXT NOT NULL,
                    peer_asn INTEGER NOT NULL,
                    prefix TEXT NOT NULL,
                    path_id INTEGER,
                    as_path TEXT,
                    origin_asns TEXT,
                    elem_type TEXT NOT NULL
                );
                "#,
            )
            .map_err(|e| anyhow!("Failed to initialize RIB SQLite schema: {}", e))?;
        Ok(())
    }

    /// Insert a RIB snapshot and its associated filtered updates.
    ///
    /// # Arguments
    /// * `rib_ts` - The target RIB timestamp
    /// * `state_store` - The final RIB state to store
    /// * `filtered_updates` - Updates that contributed to this RIB (only stored for 2nd+ RIBs)
    pub fn insert_snapshot(
        &mut self,
        rib_ts: i64,
        state_store: &RibStateStore,
        filtered_updates: &[StoredRibUpdate],
    ) -> Result<()> {
        let tx = self
            .db
            .conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("Failed to begin RIB output transaction: {}", e))?;

        // Insert RIB entries into 'ribs' table (always populated)
        {
            let mut rib_stmt = tx
                .prepare_cached(
                    r#"
                    INSERT INTO ribs (
                        rib_ts, timestamp, collector, peer_ip, peer_asn, 
                        prefix, path_id, as_path, origin_asns
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    "#,
                )
                .map_err(|e| anyhow!("Failed to prepare ribs insert statement: {}", e))?;

            state_store.visit_entries(|entry| {
                rib_stmt
                    .execute(params![
                        rib_ts,
                        entry.timestamp,
                        entry.collector,
                        entry.peer_ip.to_string(),
                        entry.peer_asn,
                        entry.prefix,
                        entry.path_id,
                        entry.as_path,
                        entry.origin_asns_string(),
                    ])
                    .map_err(|e| anyhow!("Failed to insert into ribs table: {}", e))?;
                Ok(())
            })?;
        }

        // Insert filtered updates into 'updates' table (only for 2nd and later RIBs)
        if self.snapshot_index > 0 && !filtered_updates.is_empty() {
            let mut update_stmt = tx
                .prepare_cached(
                    r#"
                    INSERT INTO updates (
                        rib_ts, timestamp, collector, peer_ip, peer_asn,
                        prefix, path_id, as_path, origin_asns, elem_type
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                    "#,
                )
                .map_err(|e| anyhow!("Failed to prepare updates insert statement: {}", e))?;

            for update in filtered_updates {
                let elem_type_str = match update.elem_type {
                    ElemType::ANNOUNCE => "ANNOUNCE",
                    ElemType::WITHDRAW => "WITHDRAW",
                };

                update_stmt
                    .execute(params![
                        update.rib_ts,
                        update.timestamp,
                        update.collector,
                        update.peer_ip.to_string(),
                        update.peer_asn,
                        update.prefix,
                        update.path_id,
                        update.as_path,
                        update.origin_asns_string(),
                        elem_type_str,
                    ])
                    .map_err(|e| anyhow!("Failed to insert into updates table: {}", e))?;
            }
        }

        tx.commit()
            .map_err(|e| anyhow!("Failed to commit RIB output transaction: {}", e))?;
        self.snapshot_index += 1;
        Ok(())
    }

    pub fn finalize_indexes(&self) -> Result<()> {
        self.db
            .conn
            .execute_batch(
                r#"
                -- Indexes for ribs table (final RIB states)
                CREATE INDEX IF NOT EXISTS idx_ribs_rib_ts ON ribs(rib_ts);
                CREATE INDEX IF NOT EXISTS idx_ribs_rib_ts_prefix ON ribs(rib_ts, prefix);
                CREATE INDEX IF NOT EXISTS idx_ribs_rib_ts_peer_asn ON ribs(rib_ts, peer_asn);
                CREATE INDEX IF NOT EXISTS idx_ribs_rib_ts_collector ON ribs(rib_ts, collector);

                -- Indexes for updates table (intermediate changes)
                CREATE INDEX IF NOT EXISTS idx_updates_rib_ts ON updates(rib_ts);
                CREATE INDEX IF NOT EXISTS idx_updates_rib_ts_prefix ON updates(rib_ts, prefix);
                CREATE INDEX IF NOT EXISTS idx_updates_timestamp ON updates(timestamp);
                CREATE INDEX IF NOT EXISTS idx_updates_collector ON updates(collector);
                "#,
            )
            .map_err(|e| anyhow!("Failed to create RIB SQLite indexes: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgpkit_parser::models::{AsPath, AsPathSegment, ElemType, NetworkPrefix};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    fn test_elem() -> Result<BgpElem> {
        Ok(BgpElem {
            timestamp: 1234.0,
            elem_type: ElemType::ANNOUNCE,
            peer_ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            peer_asn: 64496.into(),
            peer_bgp_id: None,
            prefix: NetworkPrefix::new("203.0.113.0/24".parse()?, Some(7)),
            next_hop: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2))),
            as_path: Some(AsPath {
                segments: vec![AsPathSegment::AsSequence(
                    vec![64496.into(), 64497.into()].into(),
                )]
                .into(),
            }),
            origin_asns: Some(vec![64497.into()]),
            origin: None,
            local_pref: Some(100),
            med: Some(50),
            communities: None,
            atomic: false,
            aggr_asn: None,
            aggr_ip: None,
            only_to_customer: None,
            unknown: None,
            deprecated: None,
        })
    }

    #[test]
    fn test_rib_state_store_round_trip() -> Result<()> {
        let mut store = RibStateStore::new_temp()?;
        let entry = StoredRibEntry::from_elem(Arc::from("rrc00"), test_elem()?);
        store.upsert_entry(entry.clone())?;
        assert!(store.route_exists(&entry.route_key())?);

        let mut visited = Vec::new();
        store.visit_entries(|entry| {
            visited.push(entry.clone());
            Ok(())
        })?;

        assert_eq!(visited.len(), 1);
        assert_eq!(visited[0].collector.as_ref(), "rrc00");
        assert_eq!(visited[0].path_id, Some(7));
        Ok(())
    }

    #[test]
    fn test_sqlite_store_two_tables() -> Result<()> {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path().to_str().unwrap();

        let mut store = RibSqliteStore::new(path, true)?;

        // Create first RIB snapshot (no updates should be stored)
        let mut state1 = RibStateStore::new_temp()?;
        let entry1 = StoredRibEntry::from_elem(Arc::from("rrc00"), test_elem()?);
        state1.upsert_entry(entry1)?;

        // First RIB: no updates stored
        store.insert_snapshot(1704067200, &state1, &[])?;

        // Create second RIB snapshot with updates
        let mut state2 = RibStateStore::new_temp()?;
        let entry2 = StoredRibEntry::from_elem(Arc::from("rrc00"), test_elem()?);
        state2.upsert_entry(entry2)?;

        let update = StoredRibUpdate::from_elem(
            1704069000,
            Arc::from("rrc00"),
            test_elem()?,
            ElemType::ANNOUNCE,
        );

        // Second RIB: updates should be stored
        store.insert_snapshot(1704069000, &state2, &[update])?;

        store.finalize_indexes()?;

        // Verify tables exist and have correct data
        let rib_count: i64 = store
            .db
            .conn
            .query_row("SELECT COUNT(*) FROM ribs", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to count ribs: {}", e))?;

        let update_count: i64 = store
            .db
            .conn
            .query_row("SELECT COUNT(*) FROM updates", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to count updates: {}", e))?;

        // Both RIBs stored in ribs table
        assert_eq!(rib_count, 2);
        // Only 2nd RIB has updates stored
        assert_eq!(update_count, 1);

        Ok(())
    }
}
