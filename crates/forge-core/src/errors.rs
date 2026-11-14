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
    message
        .split_inclusive('\n')
        .filter(|line| {
            !line.match_indices([':', '=']).any(|(index, _)| {
                let prefix = line[..index].trim_end().trim_end_matches(['"', '\'', '\\']);
                let name = prefix
                    .rsplit(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                    .next()
                    .unwrap();
                [
                    "authorization",
                    "proxy-authorization",
                    "x-api-key",
                    "api-key",
                ]
                .iter()
                .any(|header| name.eq_ignore_ascii_case(header))
            })
        })
        .collect()
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
    fn removes_header_lines_and_preserves_other_bytes() {
        let kept = "  上游错误\r\nauthorization failed\r\npreauthorization: pending\r\nx-api-key-info: unavailable\r\n";
        let message = format!(
            "{kept}Authorization: [redacted]\r\nproxy-authorization = hidden\r\n{{\"X-Api-Key\":\"hidden\"}}\r\nrequest headers: 'API-KEY' = hidden\r\n\\\"authorization\\\": hidden\r\nremaining details"
        );
        assert_eq!(
            remove_request_headers(&message),
            format!("{kept}remaining details")
        );
        assert_eq!(remove_request_headers("Authorization: hidden"), "");
        assert_eq!(remove_request_headers(""), "");
    }
}
