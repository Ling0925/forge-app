//! Sidebar groups and session intents (§6.1).
mod common;
use common::*;
use forge_protocol::*;
use forge_state::*;

fn ids(rows: &[SessionRowVm]) -> Vec<String> {
    rows.iter().map(|r| r.id.0.clone()).collect()
}

fn tool_and_approval(run: &str, call: &str, approval: &str) -> [Event; 2] {
    [
        Event::ToolCallProposed {
            call: ToolCall {
                id: call.into(),
                run_id: run.into(),
                tool: "write_file".into(),
                arguments_json: "{}".into(),
                state: ToolCallState::Proposed,
                result: None,
                has_side_effects: true,
            },
        },
        Event::ApprovalRequested {
            approval: Approval {
                id: approval.into(),
                tool_call_id: call.into(),
                requested_at_unix_ms: 0,
                state: ApprovalState::Pending,
                decision: None,
                resolved_by: None,
                scope: ApprovalScope::Once,
            },
        },
    ]
}

#[test]
fn ui_sb_01_approval_moves_session_to_needs_attention_and_back() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    assert_eq!(ids(&app.sidebar().in_progress), ["s1"]);
    feed(&mut app, tool_and_approval("r1", "c1", "a1"));
    let sidebar = app.sidebar();
    assert_eq!(ids(&sidebar.needs_attention), ["s1"]);
    let row = &sidebar.needs_attention[0];
    assert_eq!(row.approvals, 1);
    assert_eq!(row.icon, StatusIcon::Approval);
    assert_eq!(row.status_text, "等待批准 · write_file");
    assert_eq!(row.a11y_label, "会话 s1，等待批准 · write_file，1 个请求");
    assert_eq!(
        app.selected().unwrap().running().unwrap().phase,
        RunPhase::WaitingApproval
    );
    let approval_announced = app
        .take_announcements()
        .iter()
        .any(|a| a.text == "需要批准：write_file。按 Command J 查看");
    assert!(approval_announced);
    feed(
        &mut app,
        [Event::ApprovalResolved {
            approval: "a1".into(),
            decision: ApprovalDecision::Allow,
            scope: ApprovalScope::Once,
            by: ApprovalResolvedBy::User,
        }],
    );
    let sidebar = app.sidebar();
    assert!(sidebar.needs_attention.is_empty());
    assert_eq!(ids(&sidebar.in_progress), ["s1"]);
}

#[test]
fn ui_sb_02_cancel_with_pending_queue_needs_attention() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            Event::QueueItemAdded {
                item: item("q2", "s1", "in-x", "next", QueueItemState::Pending, 2),
            },
        ],
    );
    assert_eq!(ids(&app.sidebar().in_progress), ["s1"]);
    feed(
        &mut app,
        [Event::RunFinished {
            run: "r1".into(),
            outcome: cancelled(),
        }],
    );
    let sidebar = app.sidebar();
    assert_eq!(ids(&sidebar.needs_attention), ["s1"]);
    assert_eq!(
        sidebar.needs_attention[0].queue_paused,
        Some(QueuePauseReason::Cancelled)
    );
    // Core's QueuePaused / QueueResumed stay authoritative.
    feed(
        &mut app,
        [Event::QueueResumed {
            session: "s1".into(),
        }],
    );
    assert_eq!(ids(&app.sidebar().recent), ["s1"]);
}

#[test]
fn row_status_texts_cover_each_state() {
    let mut app = app_with_session();
    app.set_now_ms(10_000);
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    app.set_now_ms(93_000);
    let row = app.sidebar().in_progress[0].clone();
    assert_eq!(row.status_text, "运行中 · 1:23");
    assert_eq!(row.icon, StatusIcon::Spinner);
    assert_eq!(row.a11y_label, "会话 s1，运行中 · 1:23");
    feed(
        &mut app,
        [Event::RunStateChanged {
            run: "r1".into(),
            state: RunState::Cancelling,
        }],
    );
    assert_eq!(app.sidebar().in_progress[0].status_text, "正在停止…");
    feed(
        &mut app,
        [Event::RunFinished {
            run: "r1".into(),
            outcome: failed(RunErrorKind::Network),
        }],
    );
    let row = app.sidebar().recent[0].clone();
    assert_eq!(
        (row.icon, row.status_text.as_str()),
        (StatusIcon::Failed, "失败 · 网络错误")
    );

    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r2", "s1", None),
                input_id: "in-1".into(),
            },
            Event::RunFinished {
                run: "r2".into(),
                outcome: interrupted(),
            },
        ],
    );
    let row = app.sidebar().recent[0].clone();
    assert_eq!(
        (row.icon, row.status_text.as_str()),
        (StatusIcon::Interrupted, "异常退出时中断")
    );

    feed(
        &mut app,
        [Event::QueueItemUpdated {
            item: item("q", "s1", "i", "t", QueueItemState::Uncertain, 1),
        }],
    );
    let row = app.sidebar().needs_attention[0].clone();
    assert_eq!(
        (row.icon, row.status_text.as_str()),
        (StatusIcon::Uncertain, "1 条消息状态不确定")
    );
}

#[test]
fn failed_with_paused_queue_shows_paused_text() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            Event::QueueItemAdded {
                item: item("q2", "s1", "x", "n", QueueItemState::Pending, 2),
            },
            Event::RunFinished {
                run: "r1".into(),
                outcome: failed(RunErrorKind::Auth),
            },
            Event::QueuePaused {
                session: "s1".into(),
                reason: QueuePauseReason::Failed,
            },
        ],
    );
    let row = app.sidebar().needs_attention[0].clone();
    assert_eq!(row.status_text, "失败 · 认证错误 · 队列已暂停");
}

#[test]
fn recent_is_ordered_by_activity_and_filtered_by_workspace() {
    let mut app = app_with_session();
    let mut other = session("s3");
    other.workspace_id = "other".into();
    feed(
        &mut app,
        [
            Event::SessionCreated {
                session: session_at("s2", 1_000),
            },
            Event::SessionCreated { session: other },
        ],
    );
    assert_eq!(ids(&app.sidebar().recent), ["s2", "s1"]);
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    feed(
        &mut app,
        [Event::RunFinished {
            run: "r1".into(),
            outcome: RunOutcome::Succeeded,
        }],
    );
    // Core bumps `updated_at` with the run and publishes SessionUpdated.
    feed(
        &mut app,
        [Event::SessionUpdated {
            session: session_at("s1", 5_000),
        }],
    );
    assert_eq!(ids(&app.sidebar().recent), ["s1", "s2"]);
}

#[test]
fn archived_sessions_are_counted_and_hidden_until_shown() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::SessionCreated {
                session: session("s2"),
            },
            Event::SessionArchived {
                session: "s2".into(),
                archived: true,
            },
        ],
    );
    let sidebar = app.sidebar();
    assert_eq!(sidebar.archived_count, 1);
    assert!(sidebar.archived.is_empty());
    assert_eq!(ids(&sidebar.recent), ["s1"]);
    app.set_show_archived(true);
    assert_eq!(ids(&app.sidebar().archived), ["s2"]);
}

#[test]
fn empty_sidebar_and_group_labels() {
    let app = app();
    assert!(app.sidebar().is_empty());
    assert_eq!(
        SidebarVm::group_label(SidebarGroup::NeedsAttention, 2),
        "需要处理，2 个会话"
    );
}

#[test]
fn ui_sb_05_running_session_cannot_be_deleted() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    assert!(!app.can_delete(&"s1".into()));
    assert_eq!(app.delete_session(&"s1".into()), None);
    feed(
        &mut app,
        [Event::RunFinished {
            run: "r1".into(),
            outcome: RunOutcome::Succeeded,
        }],
    );
    assert!(app.delete_session(&"s1".into()).is_some());
    assert_eq!(
        single_command(&mut app).command,
        Command::DeleteSession {
            session: "s1".into()
        }
    );
}

#[test]
fn opening_a_session_loads_history_once_and_retries_on_error() {
    let mut app = app();
    feed(
        &mut app,
        [Event::SessionUpdated {
            session: session("old"),
        }],
    );
    let id: SessionId = "old".into();
    app.select_session(&id);
    let out = single_command(&mut app);
    assert_eq!(
        out.command,
        Command::LoadHistory {
            session: id.clone(),
            before: None,
            limit: HISTORY_PAGE,
        }
    );
    assert_eq!(app.sessions[&id].history, HistoryState::Loading);
    app.select_session(&id);
    assert!(app.take_outbox().is_empty());
    app.apply_result(out.token, rejected(RejectCode::Storage));
    assert_eq!(app.sessions[&id].history, HistoryState::Error);
    app.retry_history(&id);
    let retry = single_command(&mut app);
    app.apply_result(retry.token, accepted());
    // Accepted only means Core will answer; the page arrives as HistoryLoaded.
    assert_eq!(app.sessions[&id].history, HistoryState::Loading);
    feed(
        &mut app,
        [Event::HistoryLoaded {
            session: id.clone(),
            messages: vec![],
            next_before: None,
        }],
    );
    assert_eq!(app.sessions[&id].history, HistoryState::Complete);
}

#[test]
fn deleting_selected_session_selects_neighbour() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::SessionCreated {
            session: session("s2"),
        }],
    );
    app.select_session(&"s2".into());
    feed(
        &mut app,
        [Event::SessionDeleted {
            session: "s2".into(),
        }],
    );
    assert_eq!(app.selected_session, Some("s1".into()));
    feed(
        &mut app,
        [Event::SessionDeleted {
            session: "s1".into(),
        }],
    );
    assert_eq!(app.selected_session, None);
}

#[test]
fn adjacent_navigation_follows_sidebar_order() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::SessionCreated {
                session: session_at("s2", 1_000),
            },
            Event::SessionCreated {
                session: session_at("s3", 2_000),
            },
        ],
    );
    // Order: s3, s2, s1 (newest activity first).
    app.select_session(&"s3".into());
    app.select_adjacent(true);
    assert_eq!(app.selected_session, Some("s2".into()));
    app.select_adjacent(true);
    app.select_adjacent(true);
    assert_eq!(app.selected_session, Some("s1".into()));
    app.select_adjacent(false);
    assert_eq!(app.selected_session, Some("s2".into()));
}

#[test]
fn create_session_selects_it_when_created() {
    let mut app = app_with_session();
    let token = app.create_session().unwrap();
    let out = single_command(&mut app);
    assert_eq!(out.token, token);
    assert_eq!(
        out.command,
        Command::CreateSession {
            workspace: "ws".into(),
            title: "新会话".into(),
            model: settings(true).model,
            permission_mode: PermissionMode::Ask,
        }
    );
    app.apply_result(token, accepted());
    feed(
        &mut app,
        [Event::SessionCreated {
            session: session("new"),
        }],
    );
    assert_eq!(app.selected_session, Some("new".into()));
    assert_eq!(app.selected().unwrap().history, HistoryState::Complete);
}

#[test]
fn create_session_needs_workspace() {
    let mut app = app();
    assert_eq!(app.create_session(), None);
}

#[test]
fn rename_rejects_empty_and_reverts_on_reject() {
    let mut app = app_with_session();
    let id: SessionId = "s1".into();
    assert_eq!(app.rename_session(&id, "   "), None);
    let token = app.rename_session(&id, " 修复登录 ").unwrap();
    assert_eq!(app.sessions[&id].title(), "修复登录");
    assert_eq!(
        single_command(&mut app).command,
        Command::RenameSession {
            session: id.clone(),
            title: "修复登录".into()
        }
    );
    app.apply_result(token, rejected(RejectCode::Storage));
    assert_eq!(app.sessions[&id].title(), "会话 s1");
    assert_eq!(app.take_toasts().len(), 1);
}

#[test]
fn archive_sends_command_and_moves_selection() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::SessionCreated {
            session: session("s2"),
        }],
    );
    app.select_session(&"s1".into());
    app.archive_session(&"s1".into(), true).unwrap();
    assert_eq!(
        single_command(&mut app).command,
        Command::ArchiveSession {
            session: "s1".into(),
            archived: true
        }
    );
    feed(
        &mut app,
        [Event::SessionArchived {
            session: "s1".into(),
            archived: true,
        }],
    );
    assert_eq!(app.selected_session, Some("s2".into()));
}

#[test]
fn sidebar_toggle() {
    let mut app = app();
    assert!(app.sidebar().visible);
    app.toggle_sidebar();
    assert!(!app.sidebar().visible);
}

#[test]
fn workspace_events_track_active_workspace() {
    let mut app = app();
    let ws = |id: &str| Workspace {
        id: id.into(),
        root: format!("/tmp/{id}").into(),
        name: id.into(),
    };
    feed(
        &mut app,
        [
            Event::WorkspaceAdded { workspace: ws("a") },
            Event::WorkspaceAdded { workspace: ws("b") },
        ],
    );
    assert_eq!(app.active_workspace, Some("a".into()));
    feed(
        &mut app,
        [Event::WorkspaceRemoved {
            workspace: "a".into(),
        }],
    );
    assert_eq!(app.active_workspace, Some("b".into()));
    assert_eq!(app.workspaces.len(), 1);
}

#[test]
fn idle_rows_show_relative_time_and_split_by_local_day() {
    const H: u64 = 3_600_000;
    // 2026-11-14 10:00 UTC+8 = 02:00 UTC.
    let now = 1_794_621_600_000;
    let mut app = app();
    app.set_utc_offset_ms(8 * H as i64);
    feed(
        &mut app,
        [Event::WorkspaceAdded {
            workspace: Workspace {
                id: "ws".into(),
                root: "/tmp/ws".into(),
                name: "ws".into(),
            },
        }],
    );
    // s-old: 3 days ago; s-yday: 23:30 local yesterday; s-today: 2 h ago.
    for (id, at) in [
        ("s-old", now - 72 * H),
        ("s-yday", now - 10 * H - H / 2),
        ("s-today", now - 2 * H),
    ] {
        app.set_now_ms(at);
        feed(
            &mut app,
            [Event::SessionCreated {
                session: session_at(id, at),
            }],
        );
    }
    app.set_now_ms(now);
    let sidebar = app.sidebar();
    let buckets: Vec<_> = sidebar
        .recent_buckets()
        .into_iter()
        .map(|(b, rows)| (b, ids(rows)))
        .collect();
    assert_eq!(
        buckets,
        [
            (DayBucket::Today, vec!["s-today".to_owned()]),
            (DayBucket::Yesterday, vec!["s-yday".to_owned()]),
            (DayBucket::Week, vec!["s-old".to_owned()]),
        ]
    );
    let today = &sidebar.recent[0];
    assert_eq!(today.status_text, "2 小时前");
    assert_eq!(today.a11y_label, "会话 s-today，2 小时前");
    assert_eq!(sidebar.recent[2].status_text, "3 天前");
    assert_eq!(
        SidebarVm::bucket_label(DayBucket::Today, 1),
        "今天，1 个会话"
    );
    assert_eq!(relative_time(now - 30_000, now), "刚刚");
    assert_eq!(relative_time(now - 5 * 60_000, now), "5 分钟前");
    // More than a week ago → 更早.
    assert_eq!(
        DayBucket::for_time(now - 8 * 86_400_000, now, 0),
        DayBucket::Older
    );
}

#[test]
fn ui_ref_03_attention_count_change_is_announced() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    app.take_announcements();
    feed(&mut app, tool_and_approval("r1", "c1", "a1"));
    let sidebar = app.sidebar();
    assert_eq!(sidebar.needs_attention.len(), 1);
    let said: Vec<String> = app
        .take_announcements()
        .into_iter()
        .map(|a| a.text)
        .collect();
    assert!(said.iter().any(|t| t == "需要处理，1 个会话"), "{said:?}");
    // Unchanged count → no repeat.
    feed(
        &mut app,
        [Event::TextDelta {
            run: "r1".into(),
            text: "x".into(),
        }],
    );
    assert!(
        app.take_announcements()
            .iter()
            .all(|a| a.kind != AnnouncementKind::SidebarCount)
    );
    app.set_now_ms(app.now_ms() + ANNOUNCE_MERGE_MS);
    feed(
        &mut app,
        [Event::ApprovalResolved {
            approval: "a1".into(),
            decision: ApprovalDecision::Allow,
            scope: ApprovalScope::Once,
            by: ApprovalResolvedBy::User,
        }],
    );
    let said: Vec<String> = app
        .take_announcements()
        .into_iter()
        .map(|a| a.text)
        .collect();
    assert!(said.iter().any(|t| t == "需要处理，0 个会话"), "{said:?}");
}
