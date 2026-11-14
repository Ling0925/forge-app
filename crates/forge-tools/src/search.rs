//! Searches use standard ignore filters and skip hidden entries; traversal does not follow symlinks.

use crate::{
    Tool, ToolError, ToolFuture, ToolRequest, ToolResult, blocking,
    cancellation::CancellationToken, workspace::WorkspaceRoot,
};
use globset::GlobBuilder;
use ignore::WalkBuilder;
use regex::bytes::Regex;
use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

pub struct GlobRequest {
    pub path: PathBuf,
    /// Matches workspace-relative file paths; `*` does not cross directory separators.
    pub pattern: String,
    pub cancellation: CancellationToken,
}

pub struct GrepRequest {
    pub path: PathBuf,
    pub pattern: String,
    pub cancellation: CancellationToken,
}

#[derive(Debug, PartialEq, Eq)]
pub struct GrepMatch {
    pub path: PathBuf,
    /// One-based line number.
    pub line_number: usize,
    /// Line without its terminator; invalid UTF-8 is displayed with replacement characters.
    pub line: String,
}

pub struct GlobTool {
    root: WorkspaceRoot,
}

impl GlobTool {
    pub fn new(root: WorkspaceRoot) -> Self {
        Self { root }
    }
}

impl Tool for GlobTool {
    type Request = GlobRequest;
    type Output = Vec<PathBuf>;

    fn name(&self) -> &str {
        "glob"
    }

    fn has_side_effects(&self) -> bool {
        false
    }

    fn execute(
        &self,
        request: ToolRequest<GlobRequest>,
    ) -> ToolFuture<'_, ToolResult<Self::Output>> {
        let root = self.root.clone();
        Box::pin(blocking(request, move |input| {
            input.cancellation.check()?;
            let pattern = GlobBuilder::new(&input.pattern)
                .literal_separator(true)
                .build()
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?
                .compile_matcher();
            let mut matches = Vec::new();
            visit_files(&root, &input.path, &input.cancellation, |path| {
                let relative = root.relative(path);
                if pattern.is_match(&relative) {
                    matches.push(relative);
                }
                Ok(())
            })?;
            matches.sort();
            input.cancellation.check()?;
            Ok(matches)
        }))
    }
}

pub struct GrepTool {
    root: WorkspaceRoot,
}

impl GrepTool {
    pub fn new(root: WorkspaceRoot) -> Self {
        Self { root }
    }
}

impl Tool for GrepTool {
    type Request = GrepRequest;
    type Output = Vec<GrepMatch>;

    fn name(&self) -> &str {
        "grep"
    }

    fn has_side_effects(&self) -> bool {
        false
    }

    fn execute(
        &self,
        request: ToolRequest<GrepRequest>,
    ) -> ToolFuture<'_, ToolResult<Self::Output>> {
        let root = self.root.clone();
        Box::pin(blocking(request, move |input| {
            input.cancellation.check()?;
            let pattern = Regex::new(&input.pattern)
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
            let mut matches = Vec::new();
            visit_files(&root, &input.path, &input.cancellation, |path| {
                let relative = root.relative(path);
                let mut reader = BufReader::new(File::open(path)?);
                let mut line = Vec::new();
                let mut line_number = 0;
                loop {
                    input.cancellation.check()?;
                    line.clear();
                    if reader.read_until(b'\n', &mut line)? == 0 {
                        break;
                    }
                    line_number += 1;
                    if line.last() == Some(&b'\n') {
                        line.pop();
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                    }
                    if pattern.is_match(&line) {
                        matches.push(GrepMatch {
                            path: relative.clone(),
                            line_number,
                            line: String::from_utf8_lossy(&line).into_owned(),
                        });
                    }
                }
                Ok(())
            })?;
            matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line_number.cmp(&b.line_number)));
            input.cancellation.check()?;
            Ok(matches)
        }))
    }
}

fn visit_files(
    root: &WorkspaceRoot,
    path: &Path,
    cancellation: &CancellationToken,
    mut visit: impl FnMut(&Path) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    cancellation.check()?;
    let mut walk = WalkBuilder::new(root.resolve(path)?)
        .follow_links(false)
        .build();
    loop {
        cancellation.check()?;
        let Some(entry) = walk.next() else {
            return Ok(());
        };
        let entry = entry.map_err(io::Error::other)?;
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            visit(&root.resolve(entry.path())?)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_during_walk_stops_before_visiting_another_file() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a"), "a").unwrap();
        std::fs::write(directory.path().join("b"), "b").unwrap();
        let root = WorkspaceRoot::new(directory.path()).unwrap();
        let cancellation = CancellationToken::default();
        let mut visited = 0;
        let result = visit_files(&root, Path::new("."), &cancellation, |_| {
            visited += 1;
            cancellation.clone().cancel();
            Ok(())
        });
        assert!(matches!(result, Err(ToolError::Cancelled)));
        assert_eq!(visited, 1);
    }
}
