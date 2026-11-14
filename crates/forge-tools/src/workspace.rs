use crate::ToolError;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct WorkspaceRoot {
    root: PathBuf,
    supplied: PathBuf,
}

impl WorkspaceRoot {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ToolError> {
        let supplied = std::path::absolute(path)?;
        let root = fs::canonicalize(&supplied)?;
        if !fs::metadata(&root)?.is_dir() {
            return Err(ToolError::InvalidArguments(
                "workspace root must be a directory".into(),
            ));
        }
        Ok(Self { root, supplied })
    }

    pub(crate) fn resolve(&self, path: &Path) -> Result<PathBuf, ToolError> {
        self.resolve_path(path, false, &mut Vec::new())
    }

    pub(crate) fn resolve_write(&self, path: &Path) -> Result<PathBuf, ToolError> {
        self.resolve_path(path, true, &mut Vec::new())
    }

    pub(crate) fn relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .expect("resolved path is inside workspace")
            .to_owned()
    }

    fn resolve_path(
        &self,
        path: &Path,
        allow_missing: bool,
        symlinks: &mut Vec<PathBuf>,
    ) -> Result<PathBuf, ToolError> {
        let escape = || {
            ToolError::InvalidArguments(format!("path escapes workspace root: {}", path.display()))
        };
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.root)
                .or_else(|_| path.strip_prefix(&self.supplied))
                .map_err(|_| escape())?
        } else {
            path
        };
        let mut resolved = self.root.clone();
        for component in relative.components() {
            if component == Component::ParentDir && resolved == self.root {
                return Err(escape());
            }
            let candidate = resolved.join(component);
            if !candidate.starts_with(&self.root) {
                return Err(escape());
            }
            if allow_missing {
                match fs::symlink_metadata(&candidate) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        if component == Component::ParentDir {
                            resolved.pop();
                        } else {
                            resolved = candidate;
                        }
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                    Ok(_) => {}
                }
            }
            resolved = match fs::canonicalize(&candidate) {
                Ok(path) => path,
                Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => {
                    let target = fs::read_link(&candidate)?;
                    if symlinks.contains(&candidate) {
                        return Err(io::Error::other(format!(
                            "symlink loop: {}",
                            candidate.display()
                        ))
                        .into());
                    }
                    symlinks.push(candidate);
                    let target = self.resolve_path(&resolved.join(target), true, symlinks)?;
                    symlinks.pop();
                    target
                }
                Err(error) => return Err(error.into()),
            };
            if !resolved.starts_with(&self.root) {
                return Err(escape());
            }
        }
        Ok(resolved)
    }
}
