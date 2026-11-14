use crate::{Store, StoreError};
use forge_protocol::*;
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};

fn json<T: DeserializeOwned>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    let value: String = row.get(column)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            row.as_ref().column_index(column).unwrap(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}
fn sql(value: u64) -> Result<i64, StoreError> {
    Ok(value.try_into()?)
}
fn encode(value: &impl Serialize) -> Result<String, StoreError> {
    Ok(serde_json::to_string(value)?)
}
fn session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: SessionId(row.get("id")?),
        workspace_id: WorkspaceId(row.get("workspace_id")?),
        title: row.get("title")?,
        model: ModelSelection {
            provider: ProviderId(row.get("provider")?),
            model: row.get("model")?,
        },
        permission_mode: json(row, "permission_mode")?,
        state: json(row, "state")?,
        archived: row.get("archived")?,
        queue_state: json(row, "queue_state")?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        updated_at: row.get::<_, i64>("updated_at")? as u64,
        last_finished_run: None,
    })
}
fn workspace(row: &Row<'_>) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: WorkspaceId(row.get("id")?),
        root: row.get::<_, String>("root")?.into(),
        name: row.get("name")?,
    })
}
fn run(row: &Row<'_>) -> rusqlite::Result<Run> {
    Ok(Run {
        id: RunId(row.get("id")?),
        session_id: SessionId(row.get("session_id")?),
        queue_item_id: row
            .get::<_, Option<String>>("queue_item_id")?
            .map(QueueItemId),
        state: json(row, "state")?,
        outcome: row
            .get::<_, Option<String>>("outcome_json")?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        usage: json(row, "usage_json")?,
        started_at: row.get::<_, i64>("started_at")? as u64,
        finished_at: row.get::<_, Option<i64>>("finished_at")?.map(|n| n as u64),
    })
}
fn message(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: MessageId(row.get("id")?),
        ordinal: row.get::<_, i64>("ordinal")? as u64,
        input_id: row.get::<_, Option<String>>("input_id")?.map(InputId),
        run_id: RunId(row.get("run_id")?),
        turn_id: TurnId(row.get("turn_id")?),
        role: json(row, "role")?,
        content: json(row, "content_json")?,
        tool_call_id: row
            .get::<_, Option<String>>("tool_call_id")?
            .map(ToolCallId),
    })
}
impl Store {
    pub fn new_id(&self) -> Result<String, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?)
    }
    pub fn sessions(&self) -> Result<Vec<Session>, StoreError> {
        let mut sessions: Vec<Session> = self
            .connection
            .prepare("SELECT * FROM sessions ORDER BY id")?
            .query_map([], session)?
            .collect::<Result<_, _>>()?;
        for session in &mut sessions {
            session.last_finished_run = self.last_finished_run(&session.id)?;
        }
        Ok(sessions)
    }
    pub fn session(&self, id: &SessionId) -> Result<Option<Session>, StoreError> {
        let mut session = self
            .connection
            .query_row("SELECT * FROM sessions WHERE id=?", [&id.0], session)
            .optional()?;
        if let Some(session) = &mut session {
            session.last_finished_run = self.last_finished_run(id)?;
        }
        Ok(session)
    }
    fn last_finished_run(&self, id: &SessionId) -> Result<Option<FinishedRun>, StoreError> {
        Ok(self.connection.query_row(
            "SELECT id, outcome_json, finished_at,
                (SELECT input_id FROM messages WHERE run_id=runs.id AND input_id IS NOT NULL ORDER BY ordinal LIMIT 1) AS input_id
             FROM runs WHERE session_id=? AND finished_at IS NOT NULL
             ORDER BY finished_at DESC, rowid DESC LIMIT 1",
            [&id.0],
            |row| Ok(FinishedRun {
                run: RunId(row.get("id")?),
                input_id: row.get::<_, Option<String>>("input_id")?.map(InputId),
                outcome: json(row, "outcome_json")?,
                finished_at: row.get::<_, i64>("finished_at")? as u64,
            }),
        ).optional()?)
    }
    pub fn running_runs(&self) -> Result<Vec<Run>, StoreError> {
        Ok(self
            .connection
            .prepare("SELECT * FROM runs WHERE state=? ORDER BY rowid")?
            .query_map([encode(&RunState::Running)?], run)?
            .collect::<Result<_, _>>()?)
    }
    pub fn workspaces(&self) -> Result<Vec<Workspace>, StoreError> {
        Ok(self
            .connection
            .prepare("SELECT * FROM workspaces ORDER BY id")?
            .query_map([], workspace)?
            .collect::<Result<_, _>>()?)
    }
    pub fn run(&self, id: &RunId) -> Result<Option<Run>, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT * FROM runs WHERE id=?", [&id.0], run)
            .optional()?)
    }
    pub fn input(&self, id: &InputId) -> Result<Option<Message>, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT * FROM messages WHERE input_id=?", [&id.0], message)
            .optional()?)
    }
    pub fn run_input_id(&self, id: &RunId) -> Result<Option<InputId>, StoreError> {
        Ok(self.connection.query_row(
            "SELECT input_id FROM messages WHERE run_id=? AND input_id IS NOT NULL ORDER BY ordinal LIMIT 1",
            [&id.0],
            |row| Ok(InputId(row.get(0)?)),
        ).optional()?)
    }
    pub fn settings(&self) -> Result<Option<Settings>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value_json FROM settings WHERE name='settings'",
                [],
                |row| json(row, "value_json"),
            )
            .optional()?)
    }
    pub fn last_seq(&self) -> Result<u64, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT coalesce(max(seq), 0) FROM events", [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
    }
    pub fn history(
        &self,
        id: &SessionId,
        before: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<Message>, Option<u64>), StoreError> {
        let mut messages: Vec<Message> = self.connection.prepare(
            "SELECT messages.* FROM messages JOIN runs ON runs.id=messages.run_id WHERE runs.session_id=?1 AND (?2 IS NULL OR messages.ordinal < ?2) ORDER BY messages.ordinal DESC LIMIT ?3")?
            .query_map(params![id.0,before.and_then(|n| i64::try_from(n).ok()),limit], message)?.collect::<Result<_,_>>()?;
        messages.reverse();
        let next_before = if let Some(first) = messages.first() {
            let older: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM messages JOIN runs ON runs.id=messages.run_id WHERE runs.session_id=? AND messages.ordinal < ?)",params![id.0,sql(first.ordinal)?],|r| r.get(0))?;
            older.then_some(first.ordinal)
        } else {
            None
        };
        Ok((messages, next_before))
    }
    pub fn messages(&self, id: &SessionId) -> Result<Vec<Message>, StoreError> {
        Ok(self.connection.prepare("SELECT messages.* FROM messages JOIN runs ON runs.id=messages.run_id WHERE runs.session_id=? ORDER BY messages.ordinal")?
            .query_map([&id.0],message)?.collect::<Result<_,_>>()?)
    }
    pub fn write(&mut self) -> Result<WriteTransaction<'_>, StoreError> {
        Ok(WriteTransaction {
            transaction: self.connection.transaction()?,
        })
    }
}

pub struct WriteTransaction<'a> {
    transaction: Transaction<'a>,
}
impl WriteTransaction<'_> {
    pub fn workspace(&self, workspace: &Workspace) -> Result<(), StoreError> {
        self.transaction.execute(
            "INSERT INTO workspaces (id,root,name) VALUES (?,?,?)",
            params![
                workspace.id.0,
                workspace.root.to_string_lossy(),
                workspace.name
            ],
        )?;
        Ok(())
    }
    pub fn remove_workspace(&self, id: &WorkspaceId) -> Result<(), StoreError> {
        self.transaction
            .execute("DELETE FROM workspaces WHERE id=?", [&id.0])?;
        Ok(())
    }
    pub fn session(&self, s: &Session) -> Result<(), StoreError> {
        self.transaction.execute("INSERT INTO sessions (id,workspace_id,title,provider,model,permission_mode,state,archived,queue_state,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET title=excluded.title, state=excluded.state, archived=excluded.archived, updated_at=excluded.updated_at",
            params![s.id.0,s.workspace_id.0,s.title,s.model.provider.0,s.model.model,encode(&s.permission_mode)?,encode(&s.state)?,s.archived,encode(&s.queue_state)?,sql(s.created_at)?,sql(s.updated_at)?])?;
        Ok(())
    }
    pub fn delete_session(&self, id: &SessionId) -> Result<(), StoreError> {
        self.transaction
            .execute("DELETE FROM sessions WHERE id=?", [&id.0])?;
        Ok(())
    }
    pub fn settings(&self, settings: &Settings) -> Result<(), StoreError> {
        self.transaction.execute("INSERT INTO settings VALUES ('settings',?) ON CONFLICT(name) DO UPDATE SET value_json=excluded.value_json",[encode(settings)?])?;
        Ok(())
    }
    pub fn run(&self, r: &Run) -> Result<(), StoreError> {
        self.transaction.execute("INSERT INTO runs (id,session_id,queue_item_id,state,outcome_json,usage_json,started_at,finished_at) VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state, outcome_json=excluded.outcome_json, usage_json=excluded.usage_json, finished_at=excluded.finished_at",
            params![r.id.0,r.session_id.0,r.queue_item_id.as_ref().map(|i| &i.0),encode(&r.state)?,r.outcome.as_ref().map(encode).transpose()?,encode(&r.usage)?,sql(r.started_at)?,r.finished_at.map(sql).transpose()?])?;
        Ok(())
    }
    pub fn turn(&self, t: &Turn) -> Result<(), StoreError> {
        self.transaction.execute(
            "INSERT INTO turns VALUES (?,?,?)",
            params![t.id.0, t.run_id.0, sql(t.ordinal)?],
        )?;
        Ok(())
    }
    pub fn message(&self, m: &Message) -> Result<(), StoreError> {
        self.transaction.execute("INSERT INTO messages (id,input_id,run_id,turn_id,role,content_json,tool_call_id) VALUES (?,?,?,?,?,?,?)",
            params![m.id.0,m.input_id.as_ref().map(|i| &i.0),m.run_id.0,m.turn_id.0,encode(&m.role)?,encode(&m.content)?,m.tool_call_id.as_ref().map(|i| &i.0)])?;
        Ok(())
    }
    pub fn usage(&self, r: &UsageRecord) -> Result<(), StoreError> {
        self.transaction.execute(
            "INSERT INTO usage_records VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                r.id.0,
                r.run_id.0,
                r.provider.0,
                r.model,
                sql(r.usage.input_tokens)?,
                sql(r.usage.output_tokens)?,
                sql(r.usage.cached_tokens)?,
                sql(r.usage.requests)?,
                r.usage.estimated_cost_usd
            ],
        )?;
        Ok(())
    }
    pub fn events(&self, events: &[EventEnvelope]) -> Result<(), StoreError> {
        for envelope in events {
            self.transaction.execute(
                "INSERT INTO events VALUES (?,?)",
                params![sql(envelope.seq)?, encode(&envelope.event)?],
            )?;
        }
        Ok(())
    }
    pub fn commit(self) -> Result<(), StoreError> {
        Ok(self.transaction.commit()?)
    }
}
