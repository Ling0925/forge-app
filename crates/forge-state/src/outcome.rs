//! Structured run outcomes, error cards (spec §6.7) and retry countdowns
//! (§6.4.3 auto-retry marker, R7a Retry-After wait).
//!
//! The source of truth is Core's `RunOutcome` (PROTOCOL.md "Core → UI 错误映射"):
//! [`failure_of`] turns `RunOutcome::Failed` into the card input and
//! [`ErrorCardKind::for_failure`] picks one of the nine §6.7 cards.

use crate::i18n::{t, tf};
use crate::session::ErrorKind;
use forge_protocol::{RunErrorKind, RunId, RunOutcome, UnknownSource};

/// Provider attempts before a run gives up (spec §6.4.3 "共 5 次").
pub const RETRY_MAX_ATTEMPTS: u32 = 5;
/// Retry-After above this is the "服务要求等待" card (R7a); at or below it the
/// plain "请求太频繁" card with an immediate 重试 ⌘R (PROTOCOL.md mapping table).
pub const LONG_RETRY_AFTER_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureVm {
    pub category: ErrorKind,
    pub http_status: Option<u16>,
    pub retry_after_ms: Option<u64>,
    /// Core's `retryable` (informational: P1 never retries on its own, and the
    /// card's 重试 is a user action, so it does not gate the button).
    pub retryable: bool,
    /// Provider/raw message. Shown only under "详情", always via [`sanitize_details`].
    pub raw_message: String,
    /// `Failed.kind = unknown{source}`: lets the Unknown card swap its title
    /// (`storage` → "保存失败").
    pub source: Option<UnknownSource>,
}

impl FailureVm {
    pub fn new(category: ErrorKind) -> Self {
        Self {
            category,
            http_status: None,
            retry_after_ms: None,
            retryable: false,
            raw_message: String::new(),
            source: None,
        }
    }
}

impl From<&RunErrorKind> for ErrorKind {
    fn from(kind: &RunErrorKind) -> Self {
        match kind {
            RunErrorKind::Network => Self::Network,
            RunErrorKind::RateLimited => Self::RateLimited,
            RunErrorKind::Auth => Self::Auth,
            RunErrorKind::Quota => Self::Quota,
            RunErrorKind::Server => Self::Server,
            RunErrorKind::ContextTooLong => Self::ContextTooLong,
            RunErrorKind::Interrupted => Self::Interrupted,
            RunErrorKind::Unknown { .. } => Self::Unknown,
        }
    }
}

/// Card input for a finished run; `None` for succeeded / cancelled runs.
pub fn failure_of(outcome: &RunOutcome) -> Option<FailureVm> {
    let RunOutcome::Failed {
        kind,
        message,
        http_status,
        retry_after_ms,
        retryable,
    } = outcome
    else {
        return None;
    };
    Some(FailureVm {
        category: ErrorKind::from(kind),
        http_status: *http_status,
        retry_after_ms: *retry_after_ms,
        retryable: *retryable,
        raw_message: message.clone(),
        source: match kind {
            RunErrorKind::Unknown { source } => Some(source.clone()),
            _ => None,
        },
    })
}

/// R9 `interrupted` is a `Failed` outcome in P1, rendered with its own divider
/// and sidebar icon.
pub fn is_interrupted(outcome: &RunOutcome) -> bool {
    matches!(
        outcome,
        RunOutcome::Failed {
            kind: RunErrorKind::Interrupted,
            ..
        }
    )
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
            ErrorKind::Unknown => Self::Unknown,
        }
    }

    /// Title/body keys, with the source override (`unknown{source: storage}` →
    /// "保存失败"). `unknown{source: provider}` keeps "运行失败" per the
    /// PROTOCOL.md table ("其余 Provider 错误").
    pub fn keys_for(self, failure: &FailureVm) -> (&'static str, &'static str) {
        match (self, failure.source.as_ref()) {
            (Self::Unknown, Some(UnknownSource::Storage)) => {
                ("error.storage.title", "error.storage.body")
            }
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
        // D2 (designer, 2026-11-14): once the wait is over the card says so
        // instead of rendering the wait template with empty slots.
        let body_key = if kind == ErrorCardKind::RateLimitedWait && retry == RetryButtonState::Ready
        {
            "error.retry_after_ready"
        } else {
            body_key
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
        details.push(tf("error.detail.code", &[("code", &error_code(failure))]));
        if !failure.raw_message.is_empty() {
            details.push(sanitize_details(&failure.raw_message));
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

/// Protocol spelling of `Failed.kind` for the details ("错误码 unknown/storage").
fn error_code(failure: &FailureVm) -> String {
    let code = match failure.category {
        ErrorKind::Network => "network",
        ErrorKind::RateLimited => "rate_limited",
        ErrorKind::Auth => "auth",
        ErrorKind::Quota => "quota",
        ErrorKind::Server => "server",
        ErrorKind::ContextTooLong => "context_too_long",
        ErrorKind::Interrupted => "interrupted",
        ErrorKind::Unknown => "unknown",
    };
    match &failure.source {
        Some(UnknownSource::Provider) => format!("{code}/provider"),
        Some(UnknownSource::Storage) => format!("{code}/storage"),
        None => code.to_owned(),
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

/// Credential header names matched exactly (case-insensitive).
const SENSITIVE_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];
/// …and any single token ending in one of these (`x-api-key`, `token`,
/// `client_secret`, `apikey`).
const SENSITIVE_SUFFIXES: [&str; 5] = ["api-key", "api_key", "apikey", "token", "secret"];

fn is_header_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.is_empty()
        && (SENSITIVE_HEADERS.contains(&lower.as_str())
            || SENSITIVE_SUFFIXES.iter().any(|s| lower.ends_with(s)))
}

/// Byte offset where a credential header starts in `segment`: a sensitive
/// name (optionally quoted / backslash-escaped) directly followed — spaces
/// allowed — by `:` or `=`. `invalid api key: …` does not match: the token
/// before `:` is just `key`.
fn header_start(segment: &str) -> Option<usize> {
    segment.match_indices([':', '=']).find_map(|(index, _)| {
        let prefix = segment[..index]
            .trim_end()
            .trim_end_matches(['"', '\'', '\\']);
        let start = prefix
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_header_char(*c))
            .last()
            .map(|(i, _)| i)?;
        is_sensitive_name(&prefix[start..]).then_some(start)
    })
}

/// Error-details text for display (UI-ER-04, designer rule 2026-11-14; Core
/// applies the same rule and is authoritative — this is a defensive layer).
/// The text is split at real newlines and at literal `\n` / `\r\n` escapes
/// (JSON bodies collapsed onto one line). In each segment a credential header
/// (`Authorization:`, `{'Cookie': …`, `x-api-key=`, `token:` …) is replaced,
/// from its name to the end of the segment, by `[已移除请求头]`; the text
/// before it stays, so the result is never empty for a non-empty message.
/// What remains goes through [`redact_secrets`].
pub fn sanitize_details(text: &str) -> String {
    let normalized = text.replace("\\r\\n", "\n").replace("\\n", "\n");
    let placeholder = crate::i18n::t("error.detail.header_redacted");
    normalized
        .split('\n')
        .map(|segment| {
            let segment = segment.strip_suffix('\r').unwrap_or(segment);
            match header_start(segment) {
                Some(start) => format!("{}{placeholder}", redact_secrets(&segment[..start])),
                None => redact_secrets(segment),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// UI-ER-04: no API key may reach the card. Masks `sk-…`-style keys, the
/// word after `Bearer`, and `api_key=` values, line by line (whitespace is
/// preserved). No length-based guessing.
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
        let looks_secret = mask_next
            || lower.starts_with("sk-")
            || lower.contains("api_key=")
            || lower.contains("apikey=")
            || lower.contains("key=sk");
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
        let raw = "401 invalid key sk-proj-abcdefghijklmnop Bearer xyz api_key=hunter2 ok";
        let red = redact_secrets(raw);
        for secret in ["sk-proj", "xyz", "hunter2"] {
            assert!(!red.contains(secret), "{secret} leaked in {red}");
        }
        assert!(red.starts_with("401 invalid key ••••"));
        assert!(red.ends_with(" ok"));
        // No length heuristic: an ordinary long identifier stays.
        let id = "req_0123456789abcdef0123456789abcdef";
        assert_eq!(redact_secrets(id), id);
    }

    const X: &str = "[已移除请求头]";

    #[test]
    fn header_values_are_replaced_in_segment() {
        let raw = "429 Too Many Requests\nAuthorization: Bearer abc\nx-api-key: hunter2\n\
                   rate limit reached. Authorization: Bearer sk-x\ncontent-type: json";
        assert_eq!(
            sanitize_details(raw),
            format!("429 Too Many Requests\n{X}\n{X}\nrate limit reached. {X}\ncontent-type: json")
        );
    }

    #[test]
    fn d3_final_rule_cases() {
        let cases = [
            // 401 body collapsed onto one line with escaped newlines.
            (
                r#"401 {"error":{"message":"invalid key\nProxy-Authorization: Basic abc\nx-api-key: zzz"}}"#,
                format!("401 {{\"error\":{{\"message\":\"invalid key\n{X}\n{X}"),
            ),
            ("token: abc", X.to_string()),
            ("Cookie: a=b", X.to_string()),
            ("Set-Cookie: sid=1; HttpOnly", X.to_string()),
            (r#"{"X-Api-Key":"k1","ok":1}"#, format!("{{\"{X}")),
            ("client_secret=s3", X.to_string()),
            // Tester's strings.
            ("{'Authorization': 'Bearer qqq'}", format!("{{'{X}")),
            ("Authorization=Bearer tok_123", X.to_string()),
            (
                "invalid api key: please check settings",
                "invalid api key: please check settings".to_string(),
            ),
            (r"a\r\nAuthorization: Bearer q\r\nb", format!("a\n{X}\nb")),
            ("authorization failed", "authorization failed".to_string()),
            (
                "x-api-key-info: unavailable",
                "x-api-key-info: unavailable".to_string(),
            ),
        ];
        for (raw, want) in cases {
            let got = sanitize_details(raw);
            assert_eq!(got, want, "{raw}");
            for leaked in ["abc", "zzz", "qqq", "tok_123", "k1", "s3", "sid=1", "a=b"] {
                if !raw.contains("unavailable") && !raw.starts_with("authorization failed") {
                    assert!(!got.contains(leaked), "{leaked} leaked from {raw}: {got}");
                }
            }
        }
        // A single-line header message never becomes empty.
        assert_eq!(sanitize_details("Authorization: Bearer x"), X);
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
