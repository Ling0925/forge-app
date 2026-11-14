use forge_protocol::{ProviderId, ProviderKeyErrorKind, RunErrorKind, RunOutcome, UnknownSource};
use forge_provider::{ErrorKind, ProviderError, RetryAfter};
use forge_store::StoreError;
use std::time::{Duration, UNIX_EPOCH};

fn kind(error: &ProviderError, provider: &ProviderId) -> RunErrorKind {
    let status = match error.kind {
        ErrorKind::Http { status, .. } => Some(status),
        _ => None,
    };
    match (&error.kind, status, error.code.as_deref()) {
        (ErrorKind::Network | ErrorKind::Disconnected, _, _) => RunErrorKind::Network,
        (ErrorKind::Configuration, _, _) => RunErrorKind::Auth,
        (ErrorKind::InvalidResponse, _, _) => RunErrorKind::Server,
        (_, Some(401 | 403), _)
        | (_, _, Some("authentication_error" | "permission_error" | "invalid_api_key")) => {
            RunErrorKind::Auth
        }
        (_, Some(402), _) | (_, _, Some("insufficient_quota" | "billing_error")) => {
            RunErrorKind::Quota
        }
        (_, Some(429), _) | (_, _, Some("rate_limit_error" | "rate_limit_exceeded")) => {
            RunErrorKind::RateLimited
        }
        (_, Some(500..=599), _)
        | (_, _, Some("overloaded_error" | "api_error" | "server_error")) => RunErrorKind::Server,
        (_, _, Some("context_length_exceeded")) => RunErrorKind::ContextTooLong,
        (_, _, Some("invalid_request_error"))
            if provider.0 == "anthropic" && error.message.contains("prompt is too long") =>
        {
            RunErrorKind::ContextTooLong
        }
        (ErrorKind::Api, None, _) => RunErrorKind::Server,
        _ => RunErrorKind::Unknown {
            source: UnknownSource::Provider,
        },
    }
}
pub(crate) fn key_error(error: &ProviderError, provider: &ProviderId) -> ProviderKeyErrorKind {
    if error.kind == ErrorKind::Configuration {
        return ProviderKeyErrorKind::Configuration;
    }
    match kind(error, provider) {
        RunErrorKind::Auth => ProviderKeyErrorKind::Auth,
        RunErrorKind::Quota => ProviderKeyErrorKind::Quota,
        RunErrorKind::RateLimited => ProviderKeyErrorKind::RateLimited,
        RunErrorKind::Server => ProviderKeyErrorKind::Server,
        RunErrorKind::Network => ProviderKeyErrorKind::Network,
        _ => ProviderKeyErrorKind::Provider,
    }
}
fn remove_request_headers(message: &str) -> String {
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
            .unwrap();
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
        ) || ["api-key", "api_key", "apikey", "token", "secret"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
        {
            redacted.push_str(&segment[..prefix.len() - name.len()]);
            redacted.push_str("[已移除请求头]");
            return;
        }
    }
    redacted.push_str(segment);
}
pub(crate) fn outcome(error: ProviderError, now: Duration, provider: &ProviderId) -> RunOutcome {
    let http_status = match error.kind {
        ErrorKind::Http { status, .. } => Some(status),
        _ => None,
    };
    let retry_after_ms = error
        .retry_after()
        .map(|after| match after {
            RetryAfter::Delay(delay) => *delay,
            RetryAfter::At(at) => at
                .duration_since(UNIX_EPOCH + now)
                .unwrap_or(Duration::ZERO),
        })
        .map(|duration| duration.as_millis() as u64);
    let kind = kind(&error, provider);
    let retryable = error.retryable() && kind != RunErrorKind::ContextTooLong;
    RunOutcome::Failed {
        kind,
        http_status,
        retry_after_ms,
        retryable,
        message: remove_request_headers(&error.message),
    }
}

pub(crate) fn storage_outcome(error: StoreError, key: Option<&str>) -> RunOutcome {
    let mut message = error.to_string();
    if let Some(key) = key {
        message = forge_provider::redact_secret(&message, key);
    }
    RunOutcome::Failed {
        kind: RunErrorKind::Unknown {
            source: UnknownSource::Storage,
        },
        message: remove_request_headers(&message),
        http_status: None,
        retry_after_ms: None,
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credential_headers_from_name_to_segment_end() {
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
    fn redaction_preserves_all_segment_boundaries() {
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
    fn redaction_preserves_non_header_text_exactly() {
        for message in [
            "",
            "invalid api key: please check settings",
            "  上游错误\r\nauthorization failed\r\npreauthorization: pending\r\nx-api-key-info: unavailable\r\n",
        ] {
            assert_eq!(remove_request_headers(message), message);
        }
    }
}
