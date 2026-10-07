//! Checksum verification of model files.
//!
//! The service only opens a model whose size and SHA-256 match the manifest
//! (ADR 0024 §3: the GGUF parser is C++ with a history of overflow bugs, so
//! it only ever sees files we pinned). Hashing 0.5–1.3 GB takes seconds on
//! an old laptop, so after one full check a stamp of the file's identity
//! (size, modification time, inode) is kept beside it, and later loads
//! re-hash only when the identity changes.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::Model;
use crate::paths;

#[derive(Debug)]
pub enum VerifyError {
    Missing,
    WrongSize,
    WrongChecksum,
    Io(io::Error),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => formatter.write_str("the model is not downloaded"),
            Self::WrongSize => formatter.write_str("the model file has the wrong size"),
            Self::WrongChecksum => {
                formatter.write_str("the model file does not match its checksum")
            }
            Self::Io(error) => write!(formatter, "the model file could not be read: {error}"),
        }
    }
}

impl std::error::Error for VerifyError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Stamp {
    sha256: String,
    size: u64,
    modified_ns: i128,
    inode: u64,
}

fn identity(path: &Path, sha256: &str) -> io::Result<Stamp> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let modified_ns = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as i128)
        .unwrap_or(0);
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&metadata);
    #[cfg(not(unix))]
    let inode = 0;
    Ok(Stamp {
        sha256: sha256.to_owned(),
        size: metadata.len(),
        modified_ns,
        inode,
    })
}

/// SHA-256 of a file as lower-case hex, and its length.
pub fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut length = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        length += read as u64;
    }
    Ok((hex(&hasher.finalize()), length))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether the model is on disk, judged by its stamp alone (no hashing):
/// what Settings shows. A stale or missing stamp reads as not verified.
pub fn is_present(models: &Path, model: &Model) -> bool {
    let path = paths::model_file(models, model);
    let Ok(current) = identity(&path, model.sha256) else {
        return false;
    };
    read_stamp(&paths::stamp_file(models, model)).is_some_and(|stamp| stamp == current)
        && current.size == model.size
}

fn read_stamp(path: &Path) -> Option<Stamp> {
    let bytes = rmac_storage::read_bounded_no_follow(path, 4096).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The verified path of `model`, hashing it only if its stamp is stale.
pub fn verified_model(models: &Path, model: &Model) -> Result<PathBuf, VerifyError> {
    let path = paths::model_file(models, model);
    let current = match identity(&path, model.sha256) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(VerifyError::Missing),
        Err(error) => return Err(VerifyError::Io(error)),
    };
    if current.size != model.size {
        return Err(VerifyError::WrongSize);
    }
    let stamp_path = paths::stamp_file(models, model);
    if read_stamp(&stamp_path).is_some_and(|stamp| stamp == current) {
        return Ok(path);
    }
    let (sha256, size) = sha256_file(&path).map_err(VerifyError::Io)?;
    if size != model.size {
        return Err(VerifyError::WrongSize);
    }
    if sha256 != model.sha256 {
        return Err(VerifyError::WrongChecksum);
    }
    write_stamp(&stamp_path, &current);
    Ok(path)
}

/// Record a file that was just verified (the fetcher calls this after its
/// streaming hash), so the first load does not hash it again.
pub fn record_verified(models: &Path, model: &Model) -> io::Result<()> {
    let current = identity(&paths::model_file(models, model), model.sha256)?;
    write_stamp(&paths::stamp_file(models, model), &current);
    Ok(())
}

fn write_stamp(path: &Path, stamp: &Stamp) {
    if let Ok(bytes) = serde_json::to_vec(stamp) {
        // A stamp that cannot be written only costs a re-hash next time.
        let _ = rmac_storage::atomic_write_private(path, &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rmac-intelligence-verify-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn fake_model(bytes: &[u8]) -> Model {
        let (sha256, size) = {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            (hex(&hasher.finalize()), bytes.len() as u64)
        };
        Model {
            tier: crate::manifest::Tier::Tiny,
            display_name: "test",
            url: "https://example.invalid/model.gguf",
            size,
            sha256: Box::leak(sha256.into_boxed_str()),
            licence: "MIT",
            memory_budget_mib: 1,
        }
    }

    #[test]
    fn only_the_pinned_bytes_verify_and_the_stamp_skips_rehashing() {
        let models = scratch("pinned");
        let model = fake_model(b"GGUF model bytes");
        assert!(matches!(
            verified_model(&models, &model),
            Err(VerifyError::Missing)
        ));
        let path = paths::model_file(&models, &model);
        std::fs::write(&path, b"GGUF model bytez").unwrap();
        assert!(matches!(
            verified_model(&models, &model),
            Err(VerifyError::WrongChecksum)
        ));
        assert!(!is_present(&models, &model));
        std::fs::write(&path, b"GGUF model").unwrap();
        assert!(matches!(
            verified_model(&models, &model),
            Err(VerifyError::WrongSize)
        ));
        std::fs::write(&path, b"GGUF model bytes").unwrap();
        assert_eq!(verified_model(&models, &model).unwrap(), path);
        assert!(is_present(&models, &model));
        // A replaced file (new identity) is hashed again, and fails.
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"GGUF model bytez").unwrap();
        assert!(matches!(
            verified_model(&models, &model),
            Err(VerifyError::WrongChecksum)
        ));
        std::fs::remove_dir_all(&models).unwrap();
    }
}
