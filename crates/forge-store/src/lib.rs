mod blob;
pub use blob::BlobStore;

use rusqlite::Connection;
use std::{fs, path::Path};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("schema {found} is newer than supported schema {supported}")]
    NewerSchema { found: u32, supported: u32 },
}

pub struct Store {
    connection: Connection,
    pub blobs: BlobStore,
}

impl Store {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, StoreError> {
        let directory = directory.as_ref();
        fs::create_dir_all(directory)?;
        let mut connection = Connection::open(directory.join("forge.sqlite"))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection,
            blobs: BlobStore::open(directory.join("blobs"))?,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

pub fn schema_version(connection: &Connection) -> Result<u32, rusqlite::Error> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

pub fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    let version = schema_version(&transaction)?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version < 1 {
        transaction.execute_batch(include_str!("schema_v1.sql"))?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema_and_reopens_without_losing_data() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let tables: Vec<String> = store
            .connection()
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            tables,
            [
                "approvals",
                "checkpoint_files",
                "checkpoints",
                "events",
                "messages",
                "queue_items",
                "runs",
                "sessions",
                "settings",
                "steer_inputs",
                "tool_calls",
                "turns",
                "usage_records",
                "workspaces"
            ]
        );
        store
            .connection()
            .execute(
                "INSERT INTO workspaces VALUES ('w', '/project', 'Project')",
                [],
            )
            .unwrap();
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(schema_version(store.connection()).unwrap(), SCHEMA_VERSION);
        assert_eq!(
            store
                .connection()
                .query_row("SELECT name FROM workspaces", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "Project"
        );
    }

    #[test]
    fn wal_allows_reading_while_another_connection_writes() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = Store::open(directory.path()).unwrap();
        let reader = Store::open(directory.path()).unwrap();
        let mode: String = writer
            .connection()
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let transaction = writer.connection_mut().transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO workspaces VALUES ('w', '/project', 'Project')",
                [],
            )
            .unwrap();
        assert_eq!(
            reader
                .connection()
                .query_row("SELECT count(*) FROM workspaces", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        transaction.commit().unwrap();
        assert_eq!(
            reader
                .connection()
                .query_row("SELECT count(*) FROM workspaces", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn migration_never_downgrades() {
        let mut connection = Connection::open_in_memory().unwrap();
        let future_version = SCHEMA_VERSION + 1;
        connection
            .pragma_update(None, "user_version", future_version)
            .unwrap();
        assert!(
            matches!(migrate(&mut connection), Err(StoreError::NewerSchema { found, .. }) if found == future_version)
        );
        assert_eq!(schema_version(&connection).unwrap(), future_version);
    }
}
