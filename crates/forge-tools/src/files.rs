use crate::{
    Tool, ToolError, ToolFuture, ToolRequest, ToolResult, blocking,
    search::{IgnoreScope, is_ignored},
    workspace::{FileSnapshot, ResolvedPath, WorkspaceRoot},
};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::{io, ops::RangeInclusive, path::PathBuf};

pub struct ReadFileRequest {
    pub path: PathBuf,
    /// One-based, inclusive line range. Line endings are preserved.
    pub lines: Option<RangeInclusive<usize>>,
}

pub struct WriteFileRequest {
    pub path: PathBuf,
    pub content: String,
}

pub struct EditFileRequest {
    pub path: PathBuf,
    pub old_string: String,
    pub new_string: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Resolved target path relative to the workspace root.
    pub path: PathBuf,
    pub before_sha256: Option<String>,
    pub after_sha256: String,
    /// Inserted byte lines from Myers `TextDiff::from_lines`, retaining line endings.
    /// New files count all lines (empty: zero); an unterminated final line counts once.
    pub lines_added: u32,
    /// Deleted lines under the same rules; new files remove zero, unchanged writes count zero for both.
    /// Final-newline-only changes remove and add one line; CRLF is compared intact.
    pub lines_removed: u32,
}

#[derive(Debug)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub file_type: cap_std::fs::FileType,
}

pub struct ReadFileTool {
    root: WorkspaceRoot,
}

impl ReadFileTool {
    pub fn new(root: WorkspaceRoot) -> Self {
        Self { root }
    }
}

impl Tool for ReadFileTool {
    type Request = ReadFileRequest;
    type Output = String;

    fn name(&self) -> &str {
        "read_file"
    }

    fn has_side_effects(&self) -> bool {
        false
    }

    fn execute(&self, request: ToolRequest<ReadFileRequest>) -> ToolFuture<'_, ToolResult<String>> {
        let root = self.root.clone();
        Box::pin(blocking(request, move |input| {
            if let Some(lines) = &input.lines
                && (*lines.start() == 0 || lines.is_empty())
            {
                return Err(ToolError::InvalidArguments(
                    "line range must be one-based and increasing".into(),
                ));
            }
            let content = io::read_to_string(root.resolve(&input.path)?.open()?)?;
            Ok(match input.lines {
                Some(lines) => content
                    .split_inclusive('\n')
                    .skip(lines.start() - 1)
                    .take(lines.end() - lines.start() + 1)
                    .collect(),
                None => content,
            })
        }))
    }
}

pub struct WriteFileTool {
    root: WorkspaceRoot,
    #[cfg(feature = "failpoints")]
    pub failpoints: crate::failpoints::Failpoints,
}

impl WriteFileTool {
    pub fn new(root: WorkspaceRoot) -> Self {
        Self {
            root,
            #[cfg(feature = "failpoints")]
            failpoints: crate::failpoints::Failpoints::default(),
        }
    }
}

impl Tool for WriteFileTool {
    type Request = WriteFileRequest;
    type Output = FileChange;

    fn name(&self) -> &str {
        "write_file"
    }

    fn has_side_effects(&self) -> bool {
        true
    }

    fn execute(
        &self,
        request: ToolRequest<WriteFileRequest>,
    ) -> ToolFuture<'_, ToolResult<FileChange>> {
        let root = self.root.clone();
        Box::pin(async move {
            let result = blocking(request, move |input| {
                let path = root.resolve_write(&input.path)?;
                let before = match path.read() {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                };
                write(&path, before.as_ref(), input.content.as_bytes())
            })
            .await?;
            crate::failpoint!(
                self.failpoints,
                crate::failpoints::TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT
            );
            Ok(result)
        })
    }
}

pub struct EditFileTool {
    root: WorkspaceRoot,
    #[cfg(feature = "failpoints")]
    pub failpoints: crate::failpoints::Failpoints,
}

impl EditFileTool {
    pub fn new(root: WorkspaceRoot) -> Self {
        Self {
            root,
            #[cfg(feature = "failpoints")]
            failpoints: crate::failpoints::Failpoints::default(),
        }
    }
}

impl Tool for EditFileTool {
    type Request = EditFileRequest;
    type Output = FileChange;

    fn name(&self) -> &str {
        "edit_file"
    }

    fn has_side_effects(&self) -> bool {
        true
    }

    fn execute(
        &self,
        request: ToolRequest<EditFileRequest>,
    ) -> ToolFuture<'_, ToolResult<FileChange>> {
        let root = self.root.clone();
        Box::pin(async move {
            let result = blocking(request, move |input| {
                let path = root.resolve(&input.path)?;
                let before = path.read()?;
                let text = std::str::from_utf8(&before.content)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                let start = text
                    .find(&input.old_string)
                    .ok_or_else(|| ToolError::InvalidArguments("old string not found".into()))?;
                if text.rfind(&input.old_string) != Some(start) {
                    return Err(ToolError::InvalidArguments(
                        "old string is not unique".into(),
                    ));
                }
                let mut after = text.to_owned();
                after.replace_range(start..start + input.old_string.len(), &input.new_string);
                write(&path, Some(&before), after.as_bytes())
            })
            .await?;
            crate::failpoint!(
                self.failpoints,
                crate::failpoints::TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT
            );
            Ok(result)
        })
    }
}

fn write(
    path: &ResolvedPath,
    before: Option<&FileSnapshot>,
    after: &[u8],
) -> Result<FileChange, ToolError> {
    let diff = TextDiff::from_lines(
        before.map_or(b"".as_slice(), |snapshot| snapshot.content.as_slice()),
        after,
    );
    let mut lines_added = 0;
    let mut lines_removed = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => lines_added += 1,
            ChangeTag::Delete => lines_removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let change = FileChange {
        path: path.path.clone(),
        before_sha256: before.map(|snapshot| format!("{:x}", Sha256::digest(&snapshot.content))),
        after_sha256: format!("{:x}", Sha256::digest(after)),
        lines_added,
        lines_removed,
    };
    path.write(after, before)?;
    Ok(change)
}

pub struct ListDirTool {
    root: WorkspaceRoot,
}

impl ListDirTool {
    pub fn new(root: WorkspaceRoot) -> Self {
        Self { root }
    }
}

impl Tool for ListDirTool {
    type Request = PathBuf;
    type Output = Vec<DirectoryEntry>;

    fn name(&self) -> &str {
        "list_dir"
    }

    fn has_side_effects(&self) -> bool {
        false
    }

    fn execute(&self, request: ToolRequest<PathBuf>) -> ToolFuture<'_, ToolResult<Self::Output>> {
        let root = self.root.clone();
        Box::pin(blocking(request, move |path| {
            let path = root.resolve(&path)?;
            let directory = path
                .directory
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotADirectory))?;
            let scopes = [IgnoreScope::read(&directory, &path.path)?];
            let mut entries = Vec::new();
            for entry in directory.entries()? {
                let entry = entry?;
                let path = path.path.join(entry.file_name());
                let file_type = entry.file_type()?;
                if !is_ignored(&scopes, &path, file_type.is_dir()) {
                    entries.push(DirectoryEntry { path, file_type });
                }
            }
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(entries)
        }))
    }
}
