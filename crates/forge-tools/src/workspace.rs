use crate::ToolError;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions, Permissions};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct WorkspaceRoot {
    root: PathBuf,
    supplied: PathBuf,
    directory: Arc<Dir>,
}

pub(crate) struct ResolvedPath {
    pub path: PathBuf,
    pub directory: Option<Arc<Dir>>,
    parent: Arc<Dir>,
    name: OsString,
    missing_parents: Vec<OsString>,
}

pub(crate) struct FileSnapshot {
    pub content: Vec<u8>,
    permissions: Permissions,
}

enum Node {
    Directory(Arc<Dir>),
    File,
    Missing,
}

impl WorkspaceRoot {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ToolError> {
        let supplied = std::path::absolute(path)?;
        let root = fs::canonicalize(&supplied)?;
        let directory = Arc::new(Dir::open_ambient_dir(&root, cap_std::ambient_authority())?);
        Ok(Self {
            root,
            supplied,
            directory,
        })
    }

    pub(crate) fn resolve(&self, path: &Path) -> Result<ResolvedPath, ToolError> {
        self.resolve_path(path, false)
    }

    pub(crate) fn resolve_write(&self, path: &Path) -> Result<ResolvedPath, ToolError> {
        self.resolve_path(path, true)
    }

    fn relative<'a>(&self, path: &'a Path) -> Result<&'a Path, ToolError> {
        if path.is_absolute() {
            path.strip_prefix(&self.root)
                .or_else(|_| path.strip_prefix(&self.supplied))
                .map_err(|_| Self::escape(path))
        } else {
            Ok(path)
        }
    }

    fn escape(path: &Path) -> ToolError {
        ToolError::InvalidArguments(format!("path escapes workspace root: {}", path.display()))
    }

    fn resolve_path(&self, path: &Path, allow_missing: bool) -> Result<ResolvedPath, ToolError> {
        let mut nodes = vec![(OsString::new(), Node::Directory(self.directory.clone()))];
        self.walk(&mut nodes, path, allow_missing, &mut Vec::new())?;
        let path = nodes.iter().skip(1).map(|(name, _)| name).collect();
        if nodes.len() == 1 {
            return Ok(ResolvedPath {
                path,
                directory: Some(self.directory.clone()),
                parent: self.directory.clone(),
                name: ".".into(),
                missing_parents: Vec::new(),
            });
        }
        let (name, node) = nodes.pop().unwrap();
        let directory = match node {
            Node::Directory(dir) => Some(dir),
            _ => None,
        };
        let mut missing_parents = Vec::new();
        let parent = loop {
            let (name, node) = nodes.pop().unwrap();
            if let Node::Directory(dir) = node {
                break dir;
            }
            missing_parents.push(name);
        };
        missing_parents.reverse();
        Ok(ResolvedPath {
            path,
            directory,
            parent,
            name,
            missing_parents,
        })
    }

    fn walk(
        &self,
        nodes: &mut Vec<(OsString, Node)>,
        path: &Path,
        allow_missing: bool,
        symlinks: &mut Vec<PathBuf>,
    ) -> Result<(), ToolError> {
        let relative = self.relative(path)?;
        if path.is_absolute() {
            nodes.truncate(1);
        }
        for component in relative.components() {
            let parent = match &nodes.last().unwrap().1 {
                Node::Directory(dir) => Some(dir.clone()),
                Node::Missing => None,
                Node::File => return Err(io::Error::from(io::ErrorKind::NotADirectory).into()),
            };
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if nodes.len() == 1 {
                        return Err(Self::escape(path));
                    }
                    nodes.pop();
                }
                Component::Normal(name) => {
                    let metadata = match &parent {
                        Some(dir) => dir.symlink_metadata(name),
                        None => Err(io::ErrorKind::NotFound.into()),
                    };
                    match metadata {
                        Ok(metadata) if metadata.is_symlink() => {
                            let link: PathBuf = nodes
                                .iter()
                                .skip(1)
                                .map(|(name, _)| name.as_os_str())
                                .chain(std::iter::once(name))
                                .collect();
                            if symlinks.contains(&link) {
                                return Err(io::Error::other(format!(
                                    "symlink loop: {}",
                                    link.display()
                                ))
                                .into());
                            }
                            let target = parent.as_ref().unwrap().read_link_contents(name)?;
                            symlinks.push(link);
                            self.walk(nodes, &target, allow_missing, symlinks)?;
                            symlinks.pop();
                        }
                        Ok(metadata) => {
                            let node = if metadata.is_dir() {
                                Node::Directory(Arc::new(
                                    parent.as_ref().unwrap().open_dir_nofollow(name)?,
                                ))
                            } else {
                                Node::File
                            };
                            nodes.push((name.to_owned(), node));
                        }
                        Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => {
                            nodes.push((name.to_owned(), Node::Missing));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                _ => return Err(Self::escape(path)),
            }
        }
        Ok(())
    }
}

pub(crate) fn open_file(dir: &Dir, name: &OsStr) -> io::Result<File> {
    dir.open_with(
        name,
        OpenOptions::new().read(true).follow(FollowSymlinks::No),
    )
}

impl ResolvedPath {
    pub(crate) fn file(parent: Arc<Dir>, name: OsString, path: PathBuf) -> Self {
        Self {
            path,
            directory: None,
            parent,
            name,
            missing_parents: Vec::new(),
        }
    }

    pub(crate) fn open(&self) -> io::Result<File> {
        if !self.missing_parents.is_empty() {
            return Err(io::ErrorKind::NotFound.into());
        }
        open_file(&self.parent, &self.name)
    }

    pub(crate) fn read(&self) -> io::Result<FileSnapshot> {
        let mut file = self.open()?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)?;
        Ok(FileSnapshot {
            content,
            permissions: file.metadata()?.permissions(),
        })
    }

    pub(crate) fn write(&self, content: &[u8], before: Option<&FileSnapshot>) -> io::Result<()> {
        let mut parent = self.parent.clone();
        for name in &self.missing_parents {
            match parent.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            parent = Arc::new(parent.open_dir_nofollow(name)?);
        }
        let mut file = cap_tempfile::TempFile::new(&parent)?;
        file.write_all(content)?;
        if let Some(before) = before {
            file.as_file().set_permissions(before.permissions.clone())?;
        }
        file.as_file().sync_all()?;
        file.replace(&self.name)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn failed_atomic_replacement_removes_the_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = WorkspaceRoot::new(directory.path()).unwrap();
        let target = root.resolve_write(Path::new("target")).unwrap();
        fs::create_dir(directory.path().join("target")).unwrap();
        fs::write(directory.path().join("target/kept"), "original").unwrap();
        assert!(target.write(b"replacement", None).is_err());
        let entries = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, [OsString::from("target")]);
        assert_eq!(
            fs::read(directory.path().join("target/kept")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn resolved_parent_handles_survive_a_directory_swap() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("root/d")).unwrap();
        fs::create_dir(directory.path().join("outside")).unwrap();
        fs::write(directory.path().join("root/d/file"), "inside").unwrap();
        fs::write(directory.path().join("outside/file"), "outside").unwrap();
        let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
        let read = root.resolve(Path::new("d/file")).unwrap();
        let write = root.resolve_write(Path::new("d/nested/new")).unwrap();
        fs::rename(
            directory.path().join("root/d"),
            directory.path().join("root/held"),
        )
        .unwrap();
        symlink("../outside", directory.path().join("root/d")).unwrap();
        assert_eq!(read.read().unwrap().content, b"inside");
        write.write(b"created", None).unwrap();
        assert_eq!(
            fs::read(directory.path().join("root/held/nested/new")).unwrap(),
            b"created"
        );
        assert!(!directory.path().join("outside/nested").exists());
    }

    #[test]
    fn final_symlink_swap_cannot_redirect_reads_or_atomic_replacement() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        fs::write(directory.path().join("root/file"), "inside").unwrap();
        fs::write(directory.path().join("outside"), "outside").unwrap();
        fs::set_permissions(
            directory.path().join("root/file"),
            fs::Permissions::from_mode(0o751),
        )
        .unwrap();
        let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
        let file = root.resolve(Path::new("file")).unwrap();
        let before = file.read().unwrap();
        fs::remove_file(directory.path().join("root/file")).unwrap();
        symlink("../outside", directory.path().join("root/file")).unwrap();
        assert!(file.open().is_err());
        file.write(b"replaced", Some(&before)).unwrap();
        assert_eq!(
            fs::read(directory.path().join("outside")).unwrap(),
            b"outside"
        );
        assert_eq!(
            fs::read(directory.path().join("root/file")).unwrap(),
            b"replaced"
        );
        assert_eq!(
            fs::metadata(directory.path().join("root/file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o751
        );
    }
}
