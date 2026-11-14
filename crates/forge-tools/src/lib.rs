pub mod clock;
#[cfg(unix)]
pub mod command;
#[cfg(feature = "failpoints")]
pub mod failpoints;

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
