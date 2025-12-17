//! Session-based message store for BGP search results
//!
//! This module provides storage for BGP messages during search operations.
//! Unlike the shared database, this is intended for one-time use per search session.

use anyhow::{anyhow, Result};
use bgpkit_parser::models::ElemType;
use bgpkit_parser::BgpElem;
use itertools::Itertools;

use crate::database::core::DatabaseConn;

/// Message store for BGP search results
///
/// `MsgStore` provides a session-based SQLite database for storing
/// BGP elements during search operations. Each search session typically
/// creates its own database file.
pub struct MsgStore {
    db: DatabaseConn,
}

impl MsgStore {
    /// Create a new message store
    ///
    /// # Arguments
    /// * `db_path` - Optional path to the database file. If `None`, uses in-memory storage.
    /// * `reset` - If `true`, drops existing data before initializing.
    pub fn new(db_path: Option<&str>, reset: bool) -> Result<Self> {
        let db = DatabaseConn::open(db_path)?;
        let store = MsgStore { db };
        store.initialize(reset)?;
        Ok(store)
    }

    /// Create a new message store (backward-compatible signature)
    ///
    /// This method accepts `&Option<String>` for compatibility with existing code.
    /// Prefer using `new()` with `Option<&str>` for new code.
    pub fn new_from_option(db_path: &Option<String>, reset: bool) -> Result<Self> {
        Self::new(db_path.as_deref(), reset)
    }

    /// Initialize the message store schema
    fn initialize(&self, reset: bool) -> Result<()> {
        if reset {
            self.db
                .conn
                .execute("DROP TABLE IF EXISTS elems", [])
                .map_err(|e| anyhow!("Failed to drop elems table: {}", e))?;
        }

        self.db
            .conn
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS elems (
                    timestamp INTEGER,
                    elem_type TEXT,
                    collector TEXT,
                    peer_ip TEXT,
                    peer_asn INTEGER,
                    prefix TEXT,
                    next_hop TEXT,
                    as_path TEXT,
                    origin_asns TEXT,
                    origin TEXT,
                    local_pref INTEGER,
                    med INTEGER,
                    communities TEXT,
                    atomic TEXT,
                    aggr_asn INTEGER,
                    aggr_ip TEXT
                );
                "#,
                [],
            )
            .map_err(|e| anyhow!("Failed to create elems table: {}", e))?;

        // Add indexes for common query patterns
        self.db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON elems(timestamp)",
            [],
        )?;
        self.db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_peer_asn ON elems(peer_asn)",
            [],
        )?;
        self.db
            .conn
            .execute("CREATE INDEX IF NOT EXISTS idx_prefix ON elems(prefix)", [])?;
        self.db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_collector ON elems(collector)",
            [],
        )?;
        self.db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_elem_type ON elems(elem_type)",
            [],
        )?;

        Ok(())
    }

    /// Insert BGP elements into the store
    ///
    /// # Arguments
    /// * `elems` - Slice of (BgpElem, collector_name) tuples
    pub fn insert_elems(&self, elems: &[(BgpElem, String)]) -> Result<()> {
        if elems.is_empty() {
            return Ok(());
        }

        // Use a transaction for the batch
        let tx = self
            .db
            .conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("Failed to begin transaction: {}", e))?;

        {
            // Use prepared statement for better performance
            let mut stmt = tx
                .prepare_cached(
                    "INSERT INTO elems (timestamp, elem_type, collector, peer_ip, peer_asn,
                     prefix, next_hop, as_path, origin_asns, origin, local_pref, med,
                     communities, atomic, aggr_asn, aggr_ip)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                )
                .map_err(|e| anyhow!("Failed to prepare statement: {}", e))?;

            for (elem, collector) in elems {
                let elem_type = match elem.elem_type {
                    ElemType::ANNOUNCE => "A",
                    ElemType::WITHDRAW => "W",
                };
                let origin_string = elem
                    .origin_asns
                    .as_ref()
                    .and_then(|asns| asns.first())
                    .map(|asn| asn.to_string());
                let communities_str = elem.communities.as_ref().map(|v| v.iter().join(" "));

                stmt.execute(rusqlite::params![
                    elem.timestamp as u32,
                    elem_type,
                    collector,
                    elem.peer_ip.to_string(),
                    elem.peer_asn.to_u32(),
                    elem.prefix.to_string(),
                    elem.next_hop.as_ref().map(|v| v.to_string()),
                    elem.as_path.as_ref().map(|v| v.to_string()),
                    origin_string,
                    elem.origin.as_ref().map(|v| v.to_string()),
                    elem.local_pref,
                    elem.med,
                    communities_str,
                    if elem.atomic { "AG" } else { "NAG" },
                    elem.aggr_asn.map(|asn| asn.to_u32()),
                    elem.aggr_ip.as_ref().map(|v| v.to_string()),
                ])
                .map_err(|e| anyhow!("Failed to insert element: {}", e))?;
            }
        }

        tx.commit()
            .map_err(|e| anyhow!("Failed to commit transaction: {}", e))?;
        Ok(())
    }

    /// Get the count of stored elements
    pub fn count(&self) -> Result<u64> {
        self.db.table_count("elems")
    }

    /// Get access to the underlying database connection for custom queries
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.db.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgpkit_parser::models::{AsPath, AsPathSegment, NetworkPrefix, Origin};
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;

    fn create_test_elem() -> BgpElem {
        BgpElem {
            timestamp: 1234567890.0,
            elem_type: ElemType::ANNOUNCE,
            peer_ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            peer_asn: 65000.into(),
            prefix: NetworkPrefix::from_str("10.0.0.0/8").unwrap(),
            next_hop: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
            as_path: Some(AsPath {
                segments: vec![AsPathSegment::AsSequence(vec![65000.into(), 65001.into()])],
            }),
            origin_asns: Some(vec![65001.into()]),
            origin: Some(Origin::IGP),
            local_pref: Some(100),
            med: Some(0),
            communities: None,
            atomic: false,
            aggr_asn: None,
            aggr_ip: None,
            only_to_customer: None,
            unknown: None,
            deprecated: None,
        }
    }

    #[test]
    fn test_create_msg_store() {
        let store = MsgStore::new(None, false);
        assert!(store.is_ok());
    }

    #[test]
    fn test_insert_and_count() {
        let store = MsgStore::new(None, false).unwrap();

        let elem = create_test_elem();
        let elems = vec![(elem, "test_collector".to_string())];

        store.insert_elems(&elems).unwrap();

        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn test_reset() {
        let store = MsgStore::new(None, false).unwrap();

        let elem = create_test_elem();
        let elems = vec![(elem, "test_collector".to_string())];

        store.insert_elems(&elems).unwrap();
        assert_eq!(store.count().unwrap(), 1);

        // Create new store with reset
        let store2 = MsgStore::new(None, true).unwrap();
        assert_eq!(store2.count().unwrap(), 0);
    }
}
