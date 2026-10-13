//! The on-disk part of an investigation: snapshots, evidence files, agent context packs,
//! reports, and repository clones. Paths stored in the database are relative to the
//! investigation directory so investigations can be exported and moved.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::paths::{AppPaths, is_safe_relative_path, sanitize_filename};

const HASH_BUFFER_SIZE: usize = 64 * 1024;
const SHARD_PREFIX_LENGTH: usize = 2;

/// A blob written to the investigation directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredBlob {
    /// Relative to the investigation directory.
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct InvestigationFiles {
    root: PathBuf,
}

impl InvestigationFiles {
    pub fn new(paths: &AppPaths, investigation_id: Uuid) -> Self {
        Self {
            root: paths.investigation_dir(investigation_id),
        }
    }

    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)
    }

    /// Resolves a stored relative path, refusing anything that could escape the directory.
    pub fn resolve(&self, relative: &str) -> Option<PathBuf> {
        is_safe_relative_path(relative).then(|| self.root.join(relative))
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.root.join("repos")
    }

    /// Stores bytes content-addressed under `snapshots/`, writing each distinct content once.
    pub fn store_snapshot(&self, bytes: &[u8], extension: &str) -> io::Result<StoredBlob> {
        let sha256 = sha256_hex(bytes);
        let extension = sanitize_filename(extension);
        let relative_path = format!(
            "snapshots/{}/{sha256}.{extension}",
            &sha256[..SHARD_PREFIX_LENGTH]
        );
        let path = self.root.join(&relative_path);
        if !path.exists() {
            write_atomically(&path, bytes)?;
        }
        Ok(StoredBlob {
            relative_path,
            sha256,
            size_bytes: bytes.len() as u64,
        })
    }

    /// Copies an evidence file into `files/`, content-addressed, refusing files above `max_bytes`.
    pub fn import_file(&self, source: &Path, max_bytes: u64) -> io::Result<StoredBlob> {
        let metadata = fs::metadata(source)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
        if metadata.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "file is {} bytes, above the import limit of {max_bytes} bytes (storage.max_file_import_bytes)",
                    metadata.len()
                ),
            ));
        }
        let (sha256, size_bytes) = sha256_file(source)?;
        let name = source
            .file_name()
            .map(|name| sanitize_filename(&name.to_string_lossy()))
            .unwrap_or_else(|| "evidence".to_string());
        let relative_path = format!("files/{}/{sha256}-{name}", &sha256[..SHARD_PREFIX_LENGTH]);
        let destination = self.root.join(&relative_path);
        if !destination.exists() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source, &destination)?;
        }
        Ok(StoredBlob {
            relative_path,
            sha256,
            size_bytes,
        })
    }

    /// Writes a text file into a subdirectory (`agent`, `reports`, ...), returning its blob.
    pub fn write_text(
        &self,
        directory: &str,
        file_name: &str,
        text: &str,
    ) -> io::Result<StoredBlob> {
        let relative_path = format!(
            "{}/{}",
            sanitize_filename(directory),
            sanitize_filename(file_name)
        );
        write_atomically(&self.root.join(&relative_path), text.as_bytes())?;
        Ok(StoredBlob {
            relative_path,
            sha256: sha256_hex(text.as_bytes()),
            size_bytes: text.len() as u64,
        })
    }

    /// Reads a stored file as text, at most `max_bytes`.
    pub fn read_text(&self, relative: &str, max_bytes: usize) -> io::Result<String> {
        let path = self
            .resolve(relative)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        let mut buffer = Vec::new();
        fs::File::open(path)?
            .take(max_bytes as u64)
            .read_to_end(&mut buffer)?;
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Streams a file through SHA-256. Returns `(hex digest, size)`.
pub fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_SIZE];
    let mut total: u64 = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

/// Writes to a temporary sibling file and renames it into place.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_content_addressed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let files = InvestigationFiles::at(directory.path().to_path_buf());
        let first = files.store_snapshot(b"hello", "html").expect("stores");
        let second = files
            .store_snapshot(b"hello", "html")
            .expect("stores again");
        assert_eq!(first, second);
        assert_eq!(
            first.sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert!(first.relative_path.starts_with("snapshots/2c/"));
        assert!(files.resolve(&first.relative_path).expect("safe").exists());
    }

    #[test]
    fn resolve_rejects_escaping_paths() {
        let files = InvestigationFiles::at(PathBuf::from("/tmp/investigation"));
        assert!(files.resolve("../../etc/passwd").is_none());
        assert!(files.resolve("/etc/passwd").is_none());
        assert!(files.resolve("reports/report.md").is_some());
    }

    #[test]
    fn import_file_respects_size_limit() {
        let directory = tempfile::tempdir().expect("tempdir");
        let evidence = directory.path().join("Evidence File.txt");
        fs::write(&evidence, b"0123456789").expect("writes");
        let files = InvestigationFiles::at(directory.path().join("investigation"));
        assert!(files.import_file(&evidence, 5).is_err());
        let stored = files.import_file(&evidence, 1024).expect("imports");
        assert_eq!(stored.size_bytes, 10);
        assert!(stored.relative_path.ends_with("-evidence-file.txt"));
    }

    #[test]
    fn write_and_read_text() {
        let directory = tempfile::tempdir().expect("tempdir");
        let files = InvestigationFiles::at(directory.path().to_path_buf());
        let blob = files
            .write_text("reports", "Report 1.md", "# Report")
            .expect("writes");
        assert_eq!(blob.relative_path, "reports/report-1.md");
        assert_eq!(
            files.read_text(&blob.relative_path, 3).expect("reads"),
            "# R"
        );
    }
}
