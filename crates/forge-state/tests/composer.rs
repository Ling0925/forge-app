//! Composer rules (§6.9, §7.1) and R10 idempotency ids.
mod common;
use common::*;
use forge_protocol::*;
use forge_state::*;

fn input_id(out: &Outgoing) -> InputId {
    match &out.command {
        Command::Send { input_id, .. } => input_id.clone(),
        other => panic!("expected Send, got {other:?}"),
    }
}

#[test]
fn idle_enter_sends_with_ui_generated_input_id_and_clears_on_accept() {
    let mut app = app_with_session();
    app.set_draft("修复登录 bug");
    let outcome = app.composer_key(ComposerKey::Enter);
    let out = single_command(&mut app);
    assert_eq!(outcome, KeyOutcome::Sent(out.token));
    assert_eq!(
        out.command,
        Command::Send {
            session: "s1".into(),
            input_id: "in-1".into(),
            content: Content {
                text: "修复登录 bug".into(),
                attachments: vec![],
            },
        }
    );
    assert_eq!(app.composer_mode(), ComposerMode::Sending);
    // Read-only while sending.
    app.set_draft("changed");
    assert_eq!(app.selected().unwrap().composer.draft, "修复登录 bug");
    app.apply_result(out.token, accepted());
    let composer = &app.selected().unwrap().composer;
    assert_eq!(composer.draft, "");
    assert_eq!(composer.in_flight, None);
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
}

#[test]
fn ui_cp_05_rejected_send_restores_text_and_resend_uses_new_id() {
    let mut app = app_with_session();
    app.set_draft("hello");
    app.composer_key(ComposerKey::Enter);
    let first = single_command(&mut app);
    // Transport replay of the same request keeps the same input_id (R10).
    let replay = app.replay(first.token).unwrap();
    assert_eq!(replay, first);
    app.apply_result(first.token, rejected(RejectCode::Storage));
    let composer = &app.selected().unwrap().composer;
    assert_eq!(composer.draft, "hello");
    assert_eq!(composer.error.as_deref(), Some("没有发送：数据写入失败"));
    assert!(app.replay(first.token).is_none());
    app.composer_key(ComposerKey::Enter);
    let second = single_command(&mut app);
    assert_ne!(input_id(&first), input_id(&second));
    assert_eq!(app.selected().unwrap().composer.error, None);
}

#[test]
fn enter_while_in_flight_does_not_send_twice() {
    let mut app = app_with_session();
    app.set_draft("x");
    app.composer_key(ComposerKey::Enter);
    assert_eq!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Ignored(IgnoreReason::InFlight)
    );
    assert_eq!(app.take_outbox().len(), 1);
}

#[test]
fn transport_failure_behaves_like_rejection() {
    let mut app = app_with_session();
    app.set_draft("x");
    app.composer_key(ComposerKey::Enter);
    let out = single_command(&mut app);
    app.transport_failed(out.token);
    assert_eq!(
        app.selected().unwrap().composer.error.as_deref(),
        Some("没有发送：无法连接后台")
    );
}

#[test]
fn ui_cp_02_ime_composition_owns_enter() {
    let mut app = app_with_session();
    app.set_draft("ni");
    app.set_ime_composing(true);
    for key in [
        ComposerKey::Enter,
        ComposerKey::AltEnter,
        ComposerKey::ShiftEnter,
    ] {
        assert_eq!(app.composer_key(key), KeyOutcome::PassToIme);
    }
    assert!(app.take_outbox().is_empty());
    app.set_ime_composing(false);
    assert!(matches!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Sent(_)
    ));
}

#[test]
fn shift_enter_inserts_newline_and_empty_draft_is_ignored() {
    let mut app = app_with_session();
    app.set_draft("   \n ");
    assert_eq!(
        app.composer_key(ComposerKey::ShiftEnter),
        KeyOutcome::InsertNewline
    );
    assert_eq!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Ignored(IgnoreReason::Empty)
    );
    assert!(!app.can_send());
    assert!(app.take_outbox().is_empty());
}

#[test]
fn alt_enter_when_idle_equals_send() {
    let mut app = app_with_session();
    app.set_draft("x");
    assert!(matches!(
        app.composer_key(ComposerKey::AltEnter),
        KeyOutcome::Sent(_)
    ));
    assert!(matches!(
        single_command(&mut app).command,
        Command::Send { .. }
    ));
}

fn start_run(app: &mut AppState) {
    feed(
        app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    app.take_announcements();
}

#[test]
fn p1_busy_session_does_not_queue_or_steer() {
    let mut app = app_with_session();
    start_run(&mut app);
    app.set_draft("next");
    assert_eq!(app.composer_mode(), ComposerMode::Running);
    assert_eq!(app.composer_hint_key(), "composer.hint.running_p1");
    assert_eq!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Ignored(IgnoreReason::Busy)
    );
    assert_eq!(
        app.composer_key(ComposerKey::AltEnter),
        KeyOutcome::Ignored(IgnoreReason::Busy)
    );
    assert!(!app.can_send());
    assert!(app.take_outbox().is_empty());
}

#[test]
fn p3_busy_enter_queues_and_alt_enter_steers_with_steer_id() {
    let mut app = app_with_session();
    app.capabilities = Capabilities::P3;
    start_run(&mut app);
    assert_eq!(app.composer_hint_key(), "composer.hint.running");
    app.set_draft("queued");
    assert!(matches!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Sent(_)
    ));
    let send = single_command(&mut app);
    assert!(matches!(send.command, Command::Send { .. }));
    app.apply_result(send.token, accepted());

    app.set_draft("steer");
    let outcome = app.composer_key(ComposerKey::AltEnter);
    let steer = single_command(&mut app);
    assert_eq!(outcome, KeyOutcome::Steered(steer.token));
    assert_eq!(
        steer.command,
        Command::Steer {
            run: "r1".into(),
            steer_id: "steer-2".into(),
            content: Content {
                text: "steer".into(),
                attachments: vec![],
            },
        }
    );
    app.apply_result(steer.token, rejected(RejectCode::Conflict));
    let composer = &app.selected().unwrap().composer;
    assert_eq!(composer.draft, "steer");
    assert!(composer.error.is_some());
}

#[test]
fn no_key_and_storage_error_disable_the_composer() {
    let mut app = app_with_session();
    set_key(&mut app, false);
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::NoKey)
    );
    assert_eq!(app.composer_hint_key(), "composer.no_key");
    app.set_draft("x");
    assert_eq!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Ignored(IgnoreReason::Disabled)
    );
    set_key(&mut app, true);
    app.boot = BootState::StorageError {
        code: "SQLITE_READONLY".into(),
        path: "/data".into(),
    };
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::StorageError)
    );
}

#[test]
fn no_selected_session_disables_composer() {
    let mut app = app();
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::NoSession)
    );
    assert_eq!(
        app.composer_key(ComposerKey::Enter),
        KeyOutcome::Ignored(IgnoreReason::NoSession)
    );
}

#[test]
fn ui_cp_04_stop_sends_cancel_once() {
    let mut app = app_with_session();
    start_run(&mut app);
    let token = app.primary_button().expect("stop");
    let out = single_command(&mut app);
    assert_eq!(out.token, token);
    assert_eq!(out.command, Command::CancelRun { run: "r1".into() });
    assert_eq!(app.stop_run(), None);
    feed(
        &mut app,
        [Event::RunStateChanged {
            run: "r1".into(),
            state: RunState::Cancelling,
        }],
    );
    assert_eq!(app.composer_mode(), ComposerMode::Cancelling);
    assert_eq!(app.stop_run(), None);
    assert_eq!(app.primary_button(), None);
    assert!(app.take_outbox().is_empty());
}

#[test]
fn cancel_rejected_not_implemented_allows_retry_and_toasts() {
    let mut app = app_with_session();
    start_run(&mut app);
    let token = app.stop_run().unwrap();
    app.take_outbox();
    app.apply_result(token, rejected(RejectCode::NotImplemented));
    assert_eq!(app.take_toasts()[0].text, "没有发送：此功能还没有实现");
    assert!(app.stop_run().is_some());
}

#[test]
fn cancel_after_terminal_is_silent() {
    let mut app = app_with_session();
    start_run(&mut app);
    let token = app.stop_run().unwrap();
    app.apply_result(token, rejected(RejectCode::AlreadyFinished));
    assert!(app.take_toasts().is_empty());
}

#[test]
fn ui_tl_04_example_fills_composer_without_sending() {
    let mut app = app_with_session();
    assert!(app.selected().unwrap().is_empty());
    assert!(app.fill_example(2));
    assert_eq!(
        app.selected().unwrap().composer.draft,
        "找出并修复一个失败的测试"
    );
    assert!(app.take_outbox().is_empty());
    assert!(!app.fill_example(9));
}

#[test]
fn drafts_are_kept_per_session() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::SessionCreated {
            session: session("s2"),
        }],
    );
    app.select_session(&"s1".into());
    app.set_draft("draft one");
    app.select_session(&"s2".into());
    app.set_draft("draft two");
    app.select_session(&"s1".into());
    assert_eq!(app.selected().unwrap().composer.draft, "draft one");
}
