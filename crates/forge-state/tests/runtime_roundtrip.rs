//! forge-state against the real P1 `forge_core::Runtime` (no transport mocks):
//! commands go through `RuntimeHandle::execute`, events come from the runtime
//! subscription, the provider is a scripted fixture behind
//! `FixedProviderFactory`, keys live in `MemorySecretStore`.
mod common;
use common::*;
use forge_core::secrets::MemorySecretStore;
use forge_core::{Runtime, RuntimeHandle, RuntimeServices};
use forge_protocol::*;
use forge_provider::{ErrorKind as ProviderErrorKind, ProviderError, RetryAfter, StreamEvent};
use forge_state::ids::SequentialIds;
use forge_state::*;
use forge_testkit::provider::{FixedProviderFactory, Script, ScriptedProvider, Step};
use forge_tools::clock::FakeClock;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

const SENTINEL_KEY: &str = "sk-forge-sentinel-0123456789abcdef";

struct Core {
    runtime: Runtime,
    handle: RuntimeHandle,
    events: UnboundedReceiver<EventEnvelope>,
    _dir: tempfile::TempDir,
}

fn core(scripts: Vec<Script>) -> Core {
    let clock = Arc::new(FakeClock::default());
    let dir = tempfile::tempdir().unwrap();
    let store = forge_store::Store::open(dir.path()).unwrap();
    let provider = Arc::new(ScriptedProvider::new(clock.clone(), scripts));
    let (runtime, handle) = Runtime::new(
        RuntimeServices {
            clock,
            store,
            providers: Arc::new(FixedProviderFactory(provider)),
            secrets: Arc::new(MemorySecretStore::default()),
            initial_settings: settings(true),
        },
        0,
    )
    .unwrap();
    let events = handle.subscribe();
    Core {
        runtime,
        handle,
        events,
        _dir: dir,
    }
}

impl Core {
    /// Executes the app's outbox, lets provider tasks finish, then applies
    /// every published event; repeats until nothing is left to send.
    async fn pump(&mut self, app: &mut AppState) {
        for _ in 0..10 {
            let outbox = app.take_outbox();
            for out in &outbox {
                let client = self.handle.clone();
                let command = out.command.clone();
                let task = tokio::spawn(async move { client.execute(command).await });
                assert!(self.runtime.dispatch_next().await.unwrap());
                app.apply_result(out.token, task.await.unwrap().unwrap());
            }
            while let Ok(Ok(true)) =
                tokio::time::timeout(Duration::from_millis(50), self.runtime.dispatch_next()).await
            {
            }
            let mut any = false;
            while let Ok(envelope) = self.events.try_recv() {
                app.apply(envelope);
                any = true;
            }
            if outbox.is_empty() && !any {
                return;
            }
        }
    }
}

fn fresh_app() -> AppState {
    AppState::new(Box::new(SequentialIds::default()))
}

/// Boot → workspace → session → key, all through Core.
async fn ready(core: &mut Core, workspace: &std::path::Path) -> AppState {
    let mut app = fresh_app();
    app.request_snapshot();
    core.pump(&mut app).await;
    assert!(app.synced);
    assert!(!app.has_api_key(), "fresh store has no key");
    app.add_workspace(workspace.to_path_buf());
    core.pump(&mut app).await;
    app.create_session().unwrap();
    core.pump(&mut app).await;
    assert!(app.selected().is_some());
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::NoKey)
    );
    app.set_provider_key("openai".into(), Some(SENTINEL_KEY.into()));
    core.pump(&mut app).await;
    assert!(app.has_api_key());
    assert_eq!(
        app.settings.key_status[&ProviderId::from("openai")],
        KeyStatus::SavedUnverified
    );
    app
}

#[tokio::test]
async fn boot_send_and_reload_history_through_core() {
    let mut core = core(vec![Script::Stream(vec![
        Step::Event(StreamEvent::TextDelta("你好".into())),
        Step::Event(StreamEvent::TextDelta("，世界".into())),
        Step::Event(StreamEvent::Finished),
    ])]);
    let ws = tempfile::tempdir().unwrap();
    let mut app = ready(&mut core, ws.path()).await;
    app.set_draft("hello core");
    assert!(matches!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Sent(_)
    ));
    core.pump(&mut app).await;
    let session = app.selected_session.clone().unwrap();
    assert_eq!(
        assistant_text(
            &app,
            &session.0,
            &app.sessions[&session].runs.keys().next().unwrap().0
        )
        .as_deref(),
        Some("你好，世界")
    );
    assert!(matches!(
        app.sessions[&session].last_outcome,
        Some(RunOutcome::Succeeded)
    ));
    // No key anywhere in the reduced state.
    assert!(!format!("{:?}", app.sessions).contains(SENTINEL_KEY));

    // A second window: snapshot + first history page rebuild the timeline.
    let mut second = fresh_app();
    second.request_snapshot();
    core.events = core.handle.subscribe();
    core.pump(&mut second).await;
    assert_eq!(second.selected_session.as_ref(), Some(&session));
    assert!(second.has_api_key());
    let vm = &second.sessions[&session];
    assert_eq!(vm.history, HistoryState::Complete);
    let texts: Vec<_> = vm
        .timeline
        .iter()
        .filter_map(|i| match i {
            TimelineItem::User(m) => Some(m.text.clone()),
            TimelineItem::Assistant(m) => Some(m.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["hello core", "你好，世界"]);
}

#[tokio::test]
async fn long_retry_after_from_core_becomes_wait_card() {
    let mut core = core(vec![Script::Error(ProviderError {
        code: Some("rate_limit_exceeded".into()),
        kind: ProviderErrorKind::Http {
            status: 429,
            retry_after: Some(RetryAfter::Delay(Duration::from_secs(120))),
        },
        message: "slow down\nAuthorization: Bearer sk-leak".into(),
    })]);
    let ws = tempfile::tempdir().unwrap();
    let mut app = ready(&mut core, ws.path()).await;
    app.set_now_ms(1_000);
    app.set_draft("hi");
    app.composer_key(ComposerKey::Enter);
    core.pump(&mut app).await;
    let card = app
        .timeline_rows()
        .into_iter()
        .find_map(|r| match r {
            TimelineRowVm::Error(card) => Some(card),
            _ => None,
        })
        .expect("error card");
    assert_eq!(card.kind, ErrorCardKind::RateLimitedWait);
    assert!(matches!(card.retry, RetryButtonState::Waiting { .. }));
    assert!(card.details.iter().all(|d| !d.contains("sk-leak")));
}

#[tokio::test]
async fn busy_session_send_is_rejected_with_invalid_state() {
    // The first run never finishes (no script step ends it before the test).
    let mut core = core(vec![Script::Stream(vec![Step::Delay(
        Duration::from_secs(3_600),
    )])]);
    let ws = tempfile::tempdir().unwrap();
    let mut app = ready(&mut core, ws.path()).await;
    app.set_draft("first");
    app.composer_key(ComposerKey::Enter);
    core.pump(&mut app).await;
    assert_eq!(app.composer_mode(), ComposerMode::Running);
    // Another client (or a race) sends to the same session.
    let session = app.selected_session.clone().unwrap();
    let client = core.handle.clone();
    let result = async move {
        client
            .execute(Command::Send {
                session,
                input_id: "other".into(),
                content: Content {
                    text: "second".into(),
                    attachments: vec![],
                },
            })
            .await
    };
    let task = tokio::spawn(result);
    assert!(core.runtime.dispatch_next().await.unwrap());
    assert!(matches!(
        task.await.unwrap().unwrap(),
        CommandResult::Rejected {
            code: RejectCode::InvalidState,
            ..
        }
    ));
}

#[tokio::test]
async fn closed_runtime_surfaces_transport_error() {
    let core = core(vec![]);
    let handle = core.handle.clone();
    drop(core);
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
