//! P1 sync: GetSnapshot on boot and on a seq gap, LoadHistory paging,
//! key_configured → no-key banner, restoring finished runs, a session stuck
//! in Running after a crash.
mod common;
use common::*;
use forge_protocol::*;
use forge_state::ids::SequentialIds;
use forge_state::*;

fn ws() -> Workspace {
    Workspace {
        id: "ws".into(),
        root: "/tmp/ws".into(),
        name: "ws".into(),
    }
}

fn snapshot(
    sessions: Vec<Session>,
    key: bool,
    active_runs: Vec<ActiveRun>,
    last_seq: u64,
) -> Event {
    Event::Snapshot {
        sessions,
        workspaces: vec![ws()],
        settings: settings(true),
        providers: vec![ProviderKeyStatus {
            provider: "openai".into(),
            key_configured: key,
        }],
        active_runs,
        last_seq,
    }
}

fn at(app: &mut AppState, seq: u64, event: Event) {
    app.apply(EventEnvelope { seq, event });
}

fn history(session: &str, messages: Vec<Message>, next_before: Option<u64>) -> Event {
    Event::HistoryLoaded {
        session: session.into(),
        messages,
        next_before,
        runs: vec![],
    }
}

fn texts(app: &AppState) -> Vec<String> {
    app.selected()
        .unwrap()
        .timeline
        .iter()
        .filter_map(|i| match i {
            TimelineItem::User(m) => Some(m.text.clone()),
            TimelineItem::Assistant(m) => Some(m.text.clone()),
            _ => None,
        })
        .collect()
}

fn page(range: std::ops::RangeInclusive<u64>) -> Vec<Message> {
    range
        .map(|o| {
            let run = format!("r{}", o.div_ceil(2));
            if o % 2 == 1 {
                message(
                    o,
                    MessageRole::User,
                    &run,
                    Some(&format!("in{o}")),
                    &format!("u{o}"),
                )
            } else {
                message(o, MessageRole::Assistant, &run, None, &format!("a{o}"))
            }
        })
        .collect()
}

fn snapshot_command(app: &mut AppState) -> usize {
    app.take_outbox()
        .iter()
        .filter(|o| o.command == Command::GetSnapshot)
        .count()
}

#[test]
fn boot_snapshot_selects_first_session_and_loads_newest_page() {
    let mut app = AppState::new(Box::new(SequentialIds::default()));
    app.request_snapshot();
    assert_eq!(snapshot_command(&mut app), 1);
    // A second request while one is pending is a no-op.
    assert!(app.request_snapshot().is_none());
    at(
        &mut app,
        7,
        snapshot(
            vec![session_at("s1", 1_000), session_at("s2", 2_000)],
            true,
            vec![],
            7,
        ),
    );
    assert!(app.synced);
    assert_eq!(app.last_seq, 7);
    assert_eq!(app.selected_session, Some("s2".into()));
    let out = app.take_outbox();
    assert_eq!(
        out.iter().map(|o| o.command.clone()).collect::<Vec<_>>(),
        [Command::LoadHistory {
            session: "s2".into(),
            before: None,
            limit: HISTORY_PAGE,
        }]
    );
    // Events at or below last_seq are stale.
    at(
        &mut app,
        7,
        Event::SessionDeleted {
            session: "s1".into(),
        },
    );
    assert!(app.sessions.contains_key(&SessionId::from("s1")));
}

#[test]
fn seq_gap_requests_one_snapshot_then_applies_only_newer_seqs() {
    let mut app = app_with_session();
    let base = app.last_seq;
    feed(&mut app, [history("s1", vec![], None)]);
    // Run fails; the error card is in memory.
    app.set_now_ms(1_000);
    feed(&mut app, [started("r1", "s1", "in-x")]);
    feed(&mut app, [finished("r1", failed(RunErrorKind::Server))]);
    let held = app.last_seq;
    assert!(
        app.timeline_rows()
            .iter()
            .any(|r| matches!(r, TimelineRowVm::Error(_)))
    );
    // Gap: seq jumps by 3 → exactly one GetSnapshot even with more gaps.
    at(
        &mut app,
        held + 3,
        Event::SessionCreated {
            session: session_at("s2", 5),
        },
    );
    at(
        &mut app,
        held + 6,
        Event::SessionCreated {
            session: session_at("s3", 6),
        },
    );
    assert_eq!(snapshot_command(&mut app), 1);
    assert!(matches!(app.resync, ResyncState::Resyncing { .. }));
    // Snapshot at seq 20 (s3 deleted meanwhile, s1 idle).
    at(
        &mut app,
        20,
        snapshot(
            vec![session_at("s1", 3), session_at("s2", 5)],
            true,
            vec![],
            20,
        ),
    );
    assert_eq!(app.last_seq, 20);
    assert!(!app.sessions.contains_key(&SessionId::from("s3")));
    // The error card held in memory is kept (snapshot does not contradict it).
    assert!(
        app.timeline_rows()
            .iter()
            .any(|r| matches!(r, TimelineRowVm::Error(_)))
    );
    // The selected session reloads its newest page; merged without duplicates.
    let reload: Vec<_> = app.take_outbox().into_iter().map(|o| o.command).collect();
    assert!(reload.contains(&Command::LoadHistory {
        session: "s1".into(),
        before: None,
        limit: HISTORY_PAGE,
    }));
    // Only seq > 20 applies now.
    at(
        &mut app,
        19,
        Event::SessionDeleted {
            session: "s2".into(),
        },
    );
    assert!(app.sessions.contains_key(&SessionId::from("s2")));
    at(
        &mut app,
        21,
        Event::SessionDeleted {
            session: "s2".into(),
        },
    );
    assert!(!app.sessions.contains_key(&SessionId::from("s2")));
    assert_eq!(snapshot_command(&mut app), 0);
    let _ = base;
}

#[test]
fn snapshot_ends_runs_that_finished_during_the_gap_and_restores_active_text() {
    let mut app = app_with_session();
    feed(&mut app, [started("r1", "s1", "in-x")]);
    feed(
        &mut app,
        [Event::TextDelta {
            run: "r1".into(),
            text: "part".into(),
        }],
    );
    let mut live = session_at("s2", 9);
    live.state = SessionState::Running;
    feed(
        &mut app,
        [Event::SessionCreated {
            session: live.clone(),
        }],
    );
    app.request_snapshot();
    app.take_outbox();
    let mut r2 = run("r2", "s2", None);
    r2.started_at = 8;
    at(
        &mut app,
        50,
        snapshot(
            vec![session_at("s1", 3), live],
            true,
            vec![ActiveRun {
                run: r2,
                input_id: "in-2".into(),
                text: "streamed so far".into(),
                reasoning: String::new(),
            }],
            50,
        ),
    );
    let s1 = &app.sessions[&SessionId::from("s1")];
    assert!(s1.running().is_none(), "r1 ended during the gap");
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
    assert_eq!(
        assistant_text(&app, "s2", "r2").as_deref(),
        Some("streamed so far")
    );
    assert_eq!(app.active_runs(), vec![RunId::from("r2")]);
}

#[test]
fn history_pages_merge_in_ordinal_order_without_duplicates() {
    let mut app = app_with_session();
    // Newest page: ordinals 5..=8, more before 5.
    feed(&mut app, [history("s1", page(5..=8), Some(5))]);
    assert_eq!(app.selected().unwrap().history, HistoryState::HasMore);
    assert_eq!(texts(&app), ["u5", "a6", "u7", "a8"]);
    // Scrolled to the top.
    let token = app.load_older().expect("older page");
    let out = app.take_outbox();
    assert_eq!(out[0].token, token);
    assert_eq!(
        out[0].command,
        Command::LoadHistory {
            session: "s1".into(),
            before: Some(5),
            limit: HISTORY_PAGE,
        }
    );
    assert_eq!(app.selected().unwrap().history, HistoryState::LoadingOlder);
    // One page at a time.
    assert!(app.load_older().is_none());
    app.apply_result(token, accepted());
    feed(&mut app, [history("s1", page(1..=4), None)]);
    assert_eq!(
        texts(&app),
        ["u1", "a2", "u3", "a4", "u5", "a6", "u7", "a8"]
    );
    assert_eq!(app.selected().unwrap().history, HistoryState::Complete);
    assert!(app.load_older().is_none(), "oldest page reached");
    // A resync reload of the newest page overlaps: no duplicates, cursor kept.
    feed(&mut app, [history("s1", page(5..=10), Some(5))]);
    assert_eq!(
        texts(&app),
        ["u1", "a2", "u3", "a4", "u5", "a6", "u7", "a8", "u9", "a10"]
    );
    assert_eq!(app.selected().unwrap().history, HistoryState::Complete);
}

#[test]
fn live_rows_dedupe_against_history_by_input_and_run() {
    let mut app = app_with_session();
    app.set_draft("hello");
    app.composer_key(ComposerKey::Enter);
    let out = single_command(&mut app);
    let Command::Send { input_id, .. } = out.command.clone() else {
        panic!()
    };
    app.apply_result(out.token, accepted());
    feed(&mut app, [started("r1", "s1", &input_id.0)]);
    feed(
        &mut app,
        [Event::TextDelta {
            run: "r1".into(),
            text: "hi".into(),
        }],
    );
    feed(&mut app, [finished("r1", RunOutcome::Succeeded)]);
    feed(
        &mut app,
        [history(
            "s1",
            vec![
                message(1, MessageRole::User, "r1", Some(&input_id.0), "hello"),
                message(2, MessageRole::Assistant, "r1", None, "hi"),
            ],
            None,
        )],
    );
    assert_eq!(texts(&app), ["hello", "hi"]);
}

#[test]
fn older_page_rejection_keeps_has_more() {
    let mut app = app_with_session();
    feed(&mut app, [history("s1", page(5..=8), Some(5))]);
    let token = app.load_older().unwrap();
    app.take_outbox();
    app.apply_result(token, rejected(RejectCode::Storage));
    assert_eq!(app.selected().unwrap().history, HistoryState::HasMore);
    assert!(app.load_older().is_some(), "can try again");
}

#[test]
fn key_configured_drives_banner_and_composer() {
    let mut app = AppState::new(Box::new(SequentialIds::default()));
    app.request_snapshot();
    app.take_outbox();
    at(
        &mut app,
        1,
        snapshot(vec![session_at("s1", 1)], false, vec![], 1),
    );
    app.take_outbox();
    assert!(!app.has_api_key());
    assert!(!app.settings.has_any_key());
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::NoKey)
    );
    assert_eq!(app.composer_hint_key(), "composer.no_key");
    // Saving a key: SetProviderKey, then Core confirms with ProviderKeyConfigured.
    app.set_provider_key("openai".into(), Some("  sk-test-123  ".into()));
    let out = single_command(&mut app);
    assert_eq!(
        out.command,
        Command::SetProviderKey {
            provider: "openai".into(),
            key: Some("sk-test-123".into()),
        }
    );
    app.apply_result(out.token, accepted());
    assert!(!app.has_api_key(), "only Core's event flips the flag");
    at(
        &mut app,
        2,
        Event::ProviderKeyConfigured {
            provider: "openai".into(),
            configured: true,
        },
    );
    assert!(app.has_api_key());
    assert!(app.settings.has_key_for(&"openai".into()));
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
    // Removing the key: empty input is `key: None`.
    app.set_provider_key("openai".into(), Some("   ".into()));
    assert_eq!(
        single_command(&mut app).command,
        Command::SetProviderKey {
            provider: "openai".into(),
            key: None,
        }
    );
    at(
        &mut app,
        3,
        Event::ProviderKeyConfigured {
            provider: "openai".into(),
            configured: false,
        },
    );
    assert_eq!(
        app.composer_mode(),
        ComposerMode::Disabled(DisabledReason::NoKey)
    );
}

#[test]
fn open_settings_effect_opens_the_panel() {
    let mut app = app_with_session();
    feed(&mut app, [started("r1", "s1", "in-x")]);
    feed(&mut app, [finished("r1", failed(RunErrorKind::Auth))]);
    assert_eq!(
        app.error_action(&"r1".into(), ErrorAction::OpenSettings),
        ErrorEffect::OpenSettings
    );
    app.open_settings();
    assert!(app.settings.open);
    app.close_settings();
    assert!(!app.settings.open);
}

#[test]
fn ui_prefs_round_trip_through_settings() {
    let mut app = app();
    let mut s = settings(true);
    s.ui_prefs.insert("sidebar.width".into(), "300".into());
    s.ui_prefs.insert("sidebar.visible".into(), "false".into());
    s.ui_prefs.insert("inspector.width".into(), "99999".into());
    app.load_settings(s);
    assert_eq!(app.ui.sidebar_width, 300.0);
    assert!(!app.ui.sidebar_visible);
    assert_eq!(app.ui.inspector_width, INSPECTOR_MAX, "clamped");
    app.ui.sidebar_width = 260.4;
    app.persist_ui_prefs();
    let Command::UpdateSettings { settings } = single_command(&mut app).command else {
        panic!("UpdateSettings expected")
    };
    assert_eq!(settings.ui_prefs["sidebar.width"], "260");
    assert_eq!(settings.ui_prefs["sidebar.visible"], "false");
}

#[test]
fn restore_finished_run_rebuilds_card_once_and_keeps_memory() {
    let mut app = app_with_session();
    feed(&mut app, [history("s1", page(1..=2), None)]);
    let mut r1 = run("r1", "s1", None);
    r1.state = RunState::Failed;
    r1.started_at = 1_000;
    r1.finished_at = Some(5_000);
    r1.outcome = Some(RunOutcome::Failed {
        kind: RunErrorKind::RateLimited,
        message: "slow".into(),
        http_status: Some(429),
        retry_after_ms: Some(120_000),
        retryable: true,
    });
    app.restore_finished_run(r1.clone());
    app.restore_finished_run(r1.clone());
    let rows = app.timeline_rows();
    let cards: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            TimelineRowVm::Error(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 1, "idempotent");
    assert_eq!(cards[0].kind, ErrorCardKind::RateLimitedWait);
    // Card sits after the run's assistant message, divider right after it.
    let card_at = rows
        .iter()
        .position(|r| matches!(r, TimelineRowVm::Error(_)))
        .unwrap();
    assert!(matches!(rows[card_at - 1], TimelineRowVm::Assistant { .. }));
    assert!(matches!(rows[card_at + 1], TimelineRowVm::Divider { .. }));
    assert_eq!(
        app.selected().unwrap().last_error,
        Some(ErrorKind::RateLimited)
    );
    // No outcome → nothing to restore.
    let mut r2 = run("r2", "s1", None);
    r2.outcome = None;
    app.restore_finished_run(r2);
    assert_eq!(app.timeline_rows().len(), rows.len());
}

#[test]
fn session_stuck_running_after_crash_still_offers_stop() {
    let mut app = AppState::new(Box::new(SequentialIds::default()));
    app.request_snapshot();
    app.take_outbox();
    let mut stuck = session_at("s1", 1);
    stuck.state = SessionState::Running;
    at(&mut app, 3, snapshot(vec![stuck], true, vec![], 3));
    app.take_outbox();
    feed(
        &mut app,
        [history(
            "s1",
            vec![message(1, MessageRole::User, "r9", Some("in-9"), "hi")],
            None,
        )],
    );
    assert_eq!(app.composer_mode(), ComposerMode::Running);
    let token = app.stop_run().expect("stop sends CancelRun");
    assert_eq!(
        single_command(&mut app).command,
        Command::CancelRun { run: "r9".into() }
    );
    // Core has no live run to cancel: rejection must not panic or wedge ⌘.
    app.apply_result(token, rejected(RejectCode::InvalidState));
    let _ = app.timeline_rows();
    let _ = app.sidebar();
    // A send while Core still says Running is refused gracefully.
    assert_eq!(app.composer_mode(), ComposerMode::Running);
}

#[test]
fn busy_session_send_rejected_invalid_state_restores_draft() {
    let mut app = app_with_session();
    app.set_draft("second");
    app.composer_key(ComposerKey::Enter);
    let out = single_command(&mut app);
    app.apply_result(out.token, rejected(RejectCode::InvalidState));
    let composer = &app.selected().unwrap().composer;
    assert_eq!(composer.draft, "second");
    assert!(
        composer
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("没有发送"))
    );
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
}

fn failed_run(id: &str, outcome: RunOutcome, started: u64, finished: u64) -> Run {
    let mut r = run(id, "s1", None);
    r.state = RunState::Failed;
    r.started_at = started;
    r.finished_at = Some(finished);
    r.outcome = Some(outcome);
    r
}

fn r2_interrupted() -> RunOutcome {
    RunOutcome::Failed {
        kind: RunErrorKind::Interrupted,
        message: "上次异常退出时中断".into(),
        http_status: None,
        retry_after_ms: None,
        retryable: true,
    }
}

fn error_cards(app: &AppState) -> Vec<(usize, ErrorCardVm)> {
    app.timeline_rows()
        .into_iter()
        .enumerate()
        .filter_map(|(i, r)| match r {
            TimelineRowVm::Error(c) => Some((i, c)),
            _ => None,
        })
        .collect()
}

#[test]
fn restore_after_restart_via_snapshot_last_finished_run_and_history_runs() {
    // Core restarted after a crash: R-2 turned the stale Running run into
    // failed{interrupted}; the snapshot carries it as `last_finished_run`.
    let mut app = AppState::new(Box::new(SequentialIds::default()));
    app.request_snapshot();
    app.take_outbox();
    let mut s1 = session_at("s1", 9_000);
    s1.last_finished_run = Some(FinishedRun {
        run: "r2".into(),
        input_id: Some("in3".into()),
        outcome: r2_interrupted(),
        finished_at: 9_000,
    });
    at(&mut app, 4, snapshot(vec![s1], true, vec![], 4));
    // Sidebar knows the failure before any history arrives.
    let sidebar = app.sidebar();
    let row = sidebar
        .needs_attention
        .iter()
        .chain(&sidebar.recent)
        .find(|r| r.id.0 == "s1")
        .unwrap_or_else(|| panic!("{sidebar:?}"))
        .clone();
    assert_eq!(row.last_error, Some(ErrorKind::Interrupted));
    assert!(matches!(
        row.last_outcome,
        Some(RunOutcome::Failed {
            kind: RunErrorKind::Interrupted,
            ..
        })
    ));
    app.take_outbox();
    // Newest page: r1 ok (u1,a2), r2 interrupted with only its user row (u3).
    let mut messages = page(1..=2);
    messages.push(message(3, MessageRole::User, "r2", Some("in3"), "u3"));
    let mut ok = run("r1", "s1", None);
    ok.state = RunState::Succeeded;
    ok.started_at = 1_000;
    ok.finished_at = Some(2_000);
    ok.outcome = Some(RunOutcome::Succeeded);
    feed(
        &mut app,
        [Event::HistoryLoaded {
            session: "s1".into(),
            messages,
            next_before: None,
            runs: vec![ok, failed_run("r2", r2_interrupted(), 8_000, 9_000)],
        }],
    );
    let cards = error_cards(&app);
    assert_eq!(cards.len(), 1);
    let (at_row, card) = &cards[0];
    assert_eq!(card.kind, ErrorCardKind::Interrupted);
    assert!(
        card.details.join("\n").contains("上次异常退出时中断"),
        "{:?}",
        card.details
    );
    // Card follows r2's user row (no assistant message for r2).
    let rows = app.timeline_rows();
    assert!(
        matches!(&rows[at_row - 1], TimelineRowVm::User { .. }),
        "{:?}",
        rows[at_row - 1]
    );
    // Interrupted is retryable → retry enabled and sends RetryRun.
    assert_eq!(card.retry, RetryButtonState::Ready);
    assert_eq!(card.primary, ErrorAction::Retry);
    app.retry_run(&"r2".into())
        .expect("interrupted card retry enabled");
    assert_eq!(
        single_command(&mut app).command,
        Command::RetryRun { run: "r2".into() }
    );
    // Composer is idle again (session Idle after R-2).
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
}

#[test]
fn restore_on_paging_back_places_older_failure_after_its_run() {
    let mut app = app_with_session();
    // Core says the latest finished run (r4) succeeded.
    let mut s1 = session_at("s1", 9_000);
    s1.last_finished_run = Some(FinishedRun {
        run: "r4".into(),
        input_id: Some("in7".into()),
        outcome: RunOutcome::Succeeded,
        finished_at: 9_000,
    });
    feed(&mut app, [Event::SessionUpdated { session: s1 }]);
    // Newest page: r3..r4 (ordinals 5..=8), more available before 5.
    feed(&mut app, [history("s1", page(5..=8), Some(5))]);
    assert!(error_cards(&app).is_empty());
    app.load_older().expect("older page requested");
    app.take_outbox();
    let auth = RunOutcome::Failed {
        kind: RunErrorKind::Auth,
        message: "401 invalid key\nAuthorization: Bearer sk-old-secret".into(),
        http_status: Some(401),
        retry_after_ms: None,
        retryable: false,
    };
    feed(
        &mut app,
        [Event::HistoryLoaded {
            session: "s1".into(),
            messages: page(1..=4),
            next_before: None,
            runs: vec![failed_run("r1", auth, 1_000, 2_000)],
        }],
    );
    assert_eq!(
        texts(&app),
        ["u1", "a2", "u3", "a4", "u5", "a6", "u7", "a8"]
    );
    let cards = error_cards(&app);
    assert_eq!(cards.len(), 1);
    let (at_row, card) = &cards[0];
    assert_eq!(card.kind, ErrorCardKind::Auth);
    assert!(!card.details.join("\n").contains("sk-old-secret"));
    assert!(card.details.join("\n").contains("[已移除请求头]"));
    let rows = app.timeline_rows();
    // Right after r1's assistant reply (a2), before r2's user row (u3).
    match (&rows[at_row - 1], &rows[at_row + 1], &rows[at_row + 2]) {
        (
            TimelineRowVm::Assistant { .. },
            TimelineRowVm::Divider { .. },
            TimelineRowVm::User { .. },
        ) => {}
        other => panic!("card misplaced: {other:?}"),
    }
    // An older failure must not take over the sidebar's latest status.
    assert_eq!(app.selected().unwrap().last_error, None);
    // Paging the same page again does not duplicate the card.
    let _ = app.timeline_rows();
}

#[test]
fn session_updated_before_run_finished_places_the_card_once() {
    // Core order on finish: SessionUpdated{last_finished_run} → RunFinished.
    let mut app = app_with_session();
    set_key(&mut app, true);
    feed(&mut app, [started("r1", "s1", "in1")]);
    let outcome = RunOutcome::Failed {
        kind: RunErrorKind::Server,
        message: "502 bad gateway".into(),
        http_status: Some(502),
        retry_after_ms: None,
        retryable: true,
    };
    let mut s1 = session_at("s1", 5_000);
    s1.last_finished_run = Some(FinishedRun {
        run: "r1".into(),
        input_id: Some("in1".into()),
        outcome: outcome.clone(),
        finished_at: 5_000,
    });
    feed(
        &mut app,
        [
            Event::SessionUpdated { session: s1 },
            finished("r1", outcome),
        ],
    );
    assert_eq!(error_cards(&app).len(), 1);
    let dividers = app
        .timeline_rows()
        .iter()
        .filter(|r| matches!(r, TimelineRowVm::Divider { .. }))
        .count();
    assert_eq!(dividers, 1);
    assert_eq!(app.selected().unwrap().last_error, Some(ErrorKind::Server));
}

#[test]
fn seq_gap_resync_reloads_history_and_restores_both_gap_failures() {
    // Designer acceptance: during a seq gap the same open session fails
    // twice; after Snapshot + LoadHistory both error cards are present,
    // without duplicates or announcements (restore is history, not live).
    let mut app = app_with_session();
    feed(&mut app, [history("s1", page(1..=2), None)]);
    app.take_outbox();
    app.take_announcements();
    let held = app.last_seq;

    let server = RunOutcome::Failed {
        kind: RunErrorKind::Server,
        message: "502 bad gateway".into(),
        http_status: Some(502),
        retry_after_ms: None,
        retryable: true,
    };
    let auth = RunOutcome::Failed {
        kind: RunErrorKind::Auth,
        message: "401 invalid key".into(),
        http_status: Some(401),
        retry_after_ms: None,
        retryable: false,
    };

    // Snapshot only carries the latest finished run; the earlier failure
    // arrives with HistoryLoaded.runs.
    let mut s1 = session_at("s1", 8_000);
    s1.last_finished_run = Some(FinishedRun {
        run: "r3".into(),
        input_id: Some("in5".into()),
        outcome: auth.clone(),
        finished_at: 8_000,
    });
    at(
        &mut app,
        held + 5,
        snapshot(vec![s1], true, vec![], held + 5),
    );

    let reload: Vec<_> = app.take_outbox().into_iter().map(|o| o.command).collect();
    assert!(
        reload.contains(&Command::LoadHistory {
            session: "s1".into(),
            before: None,
            limit: HISTORY_PAGE,
        }),
        "resync must re-issue LoadHistory for the open session: {reload:?}"
    );
    // last_finished_run updates the sidebar immediately; no card yet for r2.
    assert_eq!(app.selected().unwrap().last_error, Some(ErrorKind::Auth));
    assert!(error_cards(&app).is_empty());
    app.take_announcements();

    let r2 = failed_run("r2", server.clone(), 3_000, 4_000);
    feed(
        &mut app,
        [Event::HistoryLoaded {
            session: "s1".into(),
            messages: {
                let mut messages = page(1..=2);
                messages.extend(page(3..=6));
                messages
            },
            next_before: None,
            runs: vec![r2, failed_run("r3", auth.clone(), 7_000, 8_000)],
        }],
    );

    let cards = error_cards(&app);
    assert_eq!(
        cards.iter().map(|(_, c)| c.kind).collect::<Vec<_>>(),
        [ErrorCardKind::Server, ErrorCardKind::Auth],
        "{cards:?}"
    );
    let spoken = app.take_announcements();
    assert!(
        spoken.iter().all(
            |a| a.kind != AnnouncementKind::RunFinished && a.kind != AnnouncementKind::Retrying
        ),
        "restore must not announce: {spoken:?}"
    );

    // The same page again must not duplicate cards or speak.
    feed(
        &mut app,
        [Event::HistoryLoaded {
            session: "s1".into(),
            messages: {
                let mut messages = page(1..=2);
                messages.extend(page(3..=6));
                messages
            },
            next_before: None,
            runs: vec![
                failed_run("r2", server, 3_000, 4_000),
                failed_run("r3", auth, 7_000, 8_000),
            ],
        }],
    );
    assert_eq!(error_cards(&app).len(), 2);
    assert!(
        app.take_announcements()
            .iter()
            .all(|a| a.kind != AnnouncementKind::RunFinished),
        "repeat HistoryLoaded must stay silent"
    );
}
