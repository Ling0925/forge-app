use forge_protocol::{
    Command, CommandResult, EventEnvelope, ProviderKeyErrorKind, RestoreFailure,
    RestorePreviewFile, RestorePreviewStatus, UncertainAction,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn round_trip<T: Serialize + DeserializeOwned>(value: Value) {
    let decoded: T = serde_json::from_value(value.clone()).unwrap();
    let encoded = serde_json::to_string(&decoded).unwrap();
    let decoded: T = serde_json::from_str(&encoded).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}

#[test]
fn every_command_round_trips_with_ui_idempotency_ids() {
    let content = json!({"text": "hello", "attachments": [{"name":"file", "media_type":"text/plain", "blob":"hash"}]});
    let model = json!({"provider":"provider", "model":"fixture"});
    for command in [
        json!({"type":"add_workspace", "path":"/project"}),
        json!({"type":"remove_workspace", "workspace":"w"}),
        json!({"type":"test_provider_key", "provider":"provider"}),
        json!({"type":"create_session", "workspace":"w", "title":"title", "model":model, "permission_mode":"ask"}),
        json!({"type":"rename_session", "session":"s", "title":"renamed"}),
        json!({"type":"archive_session", "session":"s", "archived":true}),
        json!({"type":"delete_session", "session":"s"}),
        json!({"type":"load_history", "session":"s", "before":42, "limit":20}),
        json!({"type":"load_history", "session":"s", "before":null, "limit":20}),
        json!({"type":"send", "session":"s", "input_id":"ui-input", "content":content}),
        json!({"type":"steer", "run":"r", "steer_id":"ui-steer", "content":content}),
        json!({"type":"edit_queue_item", "item":"q", "content":content}),
        json!({"type":"remove_queue_item", "item":"q"}),
        json!({"type":"reorder_queue_item", "item":"q", "before":"other"}),
        json!({"type":"reorder_queue_item", "item":"q", "before":null}),
        json!({"type":"resolve_uncertain", "item":"q", "action":"requeue"}),
        json!({"type":"resolve_uncertain", "item":"q", "action":"discard"}),
        json!({"type":"pause_queue", "session":"s"}),
        json!({"type":"resume_queue", "session":"s"}),
        json!({"type":"cancel_run", "run":"r"}),
        json!({"type":"retry_run", "run":"r"}),
        json!({"type":"retry_now", "run":"r"}),
        json!({"type":"decide_approval", "approval":"a", "decision":"allow", "scope":"session"}),
        json!({"type":"set_permission_mode", "session":"s", "mode":"read_only"}),
        json!({"type":"preview_restore", "checkpoint":"c"}),
        json!({"type":"restore_checkpoint", "checkpoint":"c", "overwrite":["/project/file"]}),
        json!({"type":"restore_checkpoint", "checkpoint":"c", "overwrite":[]}),
        json!({"type":"update_settings", "settings":{"providers":[{"id":"provider", "endpoint":null,"keychain_reference":"key-ref", "max_output_tokens":null}], "model":model,"permission_mode":"unrestricted", "ui_prefs":{}}}),
        json!({"type":"get_snapshot"}),
        json!({"type":"set_provider_key", "provider":"provider", "key":"test-only"}),
        json!({"type":"set_provider_key", "provider":"provider", "key":null}),
        json!({"type":"shutdown"}),
    ] {
        round_trip::<Command>(command);
    }
}

#[test]
fn every_event_round_trips_inside_a_sequence_envelope() {
    let session = json!({"id":"s", "workspace_id":"w", "title":"title", "model":{"provider":"provider", "model":"fixture"}, "permission_mode":"ask", "state":"idle", "archived":false, "queue_state":"active", "created_at":1000, "updated_at":2000});
    let content = json!({"text":"input", "attachments":[]});
    let item = json!({"id":"q", "input_id":"ui-input", "session_id":"s", "content":content, "state":"pending", "ordinal":0});
    let usage = json!({"input_tokens":3, "output_tokens":2, "cached_tokens":1, "requests":1, "estimated_cost_usd":0.1});
    for (seq, event) in [
        json!({"type":"workspace_added", "workspace":{"id":"w", "root":"/project", "name":"Project"}}),
        json!({"type":"workspace_removed", "workspace":"w"}),
        json!({"type":"provider_key_tested", "provider":"provider", "ok":true, "error_kind":null}),
        json!({"type":"provider_key_tested", "provider":"provider", "ok":false, "error_kind":"auth"}),
        json!({"type":"session_created", "session":session}),
        json!({"type":"session_updated", "session":session}),
        json!({"type":"session_archived", "session":"s", "archived":true}),
        json!({"type":"session_deleted", "session":"s"}),
        json!({"type":"queue_item_added", "item":item}),
        json!({"type":"queue_item_updated", "item":item}),
        json!({"type":"queue_item_removed", "session":"s", "item":"q"}),
        json!({"type":"queue_paused", "session":"s", "reason":"cancelled"}),
        json!({"type":"queue_resumed", "session":"s"}),
        json!({"type":"run_started", "run":{"id":"r", "session_id":"s", "queue_item_id":"q", "state":"running", "outcome":null, "started_at":1000, "finished_at":null, "usage":usage}, "input_id":"ui-input"}),
        json!({"type":"run_state_changed", "run":"r", "state":"cancelling"}),
        json!({"type":"run_finished", "run":"r", "outcome":{"type":"cancelled", "source":"app_quit"}}),
        json!({"type":"steer_applied", "run":"r", "steer_id":"ui-steer"}),
        json!({"type":"text_delta", "run":"r", "text":"hello"}),
        json!({"type":"reasoning_delta", "run":"r", "text":"reason"}),
        json!({"type":"provider_attempt", "run":"r", "n":1}),
        json!({"type":"provider_retrying", "run":"r", "n":1, "delay_ms":500, "reason":"503"}),
        json!({"type":"generation_reset", "run":"r"}),
        json!({"type":"tool_call_proposed", "call":{"id":"t", "run_id":"r", "tool":"command", "arguments_json":"{}", "state":"proposed", "result":null, "has_side_effects":true}}),
        json!({"type":"tool_call_state_changed", "call":"t", "state":"succeeded", "result":content}),
        json!({"type":"tool_output_chunk", "call":"t", "stream":"stdout", "bytes":[0, 255]}),
        json!({"type":"approval_requested", "approval":{"id":"a", "tool_call_id":"t", "requested_at_unix_ms":0, "state":"pending", "decision":null, "resolved_by":null, "scope":"once"}}),
        json!({"type":"approval_resolved", "approval":"a", "decision":"deny", "scope":"once", "by":"cancel"}),
        json!({"type":"context_usage", "run":"r", "used_tokens":1, "context_remaining_estimate":2}),
        json!({"type":"context_compacted", "run":"r", "before_tokens":2, "after_tokens":1}),
        json!({"type":"checkpoint_created", "checkpoint":{"id":"c", "run_id":"r", "files":[{"path":"file", "before":null, "after":"hash"}]}}),
        json!({"type":"restore_previewed", "checkpoint":"c", "files":[{"path":"clean", "status":"clean"}, {"path":"changed", "status":"modified_externally"}, {"path":"missing", "status":"deleted_externally"}]}),
        json!({"type":"checkpoint_restored", "checkpoint":"c", "restored":["clean"], "skipped":["changed"], "failed":[{"path":"missing", "reason":"permission denied"}]}),
        json!({"type":"usage_recorded", "record":{"id":"u", "run_id":"r", "provider":"provider", "model":"fixture", "usage":usage}}),
        json!({"type":"notice", "level":"info", "code":"fixture", "message":"message"}),
        json!({"type":"shutdown_complete", "stopped":["r"], "unfinished":["other"]}),
        json!({"type":"shutdown_complete", "stopped":[], "unfinished":[]}),
    ].into_iter().enumerate() { round_trip::<EventEnvelope>(json!({"seq":seq + 1,"event":event})); }
}

#[test]
fn accepted_and_reject_codes_round_trip() {
    round_trip::<CommandResult>(json!({"type":"accepted", "id":"id"}));
    for code in [
        "run_finished",
        "already_finished",
        "approval_closed",
        "conflict",
        "not_found",
        "invalid_state",
        "storage",
        "unavailable",
        "not_implemented",
    ] {
        round_trip::<CommandResult>(json!({"type":"rejected", "code":code, "message":"reason"}));
    }
}

#[test]
fn restore_preview_statuses_and_files_round_trip() {
    for status in ["clean", "modified_externally", "deleted_externally"] {
        round_trip::<RestorePreviewStatus>(json!(status));
        round_trip::<RestorePreviewFile>(json!({"path":"/project/file", "status":status}));
    }
}

#[test]
fn restore_failures_round_trip() {
    round_trip::<RestoreFailure>(json!({"path":"/project/file", "reason":"permission denied"}));
}

#[test]
fn provider_key_error_kinds_round_trip() {
    for kind in [
        "network",
        "rate_limited",
        "auth",
        "quota",
        "server",
        "configuration",
        "provider",
    ] {
        round_trip::<ProviderKeyErrorKind>(json!(kind));
        round_trip::<EventEnvelope>(json!({
            "seq":1,
            "event":{"type":"provider_key_tested", "provider":"provider", "ok":false, "error_kind":kind}
        }));
    }
}

#[test]
fn uncertain_actions_round_trip() {
    for action in ["requeue", "discard"] {
        round_trip::<UncertainAction>(json!(action));
    }
}

#[test]
fn p1_snapshots_history_settings_and_structured_outcomes_round_trip() {
    use forge_protocol::{RunOutcome, Settings};
    let settings = json!({"providers":[{"id":"openai","endpoint":null,"keychain_reference":null,"max_output_tokens":null}],"model":{"provider":"openai","model":"fixture"},"permission_mode":"ask","ui_prefs":{"onboarding_complete":"true"}});
    round_trip::<Settings>(settings.clone());
    for event in [
        json!({"type":"settings_updated","settings":settings}),
        json!({"type":"provider_key_configured","provider":"openai","configured":true}),
        json!({"type":"snapshot","sessions":[],"workspaces":[],"settings":settings,"providers":[{"provider":"openai","key_configured":false}],"active_runs":[{"run":{"id":"r","session_id":"s","queue_item_id":null,"state":"running","outcome":null,"started_at":1000,"finished_at":null,"usage":{"input_tokens":0,"output_tokens":0,"cached_tokens":0,"requests":0,"estimated_cost_usd":null}},"input_id":"i","text":"partial","reasoning":"why"}],"last_seq":10}),
        json!({"type":"history_loaded","session":"s","messages":[{"id":"m","ordinal":2,"input_id":"i","run_id":"r","turn_id":"t","role":"user","content":{"text":"hello","attachments":[]},"tool_call_id":null}],"next_before":2}),
    ] {
        round_trip::<EventEnvelope>(json!({"seq":10,"event":event}));
    }
    round_trip::<RunOutcome>(json!({"type":"succeeded"}));
    for source in ["user", "app_quit"] {
        round_trip::<RunOutcome>(json!({"type":"cancelled","source":source}));
    }
    for kind in [
        "network",
        "rate_limited",
        "auth",
        "quota",
        "server",
        "context_too_long",
        "interrupted",
    ] {
        let outcome = json!({"type":"failed","kind":kind,"message":"failure","http_status":429,"retry_after_ms":120000,"retryable":true});
        round_trip::<RunOutcome>(outcome.clone());
        round_trip::<EventEnvelope>(
            json!({"seq":10,"event":{"type":"run_finished","run":"r","outcome":outcome}}),
        );
    }
}

#[test]
fn unknown_run_errors_round_trip_with_provider_or_storage_source() {
    use forge_protocol::{RunErrorKind, RunOutcome, UnknownSource};
    for (source, name) in [
        (UnknownSource::Provider, "provider"),
        (UnknownSource::Storage, "storage"),
    ] {
        let kind = RunErrorKind::Unknown { source };
        let value = json!({"unknown":{"source":name}});
        assert_eq!(serde_json::to_value(&kind).unwrap(), value);
        assert_eq!(
            serde_json::from_value::<RunErrorKind>(value.clone()).unwrap(),
            kind
        );
        let outcome = json!({"type":"failed", "kind":value, "message":"failure", "http_status":null, "retry_after_ms":null, "retryable":false});
        round_trip::<RunOutcome>(outcome.clone());
        round_trip::<EventEnvelope>(
            json!({"seq":1, "event":{"type":"run_finished", "run":"r", "outcome":outcome}}),
        );
    }
}
