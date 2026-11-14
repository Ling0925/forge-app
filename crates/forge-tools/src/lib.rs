//! Only `list_dir`, `glob`, and `grep` filter hidden entries using in-tree `.gitignore`, `.ignore`, and `.git/info/exclude` rules without requiring Git; outside-parent/global rules are excluded, while named reads, writes, and edits bypass filtering.

pub mod cancellation;
pub mod clock;
#[cfg(unix)]
pub mod command;
#[cfg(feature = "failpoints")]
pub mod failpoints;
pub mod files;
pub mod search;
pub mod workspace;

use forge_protocol::ToolCallId;
use std::{future::Future, pin::Pin};
use thiserror::Error;

pub struct ToolRequest<T> {
    pub call: ToolCallId,
    pub input: T,
}
pub struct ToolResult<T> {
    pub call: ToolCallId,
    pub output: T,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
    #[error("{0}")]
    InvalidArguments(String),
    #[error("tool cancelled")]
    Cancelled,
}

pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ToolError>> + Send + 'a>>;

pub trait Tool: Send + Sync {
    type Request: Send;
    type Output: Send;
    fn name(&self) -> &str;
    fn has_side_effects(&self) -> bool;
    fn execute(
        &self,
        request: ToolRequest<Self::Request>,
    ) -> ToolFuture<'_, ToolResult<Self::Output>>;
}

async fn blocking<I: Send + 'static, O: Send + 'static>(
    request: ToolRequest<I>,
    run: impl FnOnce(I) -> Result<O, ToolError> + Send + 'static,
) -> Result<ToolResult<O>, ToolError> {
    let output = tokio::task::spawn_blocking(move || run(request.input)).await??;
    Ok(ToolResult {
        call: request.call,
        output,
    })
}

#[cfg(feature = "failpoints")]
#[macro_export]
macro_rules! failpoint {
    ($points:expr, $name:expr) => {
        $points.hit($name)
    };
}

#[cfg(not(feature = "failpoints"))]
#[macro_export]
macro_rules! failpoint {
    ($points:expr, $name:expr) => {{}};
}

#[cfg(all(test, not(feature = "failpoints")))]
mod tests {
    #[test]
    fn disabled_failpoints_do_not_evaluate_or_resolve_arguments() {
        crate::failpoint!(not_compiled, also_not_compiled);
    }
}
