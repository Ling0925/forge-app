use forge_protocol::ToolCall;
use forge_provider::ToolDefinition;
use forge_tools::{
    Tool, ToolError, ToolRequest,
    cancellation::CancellationToken,
    files::{DirectoryEntry, ListDirTool, ReadFileRequest, ReadFileTool},
    search::{GlobRequest, GlobTool, GrepMatch, GrepRequest, GrepTool},
    workspace::WorkspaceRoot,
};
use serde::Deserialize;
use std::path::PathBuf;

/// Tool execution outcomes reported back through the runtime loop.
pub(crate) enum Outcome {
    Succeeded(String),
    Failed(String),
    Cancelled,
}

/// Only read-only workspace file/search tools are advertised and executed in
/// this slice. Side-effect tools (write_file, edit_file, command) require the
/// approval-gated slice and are neither advertised nor executable.
pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file inside the workspace, optionally restricted to a one-based inclusive line range. Line endings are preserved.".into(),
            parameters_json: r#"{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path"},"lines":{"type":"array","items":{"type":"integer","minimum":1},"minItems":2,"maxItems":2,"description":"Optional [start, end] one-based inclusive line range"}},"required":["path"]}"#.into(),
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List the non-hidden entries of a workspace directory, honoring in-tree ignore rules. Directories are returned with a trailing slash.".into(),
            parameters_json: r#"{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative directory path"}},"required":["path"]}"#.into(),
        },
        ToolDefinition {
            name: "glob".into(),
            description: "Find workspace files whose relative paths match a glob pattern; `*` does not cross directory separators. Honors in-tree ignore rules and never follows symlinks.".into(),
            parameters_json: r#"{"type":"object","properties":{"pattern":{"type":"string","description":"Glob pattern matched against workspace-relative file paths"},"path":{"type":"string","description":"Workspace-relative directory to search; defaults to the workspace root"}},"required":["pattern"]}"#.into(),
        },
        ToolDefinition {
            name: "grep".into(),
            description: "Search workspace files for lines matching a regular expression; returns one `path:line:text` result per match. Honors in-tree ignore rules and never follows symlinks.".into(),
            parameters_json: r#"{"type":"object","properties":{"pattern":{"type":"string","description":"Regular expression in Rust regex syntax"},"path":{"type":"string","description":"Workspace-relative file or directory to search; defaults to the workspace root"}},"required":["pattern"]}"#.into(),
        },
    ]
}

pub(crate) async fn execute(
    call: ToolCall,
    workspace: PathBuf,
    cancellation: CancellationToken,
) -> Outcome {
    let result = match WorkspaceRoot::new(&workspace) {
        Ok(root) => dispatch(&call, root, cancellation).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(output) => Outcome::Succeeded(output),
        Err(ToolError::Cancelled) => Outcome::Cancelled,
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

async fn dispatch(
    call: &ToolCall,
    root: WorkspaceRoot,
    cancellation: CancellationToken,
) -> Result<String, ToolError> {
    match call.tool.as_str() {
        "read_file" => {
            let args: ReadFileArgs = parse(&call.arguments_json)?;
            ReadFileTool::new(root)
                .execute(ToolRequest {
                    call: call.id.clone(),
                    input: args.request(),
                })
                .await
                .map(|result| result.output)
        }
        "list_dir" => {
            let args: PathArgs = parse(&call.arguments_json)?;
            ListDirTool::new(root)
                .execute(ToolRequest {
                    call: call.id.clone(),
                    input: args.path,
                })
                .await
                .map(|result| entries(&result.output))
        }
        "glob" => {
            let PatternArgs { pattern, path } = parse(&call.arguments_json)?;
            GlobTool::new(root)
                .execute(ToolRequest {
                    call: call.id.clone(),
                    input: GlobRequest {
                        path: path.unwrap_or_else(|| PathBuf::from(".")),
                        pattern,
                        cancellation,
                    },
                })
                .await
                .map(|result| paths(&result.output))
        }
        "grep" => {
            let PatternArgs { pattern, path } = parse(&call.arguments_json)?;
            GrepTool::new(root)
                .execute(ToolRequest {
                    call: call.id.clone(),
                    input: GrepRequest {
                        path: path.unwrap_or_else(|| PathBuf::from(".")),
                        pattern,
                        cancellation,
                    },
                })
                .await
                .map(|result| matches(&result.output))
        }
        _ => Err(ToolError::InvalidArguments(format!(
            "tool is not available in this build: {}",
            call.tool
        ))),
    }
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, ToolError> {
    serde_json::from_str(json).map_err(|error| ToolError::InvalidArguments(error.to_string()))
}

#[derive(Deserialize)]
struct PathArgs {
    path: PathBuf,
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: PathBuf,
    lines: Option<[usize; 2]>,
}
impl ReadFileArgs {
    fn request(self) -> ReadFileRequest {
        ReadFileRequest {
            path: self.path,
            lines: self.lines.map(|[start, end]| start..=end),
        }
    }
}

#[derive(Deserialize)]
struct PatternArgs {
    pattern: String,
    path: Option<PathBuf>,
}

fn entries(entries: &[DirectoryEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            if entry.file_type.is_dir() {
                format!("{}/", entry.path.display())
            } else {
                entry.path.display().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn matches(matches: &[GrepMatch]) -> String {
    matches
        .iter()
        .map(|m| format!("{}:{}:{}", m.path.display(), m.line_number, m.line))
        .collect::<Vec<_>>()
        .join("\n")
}
