use anyhow::Result;
use bgpkit_parser::models::{ElemType, MetaCommunity};
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
macro_rules! option_to_string {
    ($a:expr) => {
        if let Some(v) = $a {
            format!("'{}'", v)
        } else {
            "NULL".to_string()
        }
    };
}

pub struct MsgStore {
    db: MonocleDatabase,
}

impl MsgStore {
    pub fn new(db_path: &Option<String>, reset: bool) -> MsgStore {
        let mut db = MonocleDatabase::new(db_path).unwrap();
        Self::initialize_msgs_db(&mut db, reset);
        MsgStore { db }
    }

    fn initialize_msgs_db(db: &mut MonocleDatabase, reset: bool) {
        if reset {
            db.conn.execute("drop table if exists elems", []).unwrap();
        }
        db.conn
            .execute(
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
            )
            .unwrap();
    }

    #[inline(always)]
    fn option_to_string_communities(o: &Option<Vec<MetaCommunity>>) -> String {
        if let Some(v) = o {
            format!("'{}'", v.iter().join(" "))
        } else {
            "NULL".to_string()
        }
    }

    pub fn insert_elems(&self, elems: &[(BgpElem, String)]) {
        for elems in elems.chunks(10000) {
            let values = elems
                .iter()
                .map(|(elem, collector)| {
                    let t = match elem.elem_type {
                        // bgpkit_parser::ElemType::ANNOUNCE => "A",
                        // bgpkit_parser::ElemType::WITHDRAW => "W",
                        ElemType::ANNOUNCE => "A",
                        ElemType::WITHDRAW => "W",
                    };
                    let origin_string = elem.origin_asns.as_ref().map(|asns| asns.first().unwrap());
                    format!(
                        "('{}','{}','{}', '{}','{}','{}', {},{},{},{},{},{},{},'{}',{},{})",
                        elem.timestamp as u32,
                        t,
                        collector,
                        elem.peer_ip,
                        elem.peer_asn,
                        elem.prefix,
                        option_to_string!(&elem.next_hop),
                        option_to_string!(&elem.as_path),
                        option_to_string!(origin_string),
                        option_to_string!(&elem.origin),
                        option_to_string!(&elem.local_pref),
                        option_to_string!(&elem.med),
                        Self::option_to_string_communities(&elem.communities),
                        match &elem.atomic {
                            true => "AG",
                            false => "NAG",
                        },
                        option_to_string!(&elem.aggr_asn),
                        option_to_string!(&elem.aggr_ip),
                    )
                })
                .join(", ")
                .to_string();
            let query = format!(
                "INSERT INTO elems (\
            timestamp, elem_type, collector, peer_ip, peer_asn, prefix, next_hop, \
            as_path, origin_asns, origin, local_pref, med, communities,\
            atomic, aggr_asn, aggr_ip)\
            VALUES {values};"
            );
            self.db.conn.execute(query.as_str(), []).unwrap();
        }
    }
}
