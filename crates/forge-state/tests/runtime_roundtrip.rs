//! forge-state against the real P0 `forge_core::Runtime` channel (no mocks for
//! the transport): commands go through `RuntimeHandle::execute`, events come
//! from `Runtime::publish` via a subscription.
mod common;
use common::*;
use forge_core::{Runtime, RuntimeServices};
use forge_protocol::*;
use forge_state::*;
use forge_testkit::provider::ScriptedProvider;
use forge_tools::clock::FakeClock;
use std::sync::Arc;

fn runtime() -> (Runtime, forge_core::RuntimeHandle, tempfile::TempDir) {
    let clock = Arc::new(FakeClock::default());
    let directory = tempfile::tempdir().unwrap();
    let store = forge_store::Store::open(directory.path()).unwrap();
    let (runtime, handle) = Runtime::new(
        RuntimeServices {
            clock: clock.clone(),
            store,
            provider: Arc::new(ScriptedProvider::new(clock, [])),
        },
        0,
    );
    (runtime, handle, directory)
}

#[tokio::test]
async fn p0_runtime_rejects_send_and_composer_restores_text() {
    let (mut runtime, handle, _dir) = runtime();
    let mut events = handle.subscribe();
    let mut app = app();

    for event in [
        Event::WorkspaceAdded {
            workspace: Workspace {
                id: "ws".into(),
                root: "/tmp/ws".into(),
                name: "ws".into(),
            },
        },
        Event::SessionCreated {
            session: session("s1"),
        },
    ] {
        runtime.publish(event);
    }
    while let Ok(envelope) = events.try_recv() {
        app.apply(envelope);
    }
    assert_eq!(app.last_seq, 2);
    app.select_session(&"s1".into());
    app.set_draft("hello core");
    app.composer_key(ComposerKey::Enter);

    for out in app.take_outbox() {
        let client = handle.clone();
        let command = out.command.clone();
        let task = tokio::spawn(async move { client.execute(command).await });
        assert!(runtime.dispatch_next().await.unwrap());
        let result = task.await.unwrap().unwrap();
        app.apply_result(out.token, result);
    }
    let composer = &app.selected().unwrap().composer;
    assert_eq!(composer.draft, "hello core");
    assert_eq!(
        composer.error.as_deref(),
        Some("没有发送：此功能还没有实现")
    );
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
}

#[tokio::test]
async fn closed_runtime_surfaces_transport_error() {
    let (runtime, handle, _dir) = runtime();
    drop(runtime);
    let mut app = app_with_session();
    app.set_draft("x");
    app.composer_key(ComposerKey::Enter);
    let out = single_command(&mut app);
    assert!(handle.execute(out.command.clone()).await.is_err());
    app.transport_failed(out.token);
    assert_eq!(
        app.selected().unwrap().composer.error.as_deref(),
        Some("没有发送：无法连接后台")
    );
}

#[tokio::test]
async fn published_run_events_reduce_into_timeline() {
    let (mut runtime, handle, _dir) = runtime();
    let mut events = handle.subscribe();
    let mut app = app_with_session();
    // Align with the runtime's seq space.
    app.last_seq = 0;
    for event in [
        Event::RunStarted {
            run: run("r1", "s1", None),
        },
        Event::TextDelta {
            run: "r1".into(),
            text: "ok".into(),
        },
        Event::RunFinished {
            run: "r1".into(),
            outcome: RunOutcome::Succeeded,
            reason: None,
        },
    ] {
        runtime.publish(event);
    }
    while let Ok(envelope) = events.try_recv() {
        app.apply(envelope);
    }
    assert_eq!(assistant_text(&app, "s1", "r1").as_deref(), Some("ok"));
    assert_eq!(app.resync, ResyncState::Idle);
}
