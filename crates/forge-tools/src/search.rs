//! Searches use in-tree ignore rules and skip hidden entries; traversal does not follow symlinks.

use crate::{
    Tool, ToolError, ToolFuture, ToolRequest, ToolResult, blocking,
    cancellation::CancellationToken,
    workspace::{ResolvedPath, WorkspaceRoot, open_file},
};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use globset::GlobBuilder;
use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};
use regex::bytes::Regex;
use std::{
    ffi::OsStr,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    sync::Arc,
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
                let relative = &path.path;
                if pattern.is_match(relative) {
                    matches.push(relative.clone());
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
                let relative = &path.path;
                let mut reader = BufReader::new(path.open()?);
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
    mut visit: impl FnMut(&ResolvedPath) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    cancellation.check()?;
    let path = root.resolve(path)?;
    if let Some(directory) = &path.directory {
        walk(
            directory,
            &path.path,
            &mut Vec::new(),
            cancellation,
            &mut visit,
        )?;
    } else {
        visit(&path)?;
    }
    cancellation.check()
}

fn walk(
    directory: &Arc<Dir>,
    path: &Path,
    scopes: &mut Vec<IgnoreScope>,
    cancellation: &CancellationToken,
    visit: &mut impl FnMut(&ResolvedPath) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    cancellation.check()?;
    scopes.push(IgnoreScope::read(directory, path)?);
    let mut entries = directory.entries()?;
    loop {
        cancellation.check()?;
        let Some(entry) = entries.next() else {
            break;
        };
        let entry = entry?;
        let name = entry.file_name();
        let path = path.join(&name);
        let kind = entry.file_type()?;
        if kind.is_symlink() || is_ignored(scopes, &path, kind.is_dir()) {
            continue;
        }
        if kind.is_dir() {
            let child = Arc::new(directory.open_dir_nofollow(&name)?);
            walk(&child, &path, scopes, cancellation, visit)?;
        } else if kind.is_file() {
            visit(&ResolvedPath::file(directory.clone(), name, path))?;
        }
    }
    scopes.pop();
    Ok(())
}

pub(crate) struct IgnoreScope {
    ignore: Gitignore,
    gitignore: Gitignore,
    exclude: Gitignore,
    repository: bool,
}

impl IgnoreScope {
    pub(crate) fn read(directory: &Dir, path: &Path) -> io::Result<Self> {
        let git = match directory.symlink_metadata(".git") {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let exclude = if git.as_ref().is_some_and(|metadata| metadata.is_dir()) {
            let git = directory.open_dir_nofollow(".git")?;
            match git.open_dir_nofollow("info") {
                Ok(info) => read_ignore(&info, path, "exclude")?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Gitignore::empty(),
                Err(error) => return Err(error),
            }
        } else {
            Gitignore::empty()
        };
        Ok(Self {
            ignore: read_ignore(directory, path, ".ignore")?,
            gitignore: read_ignore(directory, path, ".gitignore")?,
            exclude,
            repository: git.is_some(),
        })
    }
}

fn read_ignore(directory: &Dir, path: &Path, name: &str) -> io::Result<Gitignore> {
    let file = match open_file(directory, OsStr::new(name)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Gitignore::empty()),
        Err(error) => return Err(error),
    };
    let mut builder = GitignoreBuilder::new(path);
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = if index == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            &line
        };
        builder
            .add_line(Some(path.join(name)), line)
            .map_err(io::Error::other)?;
    }
    builder.build().map_err(io::Error::other)
}

pub(crate) fn is_ignored(scopes: &[IgnoreScope], path: &Path, is_dir: bool) -> bool {
    let (mut ignore, mut gitignore, mut exclude) = (Match::None, Match::None, Match::None);
    let mut repository = false;
    // Directory-handle scopes implement parents(false), require_git(false), git_global(false).
    for scope in scopes.iter().rev() {
        ignore = ignore.or(scope.ignore.matched(path, is_dir));
        if !repository {
            gitignore = gitignore.or(scope.gitignore.matched(path, is_dir));
            exclude = exclude.or(scope.exclude.matched(path, is_dir));
        }
        repository |= scope.repository;
    }
    let matched = ignore.or(gitignore).or(exclude);
    matched.is_ignore()
        || (matched.is_none()
            && path
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b".")))
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
