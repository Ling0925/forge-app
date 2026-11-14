use crate::{
    Tool, ToolError, ToolFuture, ToolRequest, ToolResult, blocking, workspace::WorkspaceRoot,
};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

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
}

#[derive(Debug)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub file_type: fs::FileType,
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
            let content = fs::read_to_string(root.resolve(&input.path)?)?;
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
                let before = match fs::read(&path) {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                };
                fs::create_dir_all(path.parent().expect("file path has a parent"))?;
                write(&root, &path, before.as_deref(), input.content.as_bytes())
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
                let before = fs::read_to_string(&path)?;
                let start = before
                    .find(&input.old_string)
                    .ok_or_else(|| ToolError::InvalidArguments("old string not found".into()))?;
                if before.rfind(&input.old_string) != Some(start) {
                    return Err(ToolError::InvalidArguments(
                        "old string is not unique".into(),
                    ));
                }
                let mut after = before.clone();
                after.replace_range(start..start + input.old_string.len(), &input.new_string);
                write(&root, &path, Some(before.as_bytes()), after.as_bytes())
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
    root: &WorkspaceRoot,
    path: &Path,
    before: Option<&[u8]>,
    after: &[u8],
) -> Result<FileChange, ToolError> {
    let change = FileChange {
        path: root.relative(path),
        before_sha256: before.map(|content| format!("{:x}", Sha256::digest(content))),
        after_sha256: format!("{:x}", Sha256::digest(after)),
    };
    fs::write(path, after)?;
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
            let mut entries = Vec::new();
            for entry in fs::read_dir(root.resolve(&path)?)? {
                let entry = entry?;
                entries.push(DirectoryEntry {
                    path: root.relative(&entry.path()),
                    file_type: entry.file_type()?,
                });
            }
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(entries)
        }))
    }
}
