use forge_protocol::BlobHash;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub struct BlobStore {
    directory: PathBuf,
}

impl BlobStore {
    pub fn open(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        Ok(Self { directory })
    }

    pub fn write(&self, content: &[u8]) -> io::Result<BlobHash> {
        let hash = BlobHash(format!("{:x}", Sha256::digest(content)));
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        file.write_all(content)?;
        match file.persist_noclobber(self.path(&hash)) {
            Ok(_) => Ok(hash),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(hash),
            Err(error) => Err(error.error),
        }
    }

    pub fn read(&self, hash: &BlobHash) -> io::Result<Vec<u8>> {
        fs::read(self.path(hash))
    }

    fn path(&self, hash: &BlobHash) -> PathBuf {
        Path::new(&self.directory).join(&hash.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_content_addressed_deduplication() {
        let directory = tempfile::tempdir().unwrap();
        let blobs = BlobStore::open(directory.path()).unwrap();
        let hash = blobs.write(b"hello").unwrap();
        assert_eq!(
            hash.0,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(blobs.read(&hash).unwrap(), b"hello");
        assert_eq!(blobs.write(b"hello").unwrap(), hash);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_ne!(blobs.write(b"world").unwrap(), hash);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn empty_blob_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let blobs = BlobStore::open(directory.path()).unwrap();
        let hash = blobs.write(b"").unwrap();
        assert!(blobs.read(&hash).unwrap().is_empty());
    }
}
