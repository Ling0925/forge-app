//! P1: error cards + retry countdown (§6.7), timeline rows (§6.3/6.4),
//! announcement merging (§8.2), layout rules (§4).
mod common;
use common::*;
use forge_protocol::*;
use forge_state::*;

fn send(app: &mut AppState, text: &str) {
    app.set_draft(text);
    app.composer_key(ComposerKey::Enter);
    let out = single_command(app);
    app.apply_result(out.token, accepted());
}

fn start(app: &mut AppState, id: &str) {
    feed(
        app,
        [Event::RunStarted {
            run: run(id, "s1", None),
            input_id: "in-1".into(),
        }],
    );
}

/// Test shorthand: `"kind[ key=value…][: message]"` → structured P1 outcome.
fn outcome(spec: &str) -> RunOutcome {
    let (head, message) = spec.split_once(": ").unwrap_or((spec, ""));
    let mut parts = head.split_whitespace();
    let word = parts.next().unwrap_or("");
    let mut http_status = None;
    let mut retry_after_ms = None;
    let mut source = UnknownSource::Provider;
    for part in parts {
        match part.split_once('=') {
            Some(("http_status", v)) => http_status = v.parse().ok(),
            Some(("retry_after_ms", v)) => retry_after_ms = v.parse().ok(),
            Some(("source", "storage")) => source = UnknownSource::Storage,
            _ => {}
        }
    }
    let (kind, retryable) = match word {
        "network" => (RunErrorKind::Network, true),
        "rate_limited" => (RunErrorKind::RateLimited, true),
        "auth" => (RunErrorKind::Auth, false),
        "quota" => (RunErrorKind::Quota, false),
        "server" => (RunErrorKind::Server, true),
        "context_too_long" => (RunErrorKind::ContextTooLong, false),
        "interrupted" => (RunErrorKind::Interrupted, true),
        "storage" => (
            RunErrorKind::Unknown {
                source: UnknownSource::Storage,
            },
            false,
        ),
        "unknown" | "provider" => (RunErrorKind::Unknown { source }, false),
        _ => (RunErrorKind::Unknown { source }, false),
    };
    RunOutcome::Failed {
        kind,
        message: message.into(),
        http_status,
        retry_after_ms,
        retryable,
    }
}

fn fail(app: &mut AppState, id: &str, reason: &str) {
    feed(app, [finished(id, outcome(reason))]);
}

fn failed_app(reason: &str) -> AppState {
    let mut app = app_with_session();
    app.set_now_ms(1_000);
    send(&mut app, "hi");
    start(&mut app, "r1");
    app.set_now_ms(5_000);
    fail(&mut app, "r1", reason);
    app.take_outbox();
    app
}

fn card(app: &AppState) -> ErrorCardVm {
    app.timeline_rows()
        .into_iter()
        .find_map(|row| match row {
            TimelineRowVm::Error(card) => Some(card),
            _ => None,
        })
        .expect("error card")
}

#[test]
fn nine_cards_with_spec_actions() {
    use ErrorAction::*;
    let cases = [
        ("network", ErrorCardKind::Network, (Retry, Details)),
        (
            "rate_limited retry_after_ms=30000",
            ErrorCardKind::RateLimited,
            (Retry, Details),
        ),
        (
            "rate_limited retry_after_ms=120000",
            ErrorCardKind::RateLimitedWait,
            (Retry, Details),
        ),
        (
            "auth http_status=401",
            ErrorCardKind::Auth,
            (OpenSettings, Retry),
        ),
        ("quota", ErrorCardKind::Quota, (OpenSettings, Details)),
        (
            "server http_status=503",
            ErrorCardKind::Server,
            (Retry, Details),
        ),
        (
            "context_too_long",
            ErrorCardKind::ContextTooLong,
            (NewSession, Retry),
        ),
        (
            "interrupted",
            ErrorCardKind::Interrupted,
            (Retry, ViewChanges),
        ),
        (
            "something odd happened",
            ErrorCardKind::Unknown,
            (Retry, Details),
        ),
    ];
    let mut seen = Vec::new();
    for (reason, kind, actions) in cases {
        let card = card(&failed_app(reason));
        assert_eq!(card.kind, kind, "{reason}");
        assert_eq!((card.primary, card.secondary), actions, "{reason}");
        assert!(
            !card.title.is_empty() && !card.title.starts_with("error."),
            "{reason}"
        );
        assert!(
            !card.body.contains('{'),
            "unfilled placeholder: {}",
            card.body
        );
        assert_eq!(card.a11y_label, format!("运行失败：{}", card.title));
        seen.push(kind);
    }
    assert_eq!(seen, ErrorCardKind::ALL.to_vec());
}

#[test]
fn structured_outcome_maps_to_failure_vm() {
    let f = failure_of(&outcome(
        "rate_limited http_status=429 retry_after_ms=120000: slow down",
    ))
    .unwrap();
    assert_eq!(f.category, ErrorKind::RateLimited);
    assert_eq!(f.http_status, Some(429));
    assert_eq!(f.retry_after_ms, Some(120_000));
    assert!(f.retryable);
    assert_eq!(f.raw_message, "slow down");
    assert_eq!(f.source, None);
    let f = failure_of(&outcome("unknown source=storage: disk full")).unwrap();
    assert_eq!(f.category, ErrorKind::Unknown);
    assert_eq!(f.source, Some(UnknownSource::Storage));
    assert_eq!(failure_of(&RunOutcome::Succeeded), None);
    assert_eq!(failure_of(&cancelled()), None);
    assert!(is_interrupted(&interrupted()));
    assert!(!is_interrupted(&outcome("network")));
}

#[test]
fn countdown_follows_the_clock_and_gates_retry() {
    let mut app = failed_app("rate_limited http_status=429 retry_after_ms=100000: wait");
    let c = card(&app);
    assert_eq!(
        c.retry,
        RetryButtonState::Waiting {
            remaining_ms: 100_000
        }
    );
    assert_eq!(c.retry_label, "重试（1 分 40 秒）");
    assert_eq!(c.details, vec!["HTTP 429", "错误码 rate_limited", "wait"]);
    // Waiting: the button does nothing.
    assert_eq!(app.retry_run(&"r1".into()), None);
    assert_eq!(
        app.error_action(&"r1".into(), ErrorAction::Retry),
        ErrorEffect::None
    );
    assert!(app.take_outbox().is_empty());

    app.set_now_ms(5_000 + 60_500);
    assert_eq!(card(&app).retry_label, "重试（40 秒）");
    app.set_now_ms(5_000 + 99_001);
    assert_eq!(card(&app).retry_label, "重试（1 秒）");
    app.take_announcements();
    app.set_now_ms(5_000 + 100_000);
    let c = card(&app);
    assert_eq!(c.retry, RetryButtonState::Ready);
    assert_eq!(c.retry_label, "重试");
    // D2: no empty template once the wait is over; announced once, politely.
    assert_eq!(c.body, "现在可以重试了。");
    assert!(!c.body.contains("（还要"));
    let spoken = app.take_announcements();
    assert_eq!(
        spoken
            .iter()
            .filter(|a| a.text == "现在可以重试")
            .map(|a| a.priority)
            .collect::<Vec<_>>(),
        vec![AnnouncementPriority::Low]
    );
    app.set_now_ms(5_000 + 130_000);
    assert!(
        app.take_announcements()
            .iter()
            .all(|a| a.text != "现在可以重试")
    );
    assert_eq!(card(&app).body, "现在可以重试了。");

    let token = app.retry_run(&"r1".into()).expect("retry allowed");
    let out = single_command(&mut app);
    assert_eq!(out.token, token);
    assert_eq!(out.command, Command::RetryRun { run: "r1".into() });
    // Double click: still in flight, no second command.
    assert_eq!(app.retry_run(&"r1".into()), None);
    assert!(app.take_outbox().is_empty());
    // Core rejects (P3 not implemented) → can retry again.
    app.apply_result(token, rejected(RejectCode::NotImplemented));
    assert!(app.retry_run(&"r1".into()).is_some());
}

#[test]
fn retry_blocked_while_another_run_is_active() {
    let mut app = failed_app("network");
    send(&mut app, "again");
    start(&mut app, "r2");
    assert_eq!(app.retry_run(&"r1".into()), None);
    assert_eq!(app.retry_run(&"r2".into()), None);
}

#[test]
fn details_toggle_and_effects() {
    let mut app =
        failed_app("auth http_status=401: Authorization: Bearer sk-abcdefghijklmnop0123456789");
    assert!(!card(&app).details_open);
    app.error_action(&"r1".into(), ErrorAction::Details);
    let c = card(&app);
    assert!(c.details_open);
    assert!(
        c.details.iter().all(|d| !d.contains("sk-abc")),
        "{:?}",
        c.details
    );
    app.error_action(&"r1".into(), ErrorAction::Details);
    assert!(!card(&app).details_open);
    assert_eq!(
        app.error_action(&"r1".into(), ErrorAction::OpenSettings),
        ErrorEffect::OpenSettings
    );
    assert_eq!(
        app.error_action(&"r1".into(), ErrorAction::ViewChanges),
        ErrorEffect::OpenInspectorChanges("r1".into())
    );
}

#[test]
fn redaction_masks_keys_but_keeps_text() {
    let raw = "bad key sk-proj-AAAA1111 api_key=xyz Bearer abc.def end";
    let out = redact_secrets(raw);
    for secret in ["sk-proj", "xyz", "abc.def"] {
        assert!(!out.contains(secret), "{secret} leaked: {out}");
    }
    assert!(
        out.starts_with("bad key ") && out.ends_with(" end"),
        "{out}"
    );
}

#[test]
fn error_card_precedes_divider() {
    let app = failed_app("server");
    let rows = app.timeline_rows();
    let error = rows
        .iter()
        .position(|r| matches!(r, TimelineRowVm::Error(_)))
        .unwrap();
    let divider = rows
        .iter()
        .position(|r| matches!(r, TimelineRowVm::Divider { .. }))
        .unwrap();
    assert_eq!(divider, error + 1);
    let TimelineRowVm::Divider {
        icon,
        text,
        a11y_label,
        ..
    } = &rows[divider]
    else {
        unreachable!()
    };
    assert_eq!(*icon, DividerIcon::Failed);
    assert!(text.starts_with("失败 · 用时 4 秒"), "{text}");
    assert!(a11y_label.starts_with("运行失败"), "{a11y_label}");
}

#[test]
fn rows_markdown_reasoning_divider_usage() {
    let mut app = app_with_session();
    app.set_now_ms(1_000);
    send(&mut app, "解释");
    start(&mut app, "r1");
    app.set_now_ms(2_000);
    feed(
        &mut app,
        [Event::ReasoningDelta {
            run: "r1".into(),
            text: "想一想".into(),
        }],
    );
    app.set_now_ms(9_000);
    feed(
        &mut app,
        [Event::TextDelta {
            run: "r1".into(),
            text: "**粗体**".into(),
        }],
    );
    let rows = app.timeline_rows();
    assert!(
        matches!(rows.last(), Some(TimelineRowVm::Running { .. })),
        "{rows:?}"
    );
    feed(
        &mut app,
        [Event::UsageRecorded {
            record: UsageRecord {
                id: UsageRecordId::from("u1"),
                run_id: "r1".into(),
                provider: "openai".into(),
                model: "gpt-5".into(),
                usage: Usage {
                    input_tokens: 12_340,
                    output_tokens: 860,
                    cached_tokens: 0,
                    requests: 1,
                    estimated_cost_usd: Some(0.0213),
                },
            },
        }],
    );
    app.set_now_ms(48_000);
    feed(
        &mut app,
        [Event::RunFinished {
            run: "r1".into(),
            outcome: RunOutcome::Succeeded,
        }],
    );
    let rows = app.timeline_rows();
    assert_eq!(rows.len(), 3, "{rows:?}");
    let TimelineRowVm::Assistant {
        text,
        reasoning,
        streaming,
        ..
    } = &rows[1]
    else {
        panic!("{rows:?}")
    };
    assert_eq!(text, "**粗体**");
    assert!(!streaming);
    let reasoning = reasoning.as_ref().unwrap();
    assert_eq!(reasoning.label, "思考了 7 秒");
    assert!(!reasoning.expanded);
    let TimelineRowVm::Divider { text, icon, .. } = &rows[2] else {
        panic!()
    };
    assert_eq!(*icon, DividerIcon::Success);
    assert_eq!(
        text,
        "完成 · 用时 47 秒 · 输入 12.3k / 输出 860 tokens · 约 $0.02"
    );

    assert!(app.toggle_reasoning(&"r1".into()));
    let TimelineRowVm::Assistant { reasoning, .. } = &app.timeline_rows()[1] else {
        panic!()
    };
    assert!(reasoning.as_ref().unwrap().expanded);
    assert!(!app.toggle_reasoning(&"r1".into()));
}

#[test]
fn retry_marker_replaces_cleared_text() {
    let mut app = app_with_session();
    app.set_now_ms(1_000);
    send(&mut app, "hi");
    start(&mut app, "r1");
    feed(
        &mut app,
        [Event::ProviderRetrying {
            run: "r1".into(),
            n: 2,
            delay_ms: 4_000,
            reason: "network".into(),
        }],
    );
    let rows = app.timeline_rows();
    let marker = rows
        .iter()
        .find_map(|r| match r {
            TimelineRowVm::Retry { marker, .. } => Some(marker.clone()),
            _ => None,
        })
        .expect("retry marker");
    assert_eq!(
        (marker.attempt, marker.max, marker.remaining_ms),
        (2, 5, 4_000)
    );
    app.set_now_ms(3_500);
    let rows = app.timeline_rows();
    let remaining = rows.iter().find_map(|r| match r {
        TimelineRowVm::Retry { marker, .. } => Some(marker.remaining_ms),
        _ => None,
    });
    assert_eq!(remaining, Some(1_500));
}

#[test]
fn five_retries_announce_twice() {
    let mut app = app_with_session();
    app.set_now_ms(1_000);
    send(&mut app, "hi");
    start(&mut app, "r1");
    app.take_announcements();
    let mut spoken = Vec::new();
    for n in 1..=5u32 {
        app.set_now_ms(1_000 + u64::from(n) * 5_000);
        feed(
            &mut app,
            [Event::ProviderRetrying {
                run: "r1".into(),
                n,
                delay_ms: 1_000,
                reason: "network".into(),
            }],
        );
        spoken.extend(app.take_announcements().into_iter().map(|a| a.text));
    }
    assert_eq!(
        spoken,
        vec!["连接中断，正在第 1 次重试", "连接中断，正在第 5 次重试"]
    );
}

#[test]
fn same_kind_within_two_seconds_merges() {
    let mut app = app();
    feed(
        &mut app,
        [
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
            Event::SessionCreated {
                session: session("s2"),
            },
        ],
    );
    app.take_announcements();
    let finish = |app: &mut AppState, run_id: &str, s: &str, at: u64, outcome: RunOutcome| {
        app.set_now_ms(at);
        feed(
            app,
            [Event::RunStarted {
                run: run(run_id, s, None),
                input_id: "in-1".into(),
            }],
        );
        feed(app, [finished(run_id, outcome)]);
    };
    finish(&mut app, "a", "s1", 10_000, RunOutcome::Succeeded);
    let first = app.take_announcements();
    let finished: Vec<_> = first
        .iter()
        .filter(|a| a.kind == AnnouncementKind::RunFinished)
        .collect();
    assert_eq!(finished.len(), 1);
    // Two more finishes inside the window: held, merged into one (latest text,
    // highest priority).
    finish(&mut app, "b", "s2", 10_500, failed(RunErrorKind::Network));
    assert!(
        app.take_announcements()
            .iter()
            .all(|a| a.kind != AnnouncementKind::RunFinished)
    );
    finish(&mut app, "c", "s1", 11_000, cancelled());
    assert!(
        app.take_announcements()
            .iter()
            .all(|a| a.kind != AnnouncementKind::RunFinished)
    );
    app.set_now_ms(12_000);
    let released: Vec<_> = app
        .take_announcements()
        .into_iter()
        .filter(|a| a.kind == AnnouncementKind::RunFinished)
        .collect();
    assert_eq!(released.len(), 1, "{released:?}");
    assert_eq!(released[0].text, "已停止");
    assert_eq!(released[0].priority, AnnouncementPriority::High);
}

#[test]
fn layout_rules() {
    let mut ui = UiPrefs::default();
    assert_eq!(ui.sidebar_width, SIDEBAR_DEFAULT);
    assert!(!ui.inspector_visible);
    let l = ui.layout(DEFAULT_WINDOW_WIDTH);
    assert_eq!(l.sidebar, PanelLayout::Docked { width: 240.0 });
    assert_eq!(l.inspector, PanelLayout::Hidden);
    assert_eq!(l.main_width, 1040.0);

    // Clamp, collapse, reset.
    assert_eq!(ui.drag_sidebar(500.0), SidebarResize::Resized(SIDEBAR_MAX));
    assert_eq!(ui.drag_sidebar(170.0), SidebarResize::Resized(SIDEBAR_MIN));
    assert_eq!(ui.drag_sidebar(150.0), SidebarResize::Collapsed);
    assert!(!ui.sidebar_visible);
    ui.reset_sidebar_width();
    assert_eq!(ui.sidebar_width, SIDEBAR_DEFAULT);
    ui.sidebar_visible = true;
    ui.drag_inspector(9_000.0);
    assert_eq!(ui.inspector_width, INSPECTOR_MAX);
    ui.drag_inspector(0.0);
    assert_eq!(ui.inspector_width, INSPECTOR_MIN);
    ui.reset_inspector_width();
    assert_eq!(ui.inspector_width, INSPECTOR_DEFAULT);

    // Three columns fit at 240 + 480 + 360 = 1080.
    ui.inspector_visible = true;
    let l = ui.layout(1_080.0);
    assert_eq!(l.sidebar, PanelLayout::Docked { width: 240.0 });
    assert_eq!(l.inspector, PanelLayout::Docked { width: 360.0 });
    assert_eq!(l.main_width, 480.0);
    // Narrower: Inspector becomes an overlay, main keeps the rest.
    let l = ui.layout(1_000.0);
    assert_eq!(l.inspector, PanelLayout::Overlay { width: 360.0 });
    assert_eq!(l.sidebar, PanelLayout::Docked { width: 240.0 });
    assert_eq!(l.main_width, 760.0);
    // Below sidebar + 480 the sidebar collapses too.
    let l = ui.layout(700.0);
    assert_eq!(l.sidebar, PanelLayout::Hidden);
    assert_eq!(l.main_width, 700.0);
}

#[test]
fn short_retry_after_is_retryable_immediately() {
    let mut app = failed_app("rate_limited http_status=429 retry_after_ms=30000: slow");
    let c = card(&app);
    assert_eq!(c.kind, ErrorCardKind::RateLimited);
    assert_eq!(c.retry, RetryButtonState::Ready);
    assert_eq!(c.retry_label, "重试");
    assert_eq!(c.primary_shortcut, Some("⌘R"));
    assert_eq!(c.body, "服务要求稍后再试。");
    assert!(
        app.retry_run(&"r1".into()).is_some(),
        "no countdown at <= 60 s"
    );
}

#[test]
fn retry_after_boundary_is_60000() {
    // ≤ 60000 → retry now (⌘R); 60001 → wait card.
    let c = card(&failed_app(
        "rate_limited http_status=429 retry_after_ms=60000: slow",
    ));
    assert_eq!(c.kind, ErrorCardKind::RateLimited);
    assert_eq!(c.retry, RetryButtonState::Ready);
    assert_eq!(c.primary_shortcut, Some("⌘R"));
    let c = card(&failed_app(
        "rate_limited http_status=429 retry_after_ms=60001: slow",
    ));
    assert_eq!(c.kind, ErrorCardKind::RateLimitedWait);
    assert_eq!(
        c.retry,
        RetryButtonState::Waiting {
            remaining_ms: 60_001
        }
    );
}

#[test]
fn token_header_values_never_reach_details() {
    for raw in [
        "{'Authorization': 'Bearer qqq'}",
        "Authorization=Bearer tok_123",
        r#"{"x-api-key": "abc"}"#,
        "Proxy-Authorization: Basic dXNlcg==",
        "token: abc",
        "Cookie: a=b",
    ] {
        let c = card(&failed_app(&format!("auth http_status=401: denied\n{raw}")));
        let all = c.details.join("\n");
        for leaked in ["qqq", "tok_123", "abc", "dXNlcg", "a=b"] {
            assert!(!all.contains(leaked), "{leaked} in {all}");
        }
        assert!(all.contains("denied\n"), "{all}");
        assert!(all.contains("[已移除请求头]"), "{all}");
    }
    // Header-only message: the placeholder keeps the details non-empty.
    let c = card(&failed_app("auth http_status=401: Authorization: Bearer x"));
    assert_eq!(c.details.last().unwrap(), "[已移除请求头]");
    let c = card(&failed_app(
        "auth http_status=401: invalid api key: please check settings",
    ));
    assert_eq!(
        c.details.last().unwrap(),
        "invalid api key: please check settings"
    );
}

#[test]
fn long_retry_after_shows_clock_time_and_countdown() {
    let mut app = app_with_session();
    app.set_utc_offset_ms(8 * 3_600_000);
    // 2026-11-14 06:03:20 UTC = 14:03:20 local; Retry-After 100 s → 14:05.
    let t0 = 1_794_636_200_000;
    app.set_now_ms(t0 - 4_000);
    send(&mut app, "hi");
    start(&mut app, "r1");
    app.set_now_ms(t0);
    fail(
        &mut app,
        "r1",
        "rate_limited http_status=429 retry_after_ms=100000: wait",
    );
    let c = card(&app);
    assert_eq!(c.kind, ErrorCardKind::RateLimitedWait);
    assert_eq!(c.title, "服务要求等待");
    assert_eq!(c.body, "可在 14:05 后重试（还要 1 分 40 秒）。");
    assert_eq!(c.retry_label, "重试（1 分 40 秒）");
    // V1 meta: source · time the failure arrived; status pill text.
    assert_eq!(c.status, "失败");
    assert!(c.meta.ends_with(" · 14:03"), "{}", c.meta);
    // Fake clock +1 s: remaining time ticks, the wall-clock time stays fixed.
    app.set_now_ms(t0 + 1_000);
    let c = card(&app);
    assert_eq!(c.body, "可在 14:05 后重试（还要 1 分 39 秒）。");
    app.set_now_ms(t0 + 41_000);
    assert_eq!(card(&app).body, "可在 14:05 后重试（还要 59 秒）。");
}

#[test]
fn details_replace_credential_headers_in_place() {
    let app = failed_app(
        "auth http_status=401: invalid key\nAuthorization: Bearer sk-live-SENTINEL\nx-api-key: SENTINEL2\nrequest id req_1",
    );
    let c = card(&app);
    let all = c.details.join("\n");
    for leaked in ["Authorization", "Bearer", "SENTINEL", "x-api-key"] {
        assert!(!all.contains(leaked), "{leaked} in {all}");
    }
    assert_eq!(
        c.details.last().unwrap(),
        "invalid key\n[已移除请求头]\n[已移除请求头]\nrequest id req_1"
    );
}

#[test]
fn cmd_r_retries_latest_failed_run_only() {
    let mut app = failed_app("network");
    let token = app.retry_latest().expect("latest run failed");
    assert_eq!(
        single_command(&mut app).command,
        Command::RetryRun { run: "r1".into() }
    );
    app.apply_result(token, accepted());
    // A later successful run: ⌘R does nothing and the old card loses its hint.
    send(&mut app, "again");
    start(&mut app, "r2");
    feed(
        &mut app,
        [Event::RunFinished {
            run: "r2".into(),
            outcome: RunOutcome::Succeeded,
        }],
    );
    app.take_outbox();
    assert_eq!(app.retry_latest(), None);
    assert!(app.take_outbox().is_empty());
    // Two failures: only the newest card keeps "⌘R".
    send(&mut app, "third");
    start(&mut app, "r3");
    fail(&mut app, "r3", "server");
    let shortcuts: Vec<_> = app
        .timeline_rows()
        .into_iter()
        .filter_map(|r| match r {
            TimelineRowVm::Error(c) => Some(c.primary_shortcut),
            _ => None,
        })
        .collect();
    assert_eq!(shortcuts, vec![None, Some("⌘R")]);
}

#[test]
fn unknown_source_picks_title_storage_vs_provider() {
    // Core already maps stream/format errors to `server` (PROTOCOL.md table);
    // the remaining provider errors arrive as unknown{provider} → 运行失败.
    let c = card(&failed_app("server: stream ended early"));
    assert_eq!(c.kind, ErrorCardKind::Server);
    assert_eq!(c.title, "模型服务出错");
    assert_eq!(
        (c.primary, c.primary_shortcut),
        (ErrorAction::Retry, Some("⌘R"))
    );
    let c = card(&failed_app("unknown source=provider: odd"));
    assert_eq!(
        (c.kind, c.title.as_str()),
        (ErrorCardKind::Unknown, "运行失败")
    );
    assert!(
        c.details.iter().any(|d| d.contains("unknown/provider")),
        "{:?}",
        c.details
    );

    let mut app = failed_app("storage: disk full");
    let c = card(&app);
    assert_eq!(c.kind, ErrorCardKind::Unknown);
    assert_eq!(c.title, "保存失败");
    assert_eq!(c.body, "运行结果没能写入本地数据，详情里有原因。");
    assert_eq!(c.a11y_label, "运行失败：保存失败");
    assert!(
        app.take_announcements()
            .iter()
            .any(|a| a.text == "运行失败：保存失败"),
        "announcement uses the overridden title"
    );

    // P1 shape: Unknown with an explicit source field.
    let c = card(&failed_app("unknown source=storage: write failed"));
    assert_eq!(
        (c.kind, c.title.as_str()),
        (ErrorCardKind::Unknown, "保存失败")
    );
    // Plain Unknown keeps the default title.
    assert_eq!(card(&failed_app("weird")).title, "运行失败");
}
