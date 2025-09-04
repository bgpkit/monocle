use anyhow::{anyhow, Result};
use bgpkit_parser::models::ElemType;
use bgpkit_parser::BgpElem;
use itertools::Itertools;
use rusqlite::Connection;

pub struct MonocleDatabase {
    pub conn: Connection,
}

impl MonocleDatabase {
    pub fn new(path: &Option<String>) -> Result<MonocleDatabase> {
        let conn = match path {
            Some(p) => Connection::open(p.as_str())?,
            None => Connection::open_in_memory()?,
        };
        Ok(MonocleDatabase { conn })
    }
}

pub struct MsgStore {
    db: MonocleDatabase,
}

impl MsgStore {
    pub fn new(db_path: &Option<String>, reset: bool) -> Result<MsgStore> {
        let mut db = MonocleDatabase::new(db_path)?;
        Self::initialize_msgs_db(&mut db, reset)?;
        Ok(MsgStore { db })
    }

    fn initialize_msgs_db(db: &mut MonocleDatabase, reset: bool) -> Result<()> {
        if reset {
            db.conn.execute("drop table if exists elems", [])?;
        }
        db.conn.execute(
            r#"
        create table if not exists elems (
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
        )?;

        // Add indexes for common query patterns
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON elems(timestamp)",
            [],
        )?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_peer_asn ON elems(peer_asn)",
            [],
        )?;
        db.conn
            .execute("CREATE INDEX IF NOT EXISTS idx_prefix ON elems(prefix)", [])?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_collector ON elems(collector)",
            [],
        )?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_elem_type ON elems(elem_type)",
            [],
        )?;

        // Enable SQLite performance optimizations
        db.conn.execute("PRAGMA journal_mode=WAL", [])?;
        db.conn.execute("PRAGMA synchronous=NORMAL", [])?;
        db.conn.execute("PRAGMA cache_size=100000", [])?;
        db.conn.execute("PRAGMA temp_store=MEMORY", [])?;
        Ok(())
    }

    pub fn insert_elems(&self, elems: &[(BgpElem, String)]) -> Result<()> {
        const BATCH_SIZE: usize = 50000;

        for batch in elems.chunks(BATCH_SIZE) {
            // Use a transaction for batch inserts
            let tx = self.db.conn.unchecked_transaction()?;

            {
                // Use prepared statement for better performance
                let mut stmt = tx.prepare_cached(
                    "INSERT INTO elems (timestamp, elem_type, collector, peer_ip, peer_asn, 
                     prefix, next_hop, as_path, origin_asns, origin, local_pref, med, 
                     communities, atomic, aggr_asn, aggr_ip) 
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"
                ).map_err(|e| anyhow!("Failed to prepare statement: {}", e))?;

                for (elem, collector) in batch {
                    let t = match elem.elem_type {
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
                        t,
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
                    .map_err(|e| anyhow!("Failed to execute statement: {}", e))?;
                }
            } // stmt is dropped here

            tx.commit()
                .map_err(|e| anyhow!("Failed to commit transaction: {}", e))?;
        }
        Ok(())
    }
}
