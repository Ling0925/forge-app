mod common;
use forge_core::*;
use forge_protocol::{ApprovalDecision, ApprovalScope, Content, PermissionMode, UncertainAction};
use forge_protocol::{Command, CommandResult, Event};
use forge_protocol::{CommandId, NoticeLevel, RejectCode};
use forge_testkit::provider::{FixedProviderFactory, ScriptedProvider};
use forge_tools::clock::FakeClock;
use std::sync::Arc;

#[tokio::test]
async fn commands_receive_accepted_and_rejected_responses() {
    let (mut runtime, handle, _directory) = runtime(0);
    for response in [
        CommandResult::Accepted {
            id: CommandId::from("accepted"),
        },
        CommandResult::Rejected {
            code: RejectCode::Conflict,
            message: "different input".into(),
        },
    ] {
        let command = Command::CancelRun { run: "run".into() };
        let client = handle.clone();
        let sent = command.clone();
        let task = tokio::spawn(async move { client.execute(sent).await });
        let request = runtime.next_command().await.unwrap();
        assert_eq!(request.command, command);
        request.respond(response.clone()).unwrap();
        assert_eq!(task.await.unwrap().unwrap(), response);
    }
}

#[tokio::test]
async fn events_share_one_sequence_across_subscribers() {
    let (mut runtime, handle, _directory) = runtime(7);
    let mut first = handle.subscribe();
    let mut second = handle.subscribe();
    let closed = handle.subscribe();
    drop(closed);
    for seq in [8, 9] {
        let envelope = runtime
            .publish(Event::Notice {
                level: NoticeLevel::Info,
                code: "test".into(),
                message: "event".into(),
            })
            .unwrap();
        assert_eq!(envelope.seq, seq);
        assert_eq!(first.recv().await.unwrap(), envelope);
        assert_eq!(second.recv().await.unwrap(), envelope);
    }
}

#[tokio::test]
async fn closed_runtime_is_reported() {
    let (runtime, handle, _directory) = runtime(0);
    drop(runtime);
    assert!(
        handle
            .execute(Command::CancelRun { run: "run".into() })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn p1_dispatch_rejects_every_unimplemented_command_without_events() {
    let (mut runtime, handle, _directory) = runtime(0);
    let mut events = handle.subscribe();
    let content = Content {
        text: "input".into(),
        attachments: vec![],
    };
    for command in [
        Command::Steer {
            run: "run".into(),
            steer_id: "steer".into(),
            content: content.clone(),
        },
        Command::EditQueueItem {
            item: "item".into(),
            content,
        },
        Command::RemoveQueueItem {
            item: "item".into(),
        },
        Command::ReorderQueueItem {
            item: "item".into(),
            before: None,
        },
        Command::ResolveUncertain {
            item: "item".into(),
            action: UncertainAction::Requeue,
        },
        Command::PauseQueue {
            session: "session".into(),
        },
        Command::ResumeQueue {
            session: "session".into(),
        },
        Command::CancelRun { run: "run".into() },
        Command::RetryRun { run: "run".into() },
        Command::RetryNow { run: "run".into() },
        Command::DecideApproval {
            approval: "approval".into(),
            decision: ApprovalDecision::Allow,
            scope: ApprovalScope::Once,
        },
        Command::SetPermissionMode {
            session: "session".into(),
            mode: PermissionMode::Ask,
        },
        Command::PreviewRestore {
            checkpoint: "checkpoint".into(),
        },
        Command::RestoreCheckpoint {
            checkpoint: "checkpoint".into(),
            overwrite: vec![],
        },
        Command::Shutdown,
    ] {
        let (response, dispatched) = tokio::join!(handle.execute(command), runtime.dispatch_next());
        assert!(dispatched.unwrap());
        assert_eq!(
            response.unwrap(),
            CommandResult::Rejected {
                code: RejectCode::NotImplemented,
                message: "command is not implemented in P1".into(),
            }
        );
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
    drop(handle);
    assert!(!runtime.dispatch_next().await.unwrap());
}

fn runtime(last_seq: u64) -> (Runtime, RuntimeHandle, tempfile::TempDir) {
    let clock = Arc::new(FakeClock::default());
    let directory = tempfile::tempdir().unwrap();
    let store = forge_store::Store::open(directory.path()).unwrap();
    let (runtime, handle) = Runtime::new(
        RuntimeServices {
            clock: clock.clone(),
            store,
            providers: Arc::new(FixedProviderFactory(Arc::new(ScriptedProvider::new(
                clock,
                [],
            )))),
            secrets: Arc::new(forge_core::secrets::MemorySecretStore::default()),
            initial_settings: common::settings("script"),
        },
        last_seq,
    )
    .unwrap();
    (runtime, handle, directory)
}
