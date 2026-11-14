//! Structured run outcomes, error cards (spec §6.7) and retry countdowns
//! (§6.4.3 auto-retry marker, R7a Retry-After wait).
//!
//! These are forge-state-local view types shaped like the agreed P1 protocol
//! `RunFinished.outcome = success | cancelled{by} | failed{category,
//! http_status, retry_after_ms, retryable, message}`. Until Core lands that
//! shape, [`outcome_from_p0`] adapts today's P0 `RunFinished{outcome, reason}`.

use crate::i18n::{t, tf};
use crate::session::ErrorKind;
use forge_protocol::{RunId, RunOutcome};

/// Provider attempts before a run gives up (spec §6.4.3 "共 5 次").
pub const RETRY_MAX_ATTEMPTS: u32 = 5;
/// Retry-After above this is the "服务要求等待" card (R7a) instead of auto-retry.
pub const LONG_RETRY_AFTER_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelledBy {
    User,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureVm {
    pub category: ErrorKind,
    pub http_status: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub retryable: bool,
    /// Provider/raw message. Shown only under "详情", always via [`redact_secrets`].
    pub raw_message: String,
}

impl FailureVm {
    pub fn new(category: ErrorKind) -> Self {
        Self {
            category,
            http_status: None,
            retry_after_ms: None,
            retryable: default_retryable(category),
            raw_message: String::new(),
        }
    }
}

/// Retry is offered for everything except auth/quota/context, where retrying
/// the same input cannot succeed without user action (§6.7 primary actions).
pub fn default_retryable(category: ErrorKind) -> bool {
    !matches!(
        category,
        ErrorKind::Auth | ErrorKind::Quota | ErrorKind::ContextTooLong
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcomeVm {
    Succeeded,
    Cancelled {
        by: CancelledBy,
    },
    Failed(FailureVm),
    /// R9: found unfinished at startup.
    Interrupted,
}

impl RunOutcomeVm {
    pub fn outcome(&self) -> RunOutcome {
        match self {
            Self::Succeeded => RunOutcome::Succeeded,
            Self::Cancelled { .. } => RunOutcome::Cancelled,
            Self::Failed(_) => RunOutcome::Failed,
            Self::Interrupted => RunOutcome::Interrupted,
        }
    }

    pub fn failure(&self) -> Option<FailureVm> {
        match self {
            Self::Failed(failure) => Some(failure.clone()),
            Self::Interrupted => Some(FailureVm {
                retryable: true,
                ..FailureVm::new(ErrorKind::Interrupted)
            }),
            _ => None,
        }
    }
}

/// Adapter from the P0 `RunFinished { outcome, reason: Option<String> }`.
///
/// TODO(core-p1): replace with a `From<forge_protocol::RunOutcome>` once
/// `RunFinished.outcome` carries `cancelled{by}` and `failed{category,
/// http_status, retry_after_ms, retryable, message}`. Until then `reason` is
/// parsed leniently as `<category>[ key=value ...][: message]`, e.g.
/// `rate_limited http_status=429 retry_after_ms=120000: slow down`; anything
/// unrecognised becomes `Unknown` with the whole string as the raw message.
/// `cancelled{by}` is inferred from what the UI itself sent.
pub fn outcome_from_p0(
    outcome: RunOutcome,
    reason: Option<&str>,
    cancel_requested: bool,
    shutting_down: bool,
) -> RunOutcomeVm {
    match outcome {
        RunOutcome::Succeeded => RunOutcomeVm::Succeeded,
        RunOutcome::Interrupted => RunOutcomeVm::Interrupted,
        RunOutcome::Cancelled => RunOutcomeVm::Cancelled {
            by: if shutting_down && !cancel_requested {
                CancelledBy::Shutdown
            } else {
                CancelledBy::User
            },
        },
        RunOutcome::Failed => RunOutcomeVm::Failed(parse_p0_reason(reason.unwrap_or(""))),
    }
}

fn parse_p0_reason(reason: &str) -> FailureVm {
    let (head, message) = match reason.split_once(':') {
        Some((head, message)) => (head, message.trim()),
        None => (reason, ""),
    };
    let mut words = head.split_whitespace();
    let category = words.next().map(|w| ErrorKind::from_reason(Some(w)));
    let Some(category) = category.filter(|c| *c != ErrorKind::Unknown || head.trim() == "unknown")
    else {
        let mut failure = FailureVm::new(ErrorKind::Unknown);
        failure.raw_message = reason.trim().to_owned();
        return failure;
    };
    let mut failure = FailureVm::new(category);
    for word in words {
        match word.split_once('=') {
            Some(("http_status", v)) => failure.http_status = v.parse().ok(),
            Some(("retry_after_ms", v)) => failure.retry_after_ms = v.parse().ok(),
            Some(("retryable", v)) => failure.retryable = v == "true",
            _ => {}
        }
    }
    failure.raw_message = message.to_owned();
    failure
}

/// The nine error cards of spec §6.7 (RateLimited splits on Retry-After).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCardKind {
    Network,
    RateLimited,
    RateLimitedWait,
    Auth,
    Quota,
    Server,
    ContextTooLong,
    Interrupted,
    Unknown,
}

impl ErrorCardKind {
    pub const ALL: [Self; 9] = [
        Self::Network,
        Self::RateLimited,
        Self::RateLimitedWait,
        Self::Auth,
        Self::Quota,
        Self::Server,
        Self::ContextTooLong,
        Self::Interrupted,
        Self::Unknown,
    ];

    pub fn for_failure(failure: &FailureVm) -> Self {
        match failure.category {
            ErrorKind::Network => Self::Network,
            ErrorKind::RateLimited
                if failure
                    .retry_after_ms
                    .is_some_and(|ms| ms > LONG_RETRY_AFTER_MS) =>
            {
                Self::RateLimitedWait
            }
            ErrorKind::RateLimited => Self::RateLimited,
            ErrorKind::Auth => Self::Auth,
            ErrorKind::Quota => Self::Quota,
            ErrorKind::Server => Self::Server,
            ErrorKind::ContextTooLong => Self::ContextTooLong,
            ErrorKind::Interrupted => Self::Interrupted,
            ErrorKind::Provider | ErrorKind::Storage | ErrorKind::Unknown => Self::Unknown,
        }
    }

    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Self::Network => ("error.network.title", "error.network.body"),
            Self::RateLimited => ("error.rate_limited.title", "error.rate_limited.body"),
            Self::RateLimitedWait => (
                "error.rate_limited_wait.title",
                "error.rate_limited_wait.body",
            ),
            Self::Auth => ("error.auth.title", "error.auth.body"),
            Self::Quota => ("error.quota.title", "error.quota.body"),
            Self::Server => ("error.server.title", "error.server.body"),
            Self::ContextTooLong => ("error.context.title", "error.context.body"),
            Self::Interrupted => ("error.interrupted.title", "error.interrupted.body"),
            Self::Unknown => ("error.unknown.title", "error.unknown.body"),
        }
    }

    /// (primary, secondary) per the §6.7 table.
    pub fn actions(self) -> (ErrorAction, ErrorAction) {
        use ErrorAction::*;
        match self {
            Self::Network | Self::RateLimited | Self::RateLimitedWait | Self::Server => {
                (Retry, Details)
            }
            Self::Auth => (OpenSettings, Retry),
            Self::Quota => (OpenSettings, Details),
            Self::ContextTooLong => (NewSession, Retry),
            Self::Interrupted => (Retry, ViewChanges),
            Self::Unknown => (Retry, Details),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorAction {
    Retry,
    OpenSettings,
    NewSession,
    ViewChanges,
    Details,
}

impl ErrorAction {
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Retry => "error.action.retry",
            Self::OpenSettings => "error.action.settings",
            Self::NewSession => "error.action.new",
            Self::ViewChanges => "error.action.view_changes",
            Self::Details => "error.action.details",
        }
    }
}

/// The retry button of an error card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryButtonState {
    Ready,
    /// Retry-After not reached: disabled, label shows the remaining time.
    Waiting {
        remaining_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCardVm {
    pub run_id: RunId,
    pub kind: ErrorCardKind,
    pub title: String,
    pub body: String,
    pub primary: ErrorAction,
    pub secondary: ErrorAction,
    pub retry: RetryButtonState,
    /// Localised label for the retry button ("重试" / "重试（1 分 40 秒）").
    pub retry_label: String,
    pub details_open: bool,
    /// "HTTP 429", "错误码 rate_limited", raw message — all redacted.
    pub details: Vec<String>,
    /// §8.1: Group "运行失败：<标题>", description = body.
    pub a11y_label: String,
}

impl ErrorCardVm {
    pub fn build(
        run_id: &RunId,
        failure: &FailureVm,
        retry_at_ms: Option<u64>,
        now_ms: u64,
        provider: &str,
        details_open: bool,
    ) -> Self {
        let kind = ErrorCardKind::for_failure(failure);
        let (title_key, body_key) = kind.keys();
        let retry = match retry_at_ms {
            Some(at) if at > now_ms => RetryButtonState::Waiting {
                remaining_ms: at - now_ms,
            },
            _ => RetryButtonState::Ready,
        };
        let remaining = match retry {
            RetryButtonState::Waiting { remaining_ms } => format_remaining(remaining_ms),
            RetryButtonState::Ready => String::new(),
        };
        let title = t(title_key).to_owned();
        let body = tf(
            body_key,
            &[("provider", provider), ("remaining", remaining.as_str())],
        );
        let retry_label = match retry {
            RetryButtonState::Ready => t("error.action.retry").to_owned(),
            RetryButtonState::Waiting { .. } => tf(
                "error.action.retry_wait",
                &[("remaining", remaining.as_str())],
            ),
        };
        let mut details = Vec::new();
        if let Some(status) = failure.http_status {
            details.push(format!("HTTP {status}"));
        }
        details.push(tf(
            "error.detail.code",
            &[("code", category_code(failure.category))],
        ));
        if !failure.raw_message.is_empty() {
            details.push(redact_secrets(&failure.raw_message));
        }
        let (primary, secondary) = kind.actions();
        Self {
            run_id: run_id.clone(),
            kind,
            a11y_label: tf("announce.run.failed", &[("title", title.as_str())]),
            title,
            body,
            primary,
            secondary,
            retry,
            retry_label,
            details_open,
            details,
        }
    }
}

fn category_code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Network => "network",
        ErrorKind::RateLimited => "rate_limited",
        ErrorKind::Auth => "auth",
        ErrorKind::Quota => "quota",
        ErrorKind::Server => "server",
        ErrorKind::ContextTooLong => "context_too_long",
        ErrorKind::Provider => "provider",
        ErrorKind::Storage => "storage",
        ErrorKind::Interrupted => "interrupted",
        ErrorKind::Unknown => "unknown",
    }
}

/// "1 分 40 秒" / "40 秒" (rounded up so it never shows 0 while waiting).
pub fn format_remaining(ms: u64) -> String {
    let secs = ms.div_ceil(1000);
    if secs >= 60 {
        tf(
            "time.min_sec",
            &[
                ("m", &(secs / 60).to_string()),
                ("s", &(secs % 60).to_string()),
            ],
        )
    } else {
        tf("time.sec", &[("s", &secs.to_string())])
    }
}

/// Auto-retry marker shown in place of the cleared text (§6.4.3 "自动重试中").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryMarkerVm {
    pub attempt: u32,
    pub max: u32,
    pub remaining_ms: u64,
    pub text: String,
}

impl RetryMarkerVm {
    pub fn build(attempt: u32, next_at_ms: u64, reason: &str, now_ms: u64) -> Self {
        let remaining_ms = next_at_ms.saturating_sub(now_ms);
        let reason = if reason.is_empty() { "-" } else { reason };
        Self {
            attempt,
            max: RETRY_MAX_ATTEMPTS,
            remaining_ms,
            text: tf(
                "retry.marker",
                &[
                    ("reason", reason),
                    ("secs", &remaining_ms.div_ceil(1000).to_string()),
                    ("n", &attempt.to_string()),
                ],
            ),
        }
    }
}

/// UI-ER-04: no API key may reach the card. Masks `sk-…`-style keys, bearer
/// tokens, `api_key=` values and any long opaque token (≥ 32 chars).
pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut mask_next = false;
    for (i, piece) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let lower = piece.to_ascii_lowercase();
        let token =
            piece.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
        let looks_secret = mask_next
            || lower.starts_with("sk-")
            || lower.contains("api_key=")
            || lower.contains("apikey=")
            || lower.contains("key=sk")
            || (token.len() >= 32
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        mask_next = lower == "bearer" || lower == "authorization:";
        if looks_secret {
            match piece.split_once('=') {
                Some((name, _)) if !lower.starts_with("sk-") => {
                    out.push_str(name);
                    out.push_str("=••••");
                }
                _ => out.push_str("••••"),
            }
        } else {
            out.push_str(piece);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_keys_and_tokens() {
        let raw = "401 invalid key sk-proj-abcdefghijklmnop Authorization: Bearer xyz api_key=hunter2 \
                   0123456789abcdef0123456789abcdef ok";
        let red = redact_secrets(raw);
        for secret in ["sk-proj", "xyz", "hunter2", "0123456789abcdef"] {
            assert!(!red.contains(secret), "{secret} leaked in {red}");
        }
        assert!(red.starts_with("401 invalid key ••••"));
        assert!(red.ends_with(" ok"));
    }

    #[test]
    fn remaining_formats() {
        assert_eq!(format_remaining(100_000), "1 分 40 秒");
        assert_eq!(format_remaining(1), "1 秒");
        assert_eq!(format_remaining(40_000), "40 秒");
    }
}
