//! Render-ready timeline rows (spec §6.3, §6.4, §6.7, §6.8) and the intents
//! behind their controls. forge-ui maps each row onto gpui-kit components.

use crate::app::{AppState, Pending};
use crate::i18n::{format_elapsed, t, tf};
use crate::outcome::{ErrorAction, ErrorCardVm, RetryButtonState, RetryMarkerVm, format_remaining};
use crate::session::{MessageOrigin, RunPhase, SessionVm, TimelineItem};
use forge_protocol::{Command, RunId, RunOutcome, Usage};

/// §8.1 labels quote the first 80 characters.
const A11Y_QUOTE_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningVm {
    /// "思考了 12 秒" when done, "思考中… 0:04" while reasoning.
    pub label: String,
    pub text: String,
    pub expanded: bool,
    pub in_progress: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DividerIcon {
    Success,
    Failed,
    Stopped,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineRowVm {
    User {
        key: String,
        text: String,
        from_queue: bool,
        a11y_label: String,
    },
    Assistant {
        key: String,
        run_id: RunId,
        /// Markdown source.
        text: String,
        streaming: bool,
        reasoning: Option<ReasoningVm>,
        a11y_label: String,
    },
    /// §6.4.3 auto-retry marker (in place of the cleared text).
    Retry {
        key: String,
        run_id: RunId,
        marker: RetryMarkerVm,
    },
    Error(ErrorCardVm),
    Divider {
        key: String,
        run_id: RunId,
        icon: DividerIcon,
        text: String,
        a11y_label: String,
    },
    /// Bottom-of-timeline "运行中 · 0:12" / "正在停止…" (§6.8 first row).
    Running {
        key: String,
        run_id: RunId,
        text: String,
    },
    Notice {
        key: String,
        text: String,
    },
}

impl TimelineRowVm {
    pub fn key(&self) -> &str {
        match self {
            Self::User { key, .. }
            | Self::Assistant { key, .. }
            | Self::Retry { key, .. }
            | Self::Divider { key, .. }
            | Self::Running { key, .. }
            | Self::Notice { key, .. } => key,
            Self::Error(card) => card.run_id.0.as_str(),
        }
    }
}

/// What the host must do after an error-card action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorEffect {
    None,
    OpenSettings,
    OpenInspectorChanges(RunId),
}

fn quote(text: &str) -> String {
    let mut out: String = text.chars().take(A11Y_QUOTE_CHARS).collect();
    if text.chars().count() > A11Y_QUOTE_CHARS {
        out.push('…');
    }
    out
}

/// "46 秒" / "1 分 40 秒".
pub fn format_duration_words(ms: u64) -> String {
    // Round to whole seconds (format_remaining rounds up, which is right for
    // countdowns but not for elapsed time).
    format_remaining((ms / 1000).max(1) * 1000)
}

/// "12.3k" for token counts (§6.8).
pub fn format_tokens(n: u64) -> String {
    if n >= 1000 {
        let tenths = (n + 50) / 100;
        format!("{}.{}k", tenths / 10, tenths % 10)
    } else {
        n.to_string()
    }
}

fn usage_text(usage: &Usage) -> String {
    if usage.input_tokens == 0 && usage.output_tokens == 0 {
        return t("divider.usage_pending").to_owned();
    }
    let mut text = tf(
        "divider.tokens",
        &[
            ("input", &format_tokens(usage.input_tokens)),
            ("output", &format_tokens(usage.output_tokens)),
        ],
    );
    if let Some(cost) = usage.estimated_cost_usd {
        text.push_str(" · ");
        text.push_str(&tf("divider.cost", &[("cost", &format!("{cost:.2}"))]));
    }
    text
}

impl AppState {
    fn provider_name(&self) -> String {
        self.settings
            .settings
            .as_ref()
            .map(|s| s.model.provider.0.clone())
            .filter(|p| !p.is_empty())
            .map(|p| match p.as_str() {
                "openai" => "OpenAI".to_owned(),
                "anthropic" => "Anthropic".to_owned(),
                _ => p,
            })
            .unwrap_or_else(|| "服务商".to_owned())
    }

    /// Rows for the selected session, in display order.
    pub fn timeline_rows(&self) -> Vec<TimelineRowVm> {
        let Some(vm) = self.selected() else {
            return Vec::new();
        };
        self.rows_for(vm)
    }

    pub fn rows_for(&self, vm: &SessionVm) -> Vec<TimelineRowVm> {
        let now = self.now_ms;
        let provider = self.provider_name();
        let mut rows = Vec::with_capacity(vm.timeline.len() + 1);
        for (index, item) in vm.timeline.iter().enumerate() {
            match item {
                TimelineItem::User(message) => rows.push(TimelineRowVm::User {
                    key: format!("user-{}", message.input_id.0),
                    text: message.text.clone(),
                    from_queue: matches!(message.origin, MessageOrigin::Queue(_)),
                    a11y_label: tf("a11y.timeline.user", &[("text", &quote(&message.text))]),
                }),
                TimelineItem::Assistant(message) => {
                    let run = vm.runs.get(&message.run_id);
                    if let Some(RunPhase::Retrying {
                        attempt,
                        reason,
                        next_at_ms,
                        ..
                    }) = run.map(|r| &r.phase)
                    {
                        rows.push(TimelineRowVm::Retry {
                            key: format!("retry-{}", message.run_id.0),
                            run_id: message.run_id.clone(),
                            marker: RetryMarkerVm::build(*attempt, *next_at_ms, reason, now),
                        });
                        if message.text.is_empty() && message.reasoning.is_empty() {
                            continue;
                        }
                    }
                    let in_progress = message.streaming && message.reasoning_ms.is_none();
                    let reasoning = (!message.reasoning.is_empty()).then(|| {
                        let label = match (message.reasoning_ms, message.reasoning_started_ms) {
                            (Some(ms), _) => tf(
                                "assistant.reasoning",
                                &[("secs", &(ms / 1000).max(1).to_string())],
                            ),
                            (None, Some(start)) => tf(
                                "assistant.thinking",
                                &[("elapsed", &format_elapsed(now.saturating_sub(start) / 1000))],
                            ),
                            (None, None) => t("a11y.reasoning").to_owned(),
                        };
                        ReasoningVm {
                            label,
                            text: message.reasoning.clone(),
                            expanded: vm.expanded_reasoning.contains(&message.run_id),
                            in_progress,
                        }
                    });
                    let mut text = message.text.clone();
                    if message.stopped {
                        text.push_str(t("assistant.stopped_suffix"));
                    }
                    rows.push(TimelineRowVm::Assistant {
                        key: format!("assistant-{}", message.run_id.0),
                        run_id: message.run_id.clone(),
                        a11y_label: tf("a11y.timeline.assistant", &[("text", &quote(&text))]),
                        text,
                        streaming: message.streaming,
                        reasoning,
                    });
                }
                TimelineItem::Error(error) => rows.push(TimelineRowVm::Error(ErrorCardVm::build(
                    &error.run_id,
                    &error.failure,
                    error.retry_at_ms,
                    now,
                    &provider,
                    vm.expanded_details.contains(&error.run_id),
                ))),
                TimelineItem::RunDivider { run_id, outcome } => {
                    let run = vm.runs.get(run_id);
                    let (icon, key) = match outcome {
                        RunOutcome::Succeeded => (DividerIcon::Success, "run.succeeded"),
                        RunOutcome::Failed => (DividerIcon::Failed, "run.failed"),
                        RunOutcome::Cancelled => (DividerIcon::Stopped, "run.cancelled"),
                        RunOutcome::Interrupted => (DividerIcon::Interrupted, "run.interrupted"),
                    };
                    let mut text = t(key).to_owned();
                    let mut a11y = tf("a11y.divider", &[("outcome", t(key))]);
                    if let Some(run) = run {
                        if let Some(finished) = run.finished_at_ms {
                            let elapsed =
                                format_duration_words(finished.saturating_sub(run.started_at_ms));
                            text.push_str(" · ");
                            text.push_str(&tf("divider.elapsed", &[("elapsed", &elapsed)]));
                            a11y.push('，');
                            a11y.push_str(&tf("divider.elapsed", &[("elapsed", &elapsed)]));
                        }
                        text.push_str(" · ");
                        text.push_str(&usage_text(&run.usage));
                    }
                    rows.push(TimelineRowVm::Divider {
                        key: format!("divider-{}-{index}", run_id.0),
                        run_id: run_id.clone(),
                        icon,
                        text,
                        a11y_label: a11y,
                    });
                }
                TimelineItem::Notice(notice) => rows.push(TimelineRowVm::Notice {
                    key: format!("notice-{index}"),
                    text: notice.text.clone(),
                }),
            }
        }
        if let Some(run) = vm.running() {
            // Retrying before the first token: no assistant row carries the
            // marker yet, so it takes the place of the running indicator.
            if let RunPhase::Retrying {
                attempt,
                reason,
                next_at_ms,
                ..
            } = &run.phase
                && !rows
                    .iter()
                    .any(|r| matches!(r, TimelineRowVm::Retry { run_id, .. } if run_id == &run.id))
            {
                rows.push(TimelineRowVm::Retry {
                    key: format!("retry-{}", run.id.0),
                    run_id: run.id.clone(),
                    marker: RetryMarkerVm::build(*attempt, *next_at_ms, reason, now),
                });
                return rows;
            }
            let text = match run.phase {
                RunPhase::Cancelling => t("composer.stopping").to_owned(),
                _ => tf(
                    "run.running",
                    &[(
                        "elapsed",
                        &format_elapsed(now.saturating_sub(run.started_at_ms) / 1000),
                    )],
                ),
            };
            rows.push(TimelineRowVm::Running {
                key: format!("running-{}", run.id.0),
                run_id: run.id.clone(),
                text,
            });
        }
        rows
    }

    /// Reasoning disclosure (§6.4.3, UI-MSG-03). Returns the new state.
    pub fn toggle_reasoning(&mut self, run: &RunId) -> bool {
        let Some(vm) = self.selected_mut() else {
            return false;
        };
        if !vm.expanded_reasoning.remove(run) {
            vm.expanded_reasoning.insert(run.clone());
            return true;
        }
        false
    }

    /// Activates an error-card button.
    pub fn error_action(&mut self, run: &RunId, action: ErrorAction) -> ErrorEffect {
        match action {
            ErrorAction::Retry => {
                self.retry_run(run);
                ErrorEffect::None
            }
            ErrorAction::NewSession => {
                self.create_session();
                ErrorEffect::None
            }
            ErrorAction::OpenSettings => ErrorEffect::OpenSettings,
            ErrorAction::ViewChanges => ErrorEffect::OpenInspectorChanges(run.clone()),
            ErrorAction::Details => {
                if let Some(vm) = self.selected_mut()
                    && !vm.expanded_details.remove(run)
                {
                    vm.expanded_details.insert(run.clone());
                }
                ErrorEffect::None
            }
        }
    }

    /// Retry state of a finished run's error card (Retry-After countdown).
    pub fn retry_state(&self, run: &RunId) -> Option<RetryButtonState> {
        let vm = self.selected()?;
        vm.timeline.iter().find_map(|item| match item {
            TimelineItem::Error(error) if &error.run_id == run => Some(match error.retry_at_ms {
                Some(at) if at > self.now_ms => RetryButtonState::Waiting {
                    remaining_ms: at - self.now_ms,
                },
                _ => RetryButtonState::Ready,
            }),
            _ => None,
        })
    }

    /// `RetryRun` (R8). Blocked while Retry-After has not passed, while a run is
    /// active, and while a retry for the same run is already in flight.
    pub fn retry_run(&mut self, run: &RunId) -> Option<crate::app::CommandToken> {
        if matches!(
            self.retry_state(run),
            Some(RetryButtonState::Waiting { .. })
        ) {
            return None;
        }
        let vm = self.selected()?;
        if vm.running().is_some() {
            return None;
        }
        let finished = vm.runs.get(run).is_some_and(|r| r.phase.is_terminal());
        if !finished {
            return None;
        }
        let in_flight = self
            .pending
            .values()
            .any(|(_, command)| matches!(command, Command::RetryRun { run: r } if r == run));
        if in_flight {
            return None;
        }
        Some(self.push(Pending::Retry, Command::RetryRun { run: run.clone() }))
    }
}
