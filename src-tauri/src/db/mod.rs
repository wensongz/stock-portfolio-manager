pub(crate) mod migrations;
mod schema;
mod snapshot_cache_schema;

use rusqlite::{Connection, Result};
use std::sync::Mutex;

pub(crate) use schema::SYSTEM_CATEGORIES;

pub struct Database {
    pub conn: Mutex<Connection>,
    pub path: String,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        migrations::run_migrations(&mut connection)?;
        Ok(Self {
            conn: Mutex::new(connection),
            path: path.to_string(),
        })
    }
}

#[cfg(test)]
mod tests;
