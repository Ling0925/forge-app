use forge_protocol::{Content, MessageRole, ModelSelection, RunId, ToolCallId, Usage};
use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMessage {
    pub role: MessageRole,
    pub content: Content,
    pub tool_call_id: Option<ToolCallId>,
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
    Network,
    Disconnected,
    InvalidResponse,
    Configuration,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn retryable(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Network
                | ErrorKind::Disconnected
                | ErrorKind::Http {
                    status: 429 | 500..=599,
                    ..
                }
        )
    }

    pub fn retry_after(&self) -> Option<&RetryAfter> {
        match &self.kind {
            ErrorKind::Http { retry_after, .. } => retry_after.as_ref(),
            _ => None,
        }
    }
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
            (ErrorKind::Network, true),
            (ErrorKind::Disconnected, true),
            (ErrorKind::InvalidResponse, false),
            (ErrorKind::Configuration, false),
        ] {
            assert_eq!(
                ProviderError {
                    kind,
                    message: "scripted".into()
                }
                .retryable(),
                retryable
            );
        }
    }
}
