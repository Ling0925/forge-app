mod http;
mod sse;
pub use http::*;

use forge_protocol::{Content, MessageRole, ModelSelection, RunId, ToolCallId, Usage};
use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMessage {
    pub role: MessageRole,
    pub content: Content,
    /// On a `role: tool` message: the call this result answers.
    pub tool_call_id: Option<ToolCallId>,
    /// On a `role: assistant` message: the calls it issued, in order.
    pub tool_calls: Vec<ProviderToolCall>,
    /// On a `role: tool` message: the call failed (Anthropic `is_error`).
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequest {
    pub run: RunId,
    pub model: ModelSelection,
    pub system: Option<String>,
    pub messages: Vec<ProviderMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStarted { id: ToolCallId, name: String },
    ToolArgumentsDelta { id: ToolCallId, json: String },
    Usage(Usage),
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryAfter {
    Delay(Duration),
    At(SystemTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    Http {
        status: u16,
        retry_after: Option<RetryAfter>,
    },
    Api,
    Network,
    Disconnected,
    InvalidResponse,
    Configuration,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProviderError {
    pub code: Option<String>,
    pub kind: ErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn redacted(mut self, key: &str) -> Self {
        for value in std::iter::once(&mut self.message).chain(self.code.iter_mut()) {
            *value = redact_secret(value, key);
        }
        self
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Network
                | ErrorKind::Disconnected
                | ErrorKind::InvalidResponse
                | ErrorKind::Http {
                    status: 429 | 500..=599,
                    ..
                }
        ) || (self.kind == ErrorKind::Api
            && !matches!(
                self.code.as_deref(),
                Some(
                    "authentication_error"
                        | "permission_error"
                        | "invalid_api_key"
                        | "insufficient_quota"
                        | "billing_error"
                        | "context_length_exceeded"
                )
            ))
    }

    pub fn retry_after(&self) -> Option<&RetryAfter> {
        match &self.kind {
            ErrorKind::Http { retry_after, .. } => retry_after.as_ref(),
            _ => None,
        }
    }
}

pub fn redact_secret(message: &str, key: &str) -> String {
    if key.is_empty() {
        return message.into();
    }
    let encoded = serde_json::to_string(key).expect("string serialization");
    message
        .replace(key, "[redacted]")
        .replace(&encoded[1..encoded.len() - 1], "[redacted]")
}

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ProviderStream: Send {
    fn next(&mut self) -> ProviderFuture<'_, Option<Result<StreamEvent, ProviderError>>>;
}

pub trait Provider: Send + Sync {
    fn request(
        &self,
        request: ProviderRequest,
    ) -> ProviderFuture<'_, Result<Box<dyn ProviderStream>, ProviderError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryability_preserves_error_classification() {
        for (kind, retryable) in [
            (
                ErrorKind::Http {
                    status: 401,
                    retry_after: None,
                },
                false,
            ),
            (
                ErrorKind::Http {
                    status: 429,
                    retry_after: None,
                },
                true,
            ),
            (
                ErrorKind::Http {
                    status: 503,
                    retry_after: None,
                },
                true,
            ),
            (ErrorKind::Api, true),
            (ErrorKind::Network, true),
            (ErrorKind::Disconnected, true),
            (ErrorKind::InvalidResponse, true),
            (ErrorKind::Configuration, false),
        ] {
            assert_eq!(
                ProviderError {
                    code: None,
                    kind,
                    message: "scripted".into()
                }
                .retryable(),
                retryable
            );
        }
    }

    #[test]
    fn api_retryability_uses_exact_codes_without_inspecting_message() {
        for (code, retryable) in [
            (Some("rate_limit_error"), true),
            (Some("overloaded_error"), true),
            (Some("api_error"), true),
            (Some("server_error"), true),
            (Some("rate_limit_exceeded"), true),
            (Some("authentication_error"), false),
            (Some("permission_error"), false),
            (Some("invalid_api_key"), false),
            (Some("not_overloaded_error"), true),
            (Some("insufficient_quota"), false),
            (Some("billing_error"), false),
            (Some("context_length_exceeded"), false),
            (Some("invalid_request_error"), true),
            (None, true),
        ] {
            for message in [
                "unavailable",
                "rate_limit_error overloaded_error api_error server_error rate_limit_exceeded",
            ] {
                let error = ProviderError {
                    code: code.map(str::to_owned),
                    kind: ErrorKind::Api,
                    message: message.into(),
                };
                assert_eq!(error.retryable(), retryable);
            }
        }
    }
}
