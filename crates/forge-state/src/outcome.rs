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
    /// Where an Unknown/Server failure came from (`storage`, `provider`, …);
    /// lets the card swap its title (§6.7 mapping, 2026-11-14).
    pub source: Option<String>,
}

impl FailureVm {
    pub fn new(category: ErrorKind) -> Self {
        Self {
            category,
            http_status: None,
            retry_after_ms: None,
            retryable: default_retryable(category),
            raw_message: String::new(),
            source: None,
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
    let first = words.next();
    let category = first.map(|w| ErrorKind::from_reason(Some(w)));
    let Some(category) = category.filter(|c| *c != ErrorKind::Unknown || first == Some("unknown"))
    else {
        let mut failure = FailureVm::new(ErrorKind::Unknown);
        failure.raw_message = reason.trim().to_owned();
        return failure;
    };
    // Design mapping (2026-11-14, final table lives in Core's PROTOCOL.md):
    // `provider` (no HTTP status, bad response/stream) → Server card;
    // `storage` during a run → Unknown card titled "保存失败".
    let (category, source) = match category {
        ErrorKind::Provider => (ErrorKind::Server, Some("provider")),
        ErrorKind::Storage => (ErrorKind::Unknown, Some("storage")),
        other => (other, None),
    };
    let mut failure = FailureVm::new(category);
    failure.source = source.map(str::to_owned);
    for word in words {
        match word.split_once('=') {
            Some(("http_status", v)) => failure.http_status = v.parse().ok(),
            Some(("retry_after_ms", v)) => failure.retry_after_ms = v.parse().ok(),
            Some(("retryable", v)) => failure.retryable = v == "true",
            Some(("source", v)) => failure.source = Some(v.to_owned()),
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

    /// Title/body keys, with the source override (Unknown + `storage` →
    /// "保存失败").
    pub fn keys_for(self, failure: &FailureVm) -> (&'static str, &'static str) {
        match (self, failure.source.as_deref()) {
            (Self::Unknown, Some("storage")) => ("error.storage.title", "error.storage.body"),
            _ => self.keys(),
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
    /// V1 status pill text ("失败"); always paired with the colour (UI-REF-02).
    pub status: &'static str,
    /// V1 meta line "来源 · 时间" ("OpenAI · 14:03"); time is when the failure
    /// arrived, empty when unknown.
    pub meta: String,
    pub primary: ErrorAction,
    pub secondary: ErrorAction,
    /// Shortcut hint shown on the primary button ("重试 ⌘R"); only the latest
    /// card of a session keeps it (the shortcut acts on the latest run).
    pub primary_shortcut: Option<&'static str>,
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
        utc_offset_ms: i64,
    ) -> Self {
        let kind = ErrorCardKind::for_failure(failure);
        let (title_key, body_key) = kind.keys_for(failure);
        // §6.7: only Retry-After > 60 s ("服务要求等待") gates the button; a
        // shorter Retry-After is already spent by Core's own retries.
        let retry = match retry_at_ms {
            Some(at) if at > now_ms && kind == ErrorCardKind::RateLimitedWait => {
                RetryButtonState::Waiting {
                    remaining_ms: at - now_ms,
                }
            }
            _ => RetryButtonState::Ready,
        };
        let until = match (retry, retry_at_ms) {
            (RetryButtonState::Waiting { .. }, Some(at)) => clock_time(at, utc_offset_ms),
            _ => String::new(),
        };
        let remaining = match retry {
            RetryButtonState::Waiting { remaining_ms } => format_remaining(remaining_ms),
            RetryButtonState::Ready => String::new(),
        };
        let title = t(title_key).to_owned();
        let body = tf(
            body_key,
            &[
                ("provider", provider),
                ("remaining", remaining.as_str()),
                ("time", until.as_str()),
            ],
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
            let (text, removed) = sanitize_details(&failure.raw_message);
            if !text.is_empty() {
                details.push(text);
            }
            if removed > 0 {
                details.push(tf(
                    "error.detail.headers_removed",
                    &[("n", &removed.to_string())],
                ));
            }
        }
        let (primary, secondary) = kind.actions();
        let primary_shortcut = match primary {
            ErrorAction::Retry => Some("⌘R"),
            ErrorAction::NewSession => Some("⌘N"),
            _ => None,
        };
        Self {
            primary_shortcut,
            run_id: run_id.clone(),
            kind,
            a11y_label: tf("announce.run.failed", &[("title", title.as_str())]),
            title,
            body,
            status: t("card.status.failed"),
            meta: provider.to_owned(),
            primary,
            secondary,
            retry,
            retry_label,
            details_open,
            details,
        }
    }

    /// Appends the local wall-clock time the failure arrived to the meta line.
    pub fn with_time(mut self, at_ms: Option<u64>, utc_offset_ms: i64) -> Self {
        if let Some(at) = at_ms {
            let time = clock_time(at, utc_offset_ms);
            self.meta = if self.meta.is_empty() {
                time
            } else {
                format!("{} · {time}", self.meta)
            };
        }
        self
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

/// Card title for a failure (also used by the "运行失败：<标题>" announcement).
pub fn card_title(failure: &FailureVm) -> &'static str {
    t(ErrorCardKind::for_failure(failure).keys_for(failure).0)
}

/// Local wall-clock "HH:MM" for a UTC-ms instant.
pub fn clock_time(utc_ms: u64, utc_offset_ms: i64) -> String {
    let local = utc_ms as i64 + utc_offset_ms;
    let minutes = local.div_euclid(60_000).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
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

/// Header names whose whole line is dropped from error details (UI-ER-04):
/// anything that names credentials or session state.
const SENSITIVE_HEADER_PARTS: [&str; 8] = [
    "auth",
    "key",
    "token",
    "secret",
    "cookie",
    "session",
    "password",
    "organization",
];

/// `Name:` / `"name":` at the start of `word` → the lower-cased header name.
fn header_name(word: &str) -> Option<String> {
    let word = word.trim_start_matches(['"', '\'', '{', ',', '[']);
    let (name, _) = word.split_once(':')?;
    let name = name.trim_end_matches(['"', '\'']);
    let valid = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| name.to_ascii_lowercase())
}

fn is_sensitive_header(word: &str) -> bool {
    header_name(word).is_some_and(|name| SENSITIVE_HEADER_PARTS.iter().any(|p| name.contains(p)))
}

/// Error-details text for display (UI-ER-04). Request-header lines that carry
/// credentials (`Authorization:`, `x-api-key:`, `Cookie:` …) are removed
/// entirely — a header that starts mid-line cuts the rest of that line — and
/// whatever remains goes through [`redact_secrets`]. Returns the text and the
/// number of header lines removed.
pub fn sanitize_details(text: &str) -> (String, usize) {
    let mut removed = 0;
    let mut lines = Vec::new();
    for line in text.lines() {
        let cut = line
            .split_whitespace()
            .find(|word| is_sensitive_header(word))
            // Byte offset of this word inside `line`.
            .map(|word| word.as_ptr() as usize - line.as_ptr() as usize);
        let kept = match cut {
            Some(offset) => {
                removed += 1;
                line[..offset].trim_end()
            }
            None => line,
        };
        if !kept.trim().is_empty() {
            lines.push(redact_secrets(kept));
        }
    }
    (lines.join("\n"), removed)
}

/// UI-ER-04: no API key may reach the card. Masks `sk-…`-style keys, bearer
/// tokens, `api_key=` values and any long opaque token (≥ 32 chars), line by
/// line (whitespace is preserved).
pub fn redact_secrets(text: &str) -> String {
    text.split('\n')
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_line(text: &str) -> String {
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
    fn header_lines_are_removed_not_masked() {
        let raw = "429 Too Many Requests\nAuthorization: Bearer abc\nx-api-key: hunter2\n\
                   Cookie: sid=1\nrate limit reached. Authorization: Bearer sk-x\ncontent-type: json";
        let (text, removed) = sanitize_details(raw);
        assert_eq!(removed, 4);
        assert_eq!(
            text,
            "429 Too Many Requests\nrate limit reached.\ncontent-type: json"
        );
        for gone in [
            "Authorization",
            "Bearer",
            "hunter2",
            "Cookie",
            "sid=1",
            "sk-x",
        ] {
            assert!(!text.contains(gone), "{gone} in {text}");
        }
        // JSON-ish header dumps too.
        let (text, removed) = sanitize_details(r#"{"error": "bad", "authorization": "Bearer t"}"#);
        assert_eq!((text.as_str(), removed), (r#"{"error": "bad","#, 1));
    }

    #[test]
    fn clock_time_uses_offset() {
        // 2026-11-14 06:05 UTC = 14:05 in UTC+8.
        let utc = 1_794_636_300_000;
        assert_eq!(clock_time(utc, 0), "06:05");
        assert_eq!(clock_time(utc, 8 * 3_600_000), "14:05");
    }

    #[test]
    fn remaining_formats() {
        assert_eq!(format_remaining(100_000), "1 分 40 秒");
        assert_eq!(format_remaining(1), "1 秒");
        assert_eq!(format_remaining(40_000), "40 秒");
    }
}
