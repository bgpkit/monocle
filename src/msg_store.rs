use bgpkit_parser::BgpElem;
use itertools::Itertools;
use crate::MonocleDatabase;

macro_rules! option_to_string{
    ($a:expr) => {
        if let Some(v) = $a {
            v.to_string()
        } else {
            String::new()
        }
    }
}

pub struct MsgStore {
    db: MonocleDatabase,
}

impl MsgStore {
    pub fn new(db_path: &Option<String>, reset: bool) -> MsgStore {
        let mut db = MonocleDatabase::new(db_path).unwrap();
        Self::initialize_msgs_db(&mut db, reset);
        MsgStore{db}
    }

    fn initialize_msgs_db(db: &mut MonocleDatabase, reset: bool) {
        db.conn.execute(r#"
        create table if not exists elems (
            timestamp INTEGER,
            elem_type TEXT,
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
        "#,[]).unwrap();

        if reset {
            db.conn.execute("delete from elems", []).unwrap();
        }
    }

    #[inline(always)]
    fn option_to_string_communities(o: &Option<Vec<bgpkit_parser::MetaCommunity>>) -> String {
        if let Some(v) = o {
            v.iter()
                .join(" ")
        } else {
            String::new()
        }
    }

    pub fn insert_elems(&self, elems: &[BgpElem]) {
        for elems in elems.chunks(100){
            let values = elems.iter().map(|elem|{
                let t = match elem.elem_type {
                    bgpkit_parser::ElemType::ANNOUNCE => "A",
                    bgpkit_parser::ElemType::WITHDRAW => "W",
                };
                let origin_string = elem.origin_asns.as_ref().map(|asns| asns.get(0).unwrap());
                format!(
                    "('{}','{}','{}','{}','{}','{}','{}','{}','{}','{}','{}','{}','{}','{}','{}')",
                    elem.timestamp as u32,
                    t,
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
                    option_to_string!(&elem.atomic),
                    option_to_string!(&elem.aggr_asn),
                    option_to_string!(&elem.aggr_ip),
                )
            }).join(", ").to_string();
            let query = format!(
                "INSERT INTO elems (\
            timestamp, elem_type, peer_ip, peer_asn, prefix, next_hop, \
            as_path, origin_asns, origin, local_pref, med, communities,\
            atomic, aggr_asn, aggr_ip)\
            VALUES {values};"
            );
            self.db.conn.execute(query.as_str(), []).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use bgpkit_parser::BgpkitParser;
    use super::*;

    #[test]
    fn test_insert() {
        let store = MsgStore::new(&Some("test.sqlite3".to_string()), false);
        let url = "https://spaces.bgpkit.org/parser/update-example.gz";
        let elems: Vec<BgpElem> = BgpkitParser::new(url).unwrap().into_elem_iter().collect();
        store.insert_elems(&elems);
    }
}
