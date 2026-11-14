//! Event → timeline reduction (§5.2, §6.3, §6.4).
mod common;
use common::*;
use forge_protocol::*;
use forge_state::*;

fn send(app: &mut AppState, text: &str) -> Outgoing {
    app.set_draft(text);
    app.composer_key(ComposerKey::Enter);
    single_command(app)
}

fn delta(run: &str, text: &str) -> Event {
    Event::TextDelta {
        run: run.into(),
        text: text.into(),
    }
}

#[test]
fn single_turn_flow() {
    let mut app = app_with_session();
    app.set_now_ms(1_000);
    let out = send(&mut app, "解释这个项目");
    app.apply_result(out.token, accepted());
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    let s = app.selected().unwrap();
    assert_eq!(s.running().unwrap().phase, RunPhase::Starting);
    assert_eq!(
        s.timeline,
        vec![TimelineItem::User(UserMessageVm {
            input_id: "in-1".into(),
            ordinal: None,
            text: "解释这个项目".into(),
            origin: MessageOrigin::Direct,
        })]
    );
    feed(
        &mut app,
        [delta("r1", "这是"), delta("r1", "一个 Rust 项目")],
    );
    assert_eq!(
        app.selected().unwrap().running().unwrap().phase,
        RunPhase::Streaming
    );
    app.set_now_ms(47_000);
    feed(&mut app, [finished("r1", RunOutcome::Succeeded)]);
    let s = app.selected().unwrap();
    assert_eq!(s.active_run, None);
    assert_eq!(
        s.timeline[1],
        TimelineItem::Assistant(AssistantMessageVm {
            run_id: "r1".into(),
            ordinal: None,
            text: "这是一个 Rust 项目".into(),
            reasoning: String::new(),
            streaming: false,
            stopped: false,
            reasoning_started_ms: None,
            reasoning_ms: None,
        })
    );
    assert_eq!(
        s.timeline[2],
        TimelineItem::RunDivider {
            run_id: "r1".into(),
            outcome: RunOutcome::Succeeded
        }
    );
    assert_eq!(s.last_outcome, Some(RunOutcome::Succeeded));
    assert_eq!(app.composer_mode(), ComposerMode::Idle);
    let announcements: Vec<_> = app
        .take_announcements()
        .into_iter()
        .map(|a| a.text)
        .collect();
    // Elapsed = local finish time − `Run.started_at` (fixture: 0).
    assert_eq!(announcements, vec!["开始运行", "运行完成，用时 0:46"]);
}

#[test]
fn run_started_before_accepted_inserts_user_message_once() {
    let mut app = app_with_session();
    let out = send(&mut app, "hi");
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    app.apply_result(out.token, accepted());
    let users = app
        .selected()
        .unwrap()
        .timeline
        .iter()
        .filter(|i| matches!(i, TimelineItem::User(_)))
        .count();
    assert_eq!(users, 1);
    assert_eq!(app.selected().unwrap().composer.draft, "");
}

#[test]
fn queue_item_events_supply_the_user_message() {
    let mut app = app_with_session();
    let out = send(&mut app, "hi");
    app.apply_result(out.token, accepted());
    feed(
        &mut app,
        [
            Event::QueueItemAdded {
                item: item("q1", "s1", "in-1", "hi", QueueItemState::Pending, 1),
            },
            Event::QueueItemUpdated {
                item: item("q1", "s1", "in-1", "hi", QueueItemState::Started, 1),
            },
            Event::RunStarted {
                run: run("r1", "s1", Some("q1")),
                input_id: "in-1".into(),
            },
        ],
    );
    let timeline = &app.selected().unwrap().timeline;
    assert_eq!(timeline.len(), 1);
    // Sent while idle → not tagged "来自队列".
    assert!(matches!(&timeline[0], TimelineItem::User(m) if m.origin == MessageOrigin::Direct));
}

#[test]
fn message_queued_while_busy_is_tagged_from_queue() {
    let mut app = app_with_session();
    app.capabilities = Capabilities::P3;
    feed(
        &mut app,
        [Event::RunStarted {
            run: run("r1", "s1", None),
            input_id: "in-1".into(),
        }],
    );
    let out = send(&mut app, "later");
    app.apply_result(out.token, accepted());
    feed(
        &mut app,
        [
            Event::QueueItemAdded {
                item: item("q2", "s1", "in-1", "later", QueueItemState::Pending, 2),
            },
            finished("r1", RunOutcome::Succeeded),
            Event::QueueItemUpdated {
                item: item("q2", "s1", "in-1", "later", QueueItemState::Started, 2),
            },
        ],
    );
    let user = app
        .selected()
        .unwrap()
        .timeline
        .iter()
        .find_map(|i| match i {
            TimelineItem::User(m) => Some(m.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(user.origin, MessageOrigin::Queue("q2".into()));
}

#[test]
fn ui_msg_01_generation_reset_leaves_only_final_text() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            delta("r1", "a"),
            delta("r1", "b"),
            delta("r1", "c"),
            Event::ProviderRetrying {
                run: "r1".into(),
                n: 2,
                delay_ms: 2_000,
                reason: "503".into(),
            },
        ],
    );
    assert_eq!(
        app.selected().unwrap().running().unwrap().phase,
        RunPhase::Retrying {
            attempt: 2,
            delay_ms: 2_000,
            reason: "503".into(),
            next_at_ms: app.now_ms() + 2_000,
        }
    );
    feed(
        &mut app,
        [
            Event::GenerationReset { run: "r1".into() },
            delta("r1", "final "),
            delta("r1", "answer"),
            finished("r1", RunOutcome::Succeeded),
        ],
    );
    assert_eq!(
        assistant_text(&app, "s1", "r1").as_deref(),
        Some("final answer")
    );
    let assistants = app
        .selected()
        .unwrap()
        .timeline
        .iter()
        .filter(|i| matches!(i, TimelineItem::Assistant(_)))
        .count();
    assert_eq!(assistants, 1);
}

#[test]
fn reasoning_sets_thinking_then_text_streams() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            Event::ReasoningDelta {
                run: "r1".into(),
                text: "想一想".into(),
            },
        ],
    );
    assert_eq!(
        app.selected().unwrap().running().unwrap().phase,
        RunPhase::Thinking
    );
    feed(&mut app, [delta("r1", "好")]);
    assert_eq!(
        app.selected().unwrap().running().unwrap().phase,
        RunPhase::Streaming
    );
    match &app.selected().unwrap().timeline[0] {
        TimelineItem::Assistant(m) => {
            assert_eq!(m.reasoning, "想一想");
            assert!(m.streaming);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn cancelled_run_keeps_text_and_marks_stopped() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            delta("r1", "partial"),
            Event::RunStateChanged {
                run: "r1".into(),
                state: RunState::Cancelling,
            },
            delta("r1", " more"),
        ],
    );
    assert_eq!(
        app.selected().unwrap().running().unwrap().phase,
        RunPhase::Cancelling
    );
    feed(&mut app, [finished("r1", cancelled())]);
    match &app.selected().unwrap().timeline[0] {
        TimelineItem::Assistant(m) => {
            assert_eq!(m.text, "partial more");
            assert!(m.stopped && !m.streaming);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn failed_run_adds_error_item_with_kind() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            finished("r1", failed(RunErrorKind::Auth)),
        ],
    );
    let s = app.selected().unwrap();
    let mut failure = FailureVm::new(ErrorKind::Auth);
    failure.retryable = false;
    assert!(s.timeline.contains(&TimelineItem::Error(ErrorVm {
        run_id: "r1".into(),
        kind: ErrorKind::Auth,
        failure,
        retry_at_ms: None,
    })));
    let announcements: Vec<_> = app.take_announcements();
    let last = announcements.last().unwrap();
    assert_eq!(last.text, "运行失败：API Key 无效或没有权限");
    assert_eq!(last.priority, AnnouncementPriority::High);
}

#[test]
fn terminal_state_is_applied_once() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            Event::RunStateChanged {
                run: "r1".into(),
                state: RunState::Succeeded,
            },
            finished("r1", RunOutcome::Succeeded),
            finished("r1", failed(RunErrorKind::Server)),
        ],
    );
    let s = app.selected().unwrap();
    let dividers = s
        .timeline
        .iter()
        .filter(|i| matches!(i, TimelineItem::RunDivider { .. }))
        .count();
    assert_eq!(dividers, 1);
    assert_eq!(s.last_outcome, Some(RunOutcome::Succeeded));
}

#[test]
fn ui_tb_01_context_warning_at_80_percent_and_compaction_notice() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            Event::ContextUsage {
                run: "r1".into(),
                used_tokens: 79_000,
                context_remaining_estimate: 21_000,
            },
        ],
    );
    assert!(!app.selected().unwrap().context.is_warning());
    feed(
        &mut app,
        [Event::ContextUsage {
            run: "r1".into(),
            used_tokens: 82_000,
            context_remaining_estimate: 18_000,
        }],
    );
    let ctx = app.selected().unwrap().context;
    assert_eq!(ctx.percent(), Some(82));
    assert!(ctx.is_warning());
    feed(
        &mut app,
        [Event::ContextCompacted {
            run: "r1".into(),
            before_tokens: 82_000,
            after_tokens: 30_000,
        }],
    );
    let s = app.selected().unwrap();
    assert_eq!(s.context.compacted_count, 1);
    assert!(
        s.timeline
            .iter()
            .any(|i| matches!(i, TimelineItem::Notice(n) if n.text == "已压缩较早的上下文"))
    );
}

#[test]
fn events_for_unknown_runs_are_ignored() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            delta("ghost", "x"),
            finished("ghost", failed(RunErrorKind::Network)),
        ],
    );
    assert!(app.selected().unwrap().timeline.is_empty());
}

#[test]
fn duplicate_seq_is_ignored_and_gap_flags_resync() {
    let mut app = app_with_session();
    let seq = app.last_seq;
    app.apply(EventEnvelope {
        seq,
        event: Event::SessionCreated {
            session: session("dup"),
        },
    });
    assert!(!app.sessions.contains_key(&SessionId::from("dup")));
    app.apply(EventEnvelope {
        seq: seq + 5,
        event: Event::SessionCreated {
            session: session("s9"),
        },
    });
    assert_eq!(
        app.resync,
        ResyncState::Resyncing {
            expected: seq + 1,
            got: seq + 5
        }
    );
    // Old state is kept and the event still applied; exactly one snapshot
    // request goes out.
    assert!(app.sessions.contains_key(&SessionId::from("s1")));
    assert!(app.sessions.contains_key(&SessionId::from("s9")));
    assert_eq!(single_command(&mut app).command, Command::GetSnapshot);
}

#[test]
fn notice_event_becomes_toast() {
    let mut app = app();
    feed(
        &mut app,
        [Event::Notice {
            level: NoticeLevel::Warning,
            code: "x".into(),
            message: "注意".into(),
        }],
    );
    assert_eq!(
        app.take_toasts(),
        vec![ToastVm {
            level: NoticeLevel::Warning,
            text: "注意".into()
        }]
    );
}

#[test]
fn usage_recorded_updates_run() {
    let mut app = app_with_session();
    feed(
        &mut app,
        [
            Event::RunStarted {
                run: run("r1", "s1", None),
                input_id: "in-1".into(),
            },
            Event::UsageRecorded {
                record: UsageRecord {
                    id: "u".into(),
                    run_id: "r1".into(),
                    provider: "openai".into(),
                    model: "gpt-5".into(),
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 20,
                        cached_tokens: 0,
                        requests: 1,
                        estimated_cost_usd: Some(0.01),
                    },
                },
            },
        ],
    );
    let s = app.selected().unwrap();
    assert_eq!(s.runs[&RunId::from("r1")].usage.output_tokens, 20);
}
