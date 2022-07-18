use rusqlite::Connection;
use anyhow::Result;

pub struct MonocleDatabase {
    pub conn: Connection,
}

impl MonocleDatabase {
    pub fn new(path: &Option<String>) -> Result<MonocleDatabase> {
        let conn = match path {
            Some(p) => {
                Connection::open(p.as_str())?
            }
            None => {
                Connection::open_in_memory()?
            }
        };
        Ok(MonocleDatabase{conn})
    }
}