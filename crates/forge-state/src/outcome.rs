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
    /// V1 meta line "来源 · 时间" ("OpenAI · 14:03"); time is the Core
    /// `finished_at` when known, omitted when it is 0 / missing.
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
        if let Some(at) = known_optional_time(at_ms) {
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

/// A Core UTC-ms timestamp. `0` means the time is unknown (legacy payloads
/// that lacked the field, or old session data) and must not be shown as
/// 1970-01-01 or used to compute a duration.
pub fn known_time(ms: u64) -> Option<u64> {
    (ms != 0).then_some(ms)
}

pub fn known_optional_time(ms: Option<u64>) -> Option<u64> {
    ms.and_then(known_time)
}

/// Elapsed milliseconds when both instants are known and `finished` is not
/// before `started`. Otherwise the duration is omitted (no negative or
/// epoch-spanning "huge" 用时).
pub fn known_duration_ms(started_at: u64, finished_at: u64) -> Option<u64> {
    let started = known_time(started_at)?;
    let finished = known_time(finished_at)?;
    finished.checked_sub(started)
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

/// Mirror of Core's `remove_request_headers` (forge-core errors.rs @
/// 3ea2965; Core is authoritative, this is the defensive second layer).
/// Segments split at LF, CRLF and the literal two-char `\n` / four-char
/// `\r\n`; separators are kept verbatim. In a segment, a credential header
/// name (case-insensitive, optionally quoted / backslash-quoted, then
/// optional spaces / closing quotes, then `:` or `=`) is replaced — from the
/// name's first char to the segment end — by `[已移除请求头]`; text before it
/// (including an opening quote) stays. Names are single words of letters,
/// digits, `-`, `_`: authorization, proxy-authorization, cookie, set-cookie,
/// or anything ending in api-key / api_key / apikey / token / secret.
pub fn remove_request_headers(message: &str) -> String {
    let mut redacted = String::new();
    let mut start = 0;
    for (index, _) in message.match_indices(['\r', '\n', '\\']) {
        if index < start {
            continue;
        }
        if let Some(boundary) = ["\r\n", "\n", r"\r\n", r"\n"]
            .into_iter()
            .find(|boundary| message[index..].starts_with(boundary))
        {
            redact_segment(&message[start..index], &mut redacted);
            redacted.push_str(boundary);
            start = index + boundary.len();
        }
    }
    redact_segment(&message[start..], &mut redacted);
    redacted
}

fn redact_segment(segment: &str, redacted: &mut String) {
    for (index, _) in segment.match_indices([':', '=']) {
        let prefix = segment[..index]
            .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
        let name = prefix
            .rsplit(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            .next()
            .unwrap_or("");
        let lower = name.to_ascii_lowercase();
        if !lower.is_empty()
            && (matches!(
                lower.as_str(),
                "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
            ) || ["api-key", "api_key", "apikey", "token", "secret"]
                .iter()
                .any(|suffix| lower.ends_with(suffix)))
        {
            redacted.push_str(&segment[..prefix.len() - name.len()]);
            redacted.push_str(crate::i18n::t("error.detail.header_redacted"));
            return;
        }
    }
    redacted.push_str(segment);
}

/// Error-details text for display (UI-ER-04): [`remove_request_headers`]
/// (same rule as Core), then literal `\r\n` / `\n` escapes from one-line JSON
/// bodies become real line breaks for reading, then [`redact_secrets`] per
/// line. Never empty for a non-empty message.
pub fn sanitize_details(text: &str) -> String {
    remove_request_headers(text)
        .replace(r"\r\n", "\n")
        .replace(r"\n", "\n")
        .split('\n')
        .map(|line| redact_secrets(line.strip_suffix('\r').unwrap_or(line)))
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

    // Mirrors forge-core errors.rs tests @ 3ea2965.
    #[test]
    fn core_mirror_redacts_credential_headers_from_name_to_segment_end() {
        for (message, expected) in [
            ("Authorization: hidden", "[已移除请求头]"),
            ("Authorization=Bearer tok_123", "[已移除请求头]"),
            ("token: hidden", "[已移除请求头]"),
            ("Cookie: hidden", "[已移除请求头]"),
            ("Set-Cookie: hidden", "[已移除请求头]"),
            ("x-auth-token: hidden", "[已移除请求头]"),
            ("client_secret=hidden", "[已移除请求头]"),
            ("API_KEY: hidden", "[已移除请求头]"),
            ("X-ApiKey: hidden", "[已移除请求头]"),
            (r#"{"x-api-key":"hidden"}"#, "{\"[已移除请求头]"),
            (r#"\"Authorization\": \"Bearer qqq\""#, "\\\"[已移除请求头]"),
            ("{'Authorization': 'Bearer qqq'}", "{'[已移除请求头]"),
            (
                "  上游错误 proxy-AUTHORIZATION = hidden",
                "  上游错误 [已移除请求头]",
            ),
            (
                "request headers: 'API-KEY' = hidden",
                "request headers: '[已移除请求头]",
            ),
        ] {
            assert_eq!(remove_request_headers(message), expected, "{message}");
        }
    }

    #[test]
    fn core_mirror_preserves_all_segment_boundaries() {
        for boundary in ["\n", "\r\n", r"\n", r"\r\n"] {
            let message = format!(
                "details{boundary}Proxy-Authorization: Basic secret{boundary}prefix x-api-key: key{boundary}tail{boundary}"
            );
            assert_eq!(
                remove_request_headers(&message),
                format!(
                    "details{boundary}[已移除请求头]{boundary}prefix [已移除请求头]{boundary}tail{boundary}"
                )
            );
        }
        assert_eq!(
            remove_request_headers("token: a\nCookie: b\\nsecret=c\r\napi-key: d\\r\\ntail"),
            "[已移除请求头]\n[已移除请求头]\\n[已移除请求头]\r\n[已移除请求头]\\r\\ntail"
        );
    }

    #[test]
    fn core_mirror_preserves_non_header_text_exactly() {
        for message in [
            "",
            "invalid api key: please check settings",
            "  上游错误\r\nauthorization failed\r\npreauthorization: pending\r\nx-api-key-info: unavailable\r\n",
        ] {
            assert_eq!(remove_request_headers(message), message);
        }
    }

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
    fn known_time_treats_zero_as_unknown() {
        assert_eq!(known_time(0), None);
        assert_eq!(known_time(1), Some(1));
        assert_eq!(known_optional_time(None), None);
        assert_eq!(known_optional_time(Some(0)), None);
        assert_eq!(known_optional_time(Some(5)), Some(5));
        assert_eq!(known_duration_ms(0, 1_000), None);
        assert_eq!(known_duration_ms(1_000, 0), None);
        assert_eq!(
            known_duration_ms(5_000, 1_000),
            None,
            "finished before start"
        );
        assert_eq!(known_duration_ms(1_000, 5_000), Some(4_000));
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
