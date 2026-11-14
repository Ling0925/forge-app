CREATE TABLE workspaces (
    id TEXT PRIMARY KEY, root TEXT NOT NULL UNIQUE, name TEXT NOT NULL
);
CREATE TABLE sessions (
    id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    title TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
    permission_mode TEXT NOT NULL, state TEXT NOT NULL, archived INTEGER NOT NULL,
    queue_state TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE queue_items (
    id TEXT PRIMARY KEY, input_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, content_json TEXT NOT NULL,
    state TEXT NOT NULL, ordinal INTEGER NOT NULL,
    UNIQUE(session_id, ordinal)
);
CREATE TABLE runs (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    queue_item_id TEXT REFERENCES queue_items(id), state TEXT NOT NULL,
    outcome_json TEXT, usage_json TEXT NOT NULL,
    started_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE TABLE turns (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE, ordinal INTEGER NOT NULL
);
CREATE TABLE messages (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE, input_id TEXT UNIQUE, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE, role TEXT NOT NULL,
    content_json TEXT NOT NULL, tool_call_id TEXT REFERENCES tool_calls(id) ON DELETE CASCADE
);
CREATE TABLE tool_calls (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE, tool TEXT NOT NULL,
    arguments_json TEXT NOT NULL, state TEXT NOT NULL, result_json TEXT,
    has_side_effects INTEGER NOT NULL
);
CREATE TABLE approvals (
    id TEXT PRIMARY KEY, tool_call_id TEXT NOT NULL REFERENCES tool_calls(id) ON DELETE CASCADE,
    requested_at_unix_ms INTEGER NOT NULL, state TEXT NOT NULL, decision TEXT,
    resolved_by TEXT, scope TEXT NOT NULL
);
CREATE TABLE checkpoints (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE
);
CREATE TABLE checkpoint_files (
    checkpoint_id TEXT NOT NULL REFERENCES checkpoints(id) ON DELETE CASCADE, path TEXT NOT NULL,
    before_hash TEXT, after_hash TEXT, PRIMARY KEY(checkpoint_id, path)
);
CREATE TABLE usage_records (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    provider TEXT NOT NULL, model TEXT NOT NULL, input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL, cached_tokens INTEGER NOT NULL,
    requests INTEGER NOT NULL, estimated_cost_usd REAL
);
CREATE TABLE steer_inputs (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    content_json TEXT NOT NULL, consumed INTEGER NOT NULL
);
CREATE TABLE events (
    seq INTEGER PRIMARY KEY, event_json TEXT NOT NULL
);
CREATE TABLE settings (
    name TEXT PRIMARY KEY, value_json TEXT NOT NULL
);

CREATE INDEX messages_run ON messages(run_id, ordinal);
CREATE INDEX runs_session ON runs(session_id);
