//! Content-addressed blob store.
//!
//! Records hold hashes; the bytes live here. Layout:
//!
//! ```text
//! <root>/blobs/<2-hex-shard>/<64-hex>.bin
//! ```
//!
//! Files are written to a temporary name and renamed into place, so a crashed
//! write cannot leave a truncated blob readable at its content id.
//!
//! # Large artifacts
//!
//! Model checkpoints are gigabytes, so hashing and copying **stream** through a
//! fixed buffer instead of loading the artifact into memory.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::id::BlobId;
use crate::record::BlobRef;

/// Streaming buffer size (1 MiB).
const BUF_LEN: usize = 1024 * 1024;

/// A filesystem-backed content-addressed store.
#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// Open (creating if needed) a store rooted at `dir/blobs`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let root = data_dir.as_ref().join("blobs");
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// The directory a blob lives in, given its id.
    pub fn path_for(&self, id: &BlobId) -> PathBuf {
        self.root
            .join(id.shard())
            .join(format!("{}.bin", id.to_hex()))
    }

    /// True if the bytes for `id` are present locally.
    pub fn contains(&self, id: &BlobId) -> bool {
        self.path_for(id).is_file()
    }

    /// Verify stored content without buffering the artifact; return its size.
    pub fn verify(&self, id: &BlobId) -> Result<u64> {
        let mut reader = self.open_reader(id)?;
        let mut hasher = blake3::Hasher::new();
        let size = std::io::copy(&mut reader, &mut hasher)?;
        let actual = BlobId::from_digest(*hasher.finalize().as_bytes());
        if actual != *id {
            return Err(Error::BlobIntegrity {
                expected: id.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(size)
    }

    /// Store `bytes`, returning their content id.
    pub fn put_bytes(&self, bytes: &[u8]) -> Result<BlobId> {
        let id = BlobId::from_bytes(bytes);
        self.write(&id, &mut BufReader::new(bytes))?;
        Ok(id)
    }

    /// Store `bytes` as a named attachment.
    pub fn put_named(
        &self,
        name: impl Into<String>,
        bytes: &[u8],
        media: Option<String>,
    ) -> Result<BlobRef> {
        let id = self.put_bytes(bytes)?;
        Ok(BlobRef {
            name: name.into(),
            blob: id,
            size: bytes.len() as u64,
            media,
        })
    }

    /// Stream a file from `src` into the store without loading it into memory.
    ///
    /// Hashing and copying happen in a single pass.
    pub fn put_file(
        &self,
        name: impl Into<String>,
        src: impl AsRef<Path>,
        media: Option<String>,
    ) -> Result<BlobRef> {
        let src = src.as_ref();
        let file = File::open(src)?;
        let mut reader = BufReader::with_capacity(BUF_LEN, file);
        let mut staging = self.staging_file()?;
        let mut size = 0u64;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; BUF_LEN];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            staging.write_all(&buf[..n])?;
            size += n as u64;
        }
        let id = BlobId::from_digest(*hasher.finalize().as_bytes());

        self.persist(staging, &id)?;

        Ok(BlobRef {
            name: name.into(),
            blob: id,
            size,
            media,
        })
    }

    /// A temporary file on the store's own volume, for callers that receive
    /// bytes incrementally. Hand it back through [`BlobStore::put_staged`].
    pub fn staging_file(&self) -> Result<tempfile::NamedTempFile> {
        Ok(tempfile::NamedTempFile::new_in(&self.root)?)
    }

    /// Hash a file written through [`BlobStore::staging_file`] and move it into
    /// place. The bytes are read once and never copied.
    pub fn put_staged(
        &self,
        staging: tempfile::NamedTempFile,
        name: impl Into<String>,
        media: Option<String>,
    ) -> Result<BlobRef> {
        let mut reader = BufReader::with_capacity(BUF_LEN, staging.reopen()?);
        let mut hasher = blake3::Hasher::new();
        let size = std::io::copy(&mut reader, &mut hasher)?;
        let id = BlobId::from_digest(*hasher.finalize().as_bytes());
        self.persist(staging, &id)?;
        Ok(BlobRef {
            name: name.into(),
            blob: id,
            size,
            media,
        })
    }

    /// Read the bytes for `id`.
    pub fn get(&self, id: &BlobId) -> Result<Vec<u8>> {
        let path = self.path_for(id);
        match fs::read(&path) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::BlobNotFound(id.to_string()))
            }
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Open a streaming reader for `id`, for artifacts too large to buffer.
    pub fn open_reader(&self, id: &BlobId) -> Result<File> {
        let path = self.path_for(id);
        match File::open(&path) {
            Ok(f) => Ok(f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::BlobNotFound(id.to_string()))
            }
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Write to a temp file then rename, so readers never see partial content.
    fn write<R: Read>(&self, id: &BlobId, reader: &mut R) -> Result<()> {
        if self.contains(id) {
            self.verify(id)?;
            return Ok(());
        }
        let mut staging = tempfile::NamedTempFile::new_in(&self.root)?;
        std::io::copy(reader, &mut staging)?;
        self.persist(staging, id)
    }

    fn persist(&self, staging: tempfile::NamedTempFile, id: &BlobId) -> Result<()> {
        let final_path = self.path_for(id);
        fs::create_dir_all(final_path.parent().expect("blob path has parent"))?;
        staging.as_file().sync_all()?;
        match staging.persist_noclobber(final_path) {
            Ok(_) => Ok(()),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.verify(id)?;
                Ok(())
            }
            Err(e) => Err(Error::Io(e.error)),
        }
    }

    /// Number of blobs stored locally.
    pub fn count(&self) -> Result<usize> {
        let mut n = 0;
        for shard in fs::read_dir(&self.root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(shard.path())? {
                if entry?.path().extension().is_some_and(|e| e == "bin") {
                    n += 1;
                }
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = BlobStore::open(dir.path()).unwrap();
        (dir, s)
    }

    #[test]
    fn put_and_get_round_trip() {
        let (_d, s) = store();
        let id = s.put_bytes(b"weights").unwrap();
        assert!(s.contains(&id));
        assert_eq!(s.get(&id).unwrap(), b"weights");
    }

    #[test]
    fn content_addressing_is_deterministic() {
        let (_d, s) = store();
        assert_eq!(s.put_bytes(b"same").unwrap(), s.put_bytes(b"same").unwrap());
        assert_ne!(s.put_bytes(b"a").unwrap(), s.put_bytes(b"b").unwrap());
    }

    #[test]
    fn path_uses_shard_layout() {
        let (_d, s) = store();
        let id = s.put_bytes(b"x").unwrap();
        let p = s.path_for(&id);
        assert!(p.to_string_lossy().contains("blobs"));
        assert_eq!(p.extension().unwrap(), "bin");
        assert_eq!(
            p.parent().unwrap().file_name().unwrap().to_string_lossy(),
            id.shard()
        );
    }

    #[test]
    fn missing_blob_reports_not_found() {
        let (_d, s) = store();
        let id = BlobId::from_bytes(b"never-stored");
        assert!(!s.contains(&id));
        assert!(matches!(s.get(&id), Err(Error::BlobNotFound(_))));
        assert!(matches!(s.open_reader(&id), Err(Error::BlobNotFound(_))));
    }

    #[test]
    fn importing_existing_content_detects_corruption() {
        let (_dir, store) = store();
        let id = store.put_bytes(b"original").unwrap();
        fs::write(store.path_for(&id), b"modified").unwrap();
        assert!(matches!(
            store.put_bytes(b"original"),
            Err(Error::BlobIntegrity { .. })
        ));
    }

    #[test]
    fn file_streaming_matches_in_memory_hash() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("artifact.bin");
        let data: Vec<u8> = (0..(3 * BUF_LEN + 17)).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &data).unwrap();

        let (_d, s) = store();
        let r = s
            .put_file("checkpoint", &src, Some("application/octet-stream".into()))
            .unwrap();
        assert_eq!(r.size, data.len() as u64);
        assert_eq!(r.blob, BlobId::from_bytes(&data));
        assert_eq!(s.get(&r.blob).unwrap(), data);
    }

    #[test]
    fn staged_bytes_are_hashed_and_moved_into_place() {
        let (_d, s) = store();
        let data: Vec<u8> = (0..(2 * BUF_LEN + 5)).map(|i| (i % 199) as u8).collect();
        let mut staging = s.staging_file().unwrap();
        staging.write_all(&data).unwrap();
        let r = s.put_staged(staging, "checkpoint", None).unwrap();
        assert_eq!(r.size, data.len() as u64);
        assert_eq!(r.blob, BlobId::from_bytes(&data));
        assert_eq!(s.get(&r.blob).unwrap(), data);
        assert_eq!(s.count().unwrap(), 1);
        assert!(walkdir(&s.root).len() == 1, "staging file was not removed");
    }

    #[test]
    fn count_reflects_stored_blobs() {
        let (_d, s) = store();
        assert_eq!(s.count().unwrap(), 0);
        s.put_bytes(b"a").unwrap();
        s.put_bytes(b"b").unwrap();
        s.put_bytes(b"a").unwrap();
        assert_eq!(s.count().unwrap(), 2);
    }

    #[test]
    fn no_tmp_files_remain() {
        let (_d, s) = store();
        s.put_bytes(b"data").unwrap();
        let leftovers: Vec<_> = walkdir(&s.root);
        assert!(leftovers
            .iter()
            .all(|p| !p.to_string_lossy().ends_with(".tmp")));
    }

    fn walkdir(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(rd) = fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        out.push(p);
                    }
                }
            }
        }
        out
    }
}
