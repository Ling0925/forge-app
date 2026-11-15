mod blob;
mod data;
pub use blob::BlobStore;
pub use data::WriteTransaction;

use rusqlite::Connection;
use std::{fs, path::Path};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Integer(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
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
    use forge_protocol::*;

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
                "sqlite_sequence",
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

    #[test]
    fn tool_calls_round_trip_update_in_place_and_gate_tool_messages() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path()).unwrap();
        let tx = store.write().unwrap();
        tx.workspace(&Workspace {
            id: WorkspaceId("w".into()),
            root: "/w".into(),
            name: "w".into(),
        })
        .unwrap();
        tx.session(&Session {
            id: SessionId("s".into()),
            workspace_id: WorkspaceId("w".into()),
            title: "t".into(),
            model: ModelSelection {
                provider: "anthropic".into(),
                model: "m".into(),
            },
            permission_mode: PermissionMode::Ask,
            state: SessionState::Idle,
            archived: false,
            queue_state: QueueState::Active,
            created_at: 0,
            updated_at: 0,
            last_finished_run: None,
        })
        .unwrap();
        tx.run(&Run {
            id: RunId("r".into()),
            session_id: SessionId("s".into()),
            queue_item_id: None,
            state: RunState::Running,
            outcome: None,
            usage: Usage::default(),
            started_at: 0,
            finished_at: None,
        })
        .unwrap();
        tx.turn(&Turn {
            id: TurnId("t".into()),
            run_id: RunId("r".into()),
            ordinal: 0,
        })
        .unwrap();
        // A tool message cannot precede the call row it references.
        let tool_message = |id: &str, call: Option<&str>| Message {
            id: MessageId(id.into()),
            ordinal: 0,
            input_id: None,
            run_id: RunId("r".into()),
            turn_id: TurnId("t".into()),
            role: MessageRole::Tool,
            content: Content {
                text: "out".into(),
                attachments: vec![],
            },
            tool_call_id: call.map(|id| ToolCallId(id.into())),
        };
        assert!(tx.message(&tool_message("m-early", Some("c1"))).is_err());
        for (id, tool) in [("c1", "read_file"), ("c2", "write_file")] {
            tx.tool_call(&ToolCall {
                id: ToolCallId(id.into()),
                run_id: RunId("r".into()),
                tool: tool.into(),
                arguments_json: "{}".into(),
                state: ToolCallState::Proposed,
                result: None,
                has_side_effects: tool == "write_file",
            })
            .unwrap();
        }
        tx.message(&tool_message("m1", Some("c1"))).unwrap();
        tx.commit().unwrap();

        // Finishing a call updates only state and result.
        let mut call = store.tool_call(&ToolCallId("c1".into())).unwrap().unwrap();
        assert_eq!(call.state, ToolCallState::Proposed);
        call.state = ToolCallState::Succeeded;
        call.result = Some(Content {
            text: "body".into(),
            attachments: vec![],
        });
        let tx = store.write().unwrap();
        tx.tool_call(&call).unwrap();
        tx.commit().unwrap();
        let stored = store.tool_call(&ToolCallId("c1".into())).unwrap().unwrap();
        assert_eq!(stored.state, ToolCallState::Succeeded);
        assert_eq!(stored.result.unwrap().text, "body");
        assert_eq!(stored.tool, "read_file");
        assert!(!stored.has_side_effects);

        let calls = store.tool_calls(&RunId("r".into())).unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|call| call.id.0.as_str())
                .collect::<Vec<_>>(),
            ["c1", "c2"]
        );
        assert!(calls[1].has_side_effects);
    }
}
