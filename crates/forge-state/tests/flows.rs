//! Quit/Shutdown, settings, onboarding and command palette flows.
mod common;
use common::*;
use forge_protocol::*;
use forge_state::*;

#[test]
fn quit_without_runs_exits_immediately() {
    let mut app = app_with_session();
    assert_eq!(app.request_quit(), QuitDecision::ExitNow);
    assert!(app.should_exit());
    assert!(app.take_outbox().is_empty());
}

#[test]
fn quit_with_runs_confirms_then_sends_shutdown_once() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    assert_eq!(
        app.request_quit(),
        QuitDecision::NeedsConfirm { active_runs: 1 }
    );
    assert_eq!(app.request_quit(), QuitDecision::InProgress);
    app.cancel_quit();
    assert_eq!(app.quit, QuitState::Idle);
    assert!(app.take_outbox().is_empty());

    app.request_quit();
    let token = app.confirm_quit().unwrap();
    assert_eq!(app.confirm_quit(), None);
    let out = single_command(&mut app);
    assert_eq!((out.token, out.command), (token, Command::Shutdown));
    assert!(!app.should_exit());
    app.apply_result(token, accepted());
    feed(
        &mut app,
        [Event::ShutdownComplete {
            stopped: vec!["r1".into()],
            unfinished: vec![],
        }],
    );
    assert_eq!(
        app.quit,
        QuitState::ReadyToExit {
            stopped: vec!["r1".into()],
            unfinished: vec![]
        }
    );
}

#[test]
fn shutdown_rejected_still_exits_and_lists_unfinished() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    app.request_quit();
    let token = app.confirm_quit().unwrap();
    app.apply_result(token, rejected(RejectCode::NotImplemented));
    assert_eq!(
        app.quit,
        QuitState::ReadyToExit {
            stopped: vec![],
            unfinished: vec!["r1".into()]
        }
    );
}

#[test]
fn shutdown_ui_timeout() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    app.set_now_ms(1_000);
    app.request_quit();
    app.confirm_quit();
    app.set_now_ms(1_000 + UI_SHUTDOWN_TIMEOUT_MS - 1);
    assert!(!app.poll_quit_timeout());
    app.set_now_ms(1_000 + UI_SHUTDOWN_TIMEOUT_MS);
    assert!(app.poll_quit_timeout());
}

#[test]
fn ui_tb_02_unrestricted_session_mode_requires_confirmation() {
    let mut app = app_with_session();
    let id: SessionId = "s1".into();
    assert_eq!(
        app.request_session_permission_mode(&id, PermissionMode::Unrestricted),
        None
    );
    assert_eq!(
        app.settings.confirm,
        Some(ConfirmDialog::UnrestrictedSession(id.clone()))
    );
    app.confirm_dialog_cancel();
    assert_eq!(app.sessions[&id].permission_mode(), &PermissionMode::Ask);
    assert!(app.take_outbox().is_empty());

    app.request_session_permission_mode(&id, PermissionMode::Unrestricted);
    let token = app.confirm_dialog_accept().unwrap();
    assert_eq!(
        single_command(&mut app).command,
        Command::SetPermissionMode {
            session: id.clone(),
            mode: PermissionMode::Unrestricted
        }
    );
    app.apply_result(token, rejected(RejectCode::NotImplemented));
    assert_eq!(app.sessions[&id].permission_mode(), &PermissionMode::Ask);

    assert!(
        app.request_session_permission_mode(&id, PermissionMode::ReadOnly)
            .is_some()
    );
}

#[test]
fn ui_st_04_default_mode_and_settings_revert_on_reject() {
    let mut app = app();
    assert_eq!(
        app.request_default_permission_mode(PermissionMode::Unrestricted),
        None
    );
    app.confirm_dialog_cancel();
    assert_eq!(
        app.settings.settings.as_ref().unwrap().permission_mode,
        PermissionMode::Ask
    );
    let token = app
        .request_default_permission_mode(PermissionMode::ReadOnly)
        .unwrap();
    let out = single_command(&mut app);
    assert!(matches!(
        &out.command,
        Command::UpdateSettings { settings } if settings.permission_mode == PermissionMode::ReadOnly
    ));
    app.apply_result(token, rejected(RejectCode::Storage));
    assert_eq!(
        app.settings.settings.as_ref().unwrap().permission_mode,
        PermissionMode::Ask
    );
    assert_eq!(
        app.settings.error.as_deref(),
        Some("没有发送：数据写入失败")
    );
    // No-op change sends nothing.
    assert_eq!(
        app.request_default_permission_mode(PermissionMode::Ask),
        None
    );
}

#[test]
fn provider_key_status_flow() {
    let mut app = app();
    let openai: ProviderId = "openai".into();
    assert_eq!(
        app.settings.key_status[&openai],
        LocalKeyState::SavedUnverified
    );
    app.test_provider_key(openai.clone());
    assert_eq!(
        single_command(&mut app).command,
        Command::TestProviderKey {
            provider: openai.clone()
        }
    );
    assert_eq!(app.settings.key_status[&openai].label(), "正在验证…");
    feed(
        &mut app,
        [Event::ProviderKeyTested {
            provider: openai.clone(),
            ok: false,
            error_kind: Some(ProviderKeyErrorKind::Auth),
        }],
    );
    assert_eq!(
        app.settings.key_status[&openai].label(),
        "✕ 验证失败：认证错误"
    );
    feed(
        &mut app,
        [Event::ProviderKeyTested {
            provider: openai.clone(),
            ok: true,
            error_kind: None,
        }],
    );
    assert_eq!(app.settings.key_status[&openai], LocalKeyState::Verified);
}

#[test]
fn onboarding_full_flow_keyboard_only() {
    let mut app = AppState::new(Box::new(forge_state::ids::SequentialIds::default()));
    app.load_settings(settings(false));
    app.start_onboarding();
    let step = |app: &AppState| app.onboarding.as_ref().unwrap().step;
    assert!(app.onboarding_next().is_empty());
    assert_eq!(step(&app), OnboardingStep::Workspace);
    // UI-OB-03: no access → next disabled.
    app.onboarding_choose_workspace("/ro".into(), WorkspaceCheck::NoAccess);
    assert!(!app.onboarding_can_advance());
    assert!(app.onboarding_next().is_empty());
    app.onboarding_choose_workspace("/proj".into(), WorkspaceCheck::NotGit);
    let tokens = app.onboarding_next();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        single_command(&mut app).command,
        Command::AddWorkspace {
            path: "/proj".into()
        }
    );
    assert_eq!(step(&app), OnboardingStep::ApiKey);
    // Back and forward again does not add the workspace twice.
    app.onboarding_back();
    assert_eq!(step(&app), OnboardingStep::Workspace);
    app.onboarding_next();
    assert!(app.take_outbox().is_empty());
    feed(
        &mut app,
        [Event::WorkspaceAdded {
            workspace: Workspace {
                id: "ws".into(),
                root: "/proj".into(),
                name: "proj".into(),
            },
        }],
    );
    // Key step: failing verification shows the error and stays.
    assert!(!app.onboarding_can_advance());
    app.onboarding_verify_key("openai".into()).unwrap();
    app.take_outbox();
    assert_eq!(app.onboarding_verify_key("openai".into()), None);
    feed(
        &mut app,
        [Event::ProviderKeyTested {
            provider: "openai".into(),
            ok: false,
            error_kind: Some(ProviderKeyErrorKind::Network),
        }],
    );
    assert_eq!(step(&app), OnboardingStep::ApiKey);
    assert_eq!(
        app.onboarding.as_ref().unwrap().error.as_deref(),
        Some("✕ 验证失败：网络错误")
    );
    app.onboarding_verify_key("openai".into());
    app.take_outbox();
    feed(
        &mut app,
        [Event::ProviderKeyTested {
            provider: "openai".into(),
            ok: true,
            error_kind: None,
        }],
    );
    assert_eq!(step(&app), OnboardingStep::Permissions);
    app.onboarding_set_permission_mode(PermissionMode::ReadOnly);
    app.onboarding_set_notify(true);
    let tokens = app.onboarding_next();
    assert!(app.onboarding.is_none());
    let commands: Vec<_> = app.take_outbox().into_iter().map(|o| o.command).collect();
    assert_eq!(tokens.len(), 2);
    assert!(
        matches!(&commands[0], Command::UpdateSettings { settings } if settings.permission_mode == PermissionMode::ReadOnly)
    );
    assert!(matches!(
        &commands[1],
        Command::CreateSession {
            permission_mode: PermissionMode::ReadOnly,
            ..
        }
    ));
}

#[test]
fn ui_ob_04_skipping_key_leaves_composer_without_key() {
    let mut app = AppState::new(Box::new(forge_state::ids::SequentialIds::default()));
    app.load_settings(settings(false));
    app.start_onboarding();
    app.onboarding_next();
    app.onboarding_choose_workspace("/proj".into(), WorkspaceCheck::ReadWrite);
    app.onboarding_next();
    app.onboarding_skip_key();
    assert!(app.onboarding.as_ref().unwrap().key_skipped);
    feed(
        &mut app,
        [Event::WorkspaceAdded {
            workspace: Workspace {
                id: "ws".into(),
                root: "/proj".into(),
                name: "proj".into(),
            },
        }],
    );
    app.onboarding_next();
    feed(
        &mut app,
        [Event::SessionCreated {
            session: session("first"),
        }],
    );
    assert_eq!(app.selected_session, Some("first".into()));
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::NoKey)
    );
}

#[test]
fn palette_hides_unavailable_actions_and_filters() {
    let mut app = app_with_session();
    app.open_palette();
    let labels = |app: &AppState| -> Vec<String> {
        app.palette_items().into_iter().map(|i| i.label).collect()
    };
    let idle = labels(&app);
    assert!(idle.contains(&"会话 s1".to_owned()));
    assert!(idle.contains(&"新会话".to_owned()));
    assert!(!idle.contains(&"停止".to_owned()));
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    assert!(labels(&app).contains(&"停止".to_owned()));
    app.set_palette_query("侧栏");
    assert_eq!(labels(&app), vec!["显示/隐藏侧栏".to_owned()]);
    assert_eq!(app.palette_execute(), PaletteEffect::None);
    assert!(!app.palette.open);
    assert!(!app.ui.sidebar_visible);
}

#[test]
fn palette_switches_session_and_stops() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::SessionCreated {
            session: session("s2"),
        }],
    );
    app.open_palette();
    app.set_palette_query("s2");
    assert_eq!(app.palette_execute(), PaletteEffect::FocusComposer);
    assert_eq!(app.selected_session, Some("s2".into()));

    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r2", "s2", None),
            input_id: "in-1".into(),
        }],
    );
    app.open_palette();
    app.set_palette_query("停止");
    assert!(matches!(app.palette_execute(), PaletteEffect::Sent(_)));
    assert_eq!(
        single_command(&mut app).command,
        Command::CancelRun { run: "r2".into() }
    );
    app.open_palette();
    app.palette_move(true);
    app.palette_move(true);
    app.palette_move(false);
    assert_eq!(app.palette.selected, 1);
}

#[test]
fn recovery_banner_dismiss() {
    let mut app = app();
    app.recovery = Some(RecoveryReport {
        sessions_checked: 3,
        interrupted_runs: 2,
        uncertain_items: 1,
    });
    app.dismiss_recovery();
    assert!(app.recovery.is_none());
}
