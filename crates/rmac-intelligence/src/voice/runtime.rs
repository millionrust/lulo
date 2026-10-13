//! ONNX Runtime for the voice model, downloaded with it.
//!
//! The voice service needs `libonnxruntime.so` (1.23 or later). Ubuntu has
//! it in universe (`libonnxruntime1.23`, which `rmac-session` recommends),
//! but a Recommends is not always installed (`--no-install-recommends`, or
//! an update that adds one), and Lulo never asks the user to install a
//! package by hand. So "Download" in Settings ▸ Lulo Intelligence ▸ Talk to
//! Lulo also fetches Microsoft's official release (MIT), pinned by the
//! archive's SHA-256, unpacks the one shared library it needs with the
//! system `tar`, checks that library against its own pinned SHA-256 and
//! keeps it in the models folder as `<sha256>.so`, like every other pinned
//! file. The service loads only that verified copy (or an explicit
//! `ORT_DYLIB_PATH`); a system copy is the fallback where this CPU has no
//! pinned build.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::fetch::FetchError;
use crate::manifest::Pinned;
use crate::{paths, verify};

/// The shared library inside an official ONNX Runtime release archive.
#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeLibrary {
    pub version: &'static str,
    /// Revision-pinned: a release asset of a tagged version.
    pub archive_url: &'static str,
    pub archive_size: u64,
    pub archive_sha256: &'static str,
    /// The library's path inside the archive.
    pub member: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// x86-64: `onnxruntime-linux-x64-1.23.2.tgz`.
pub const X86_64: RuntimeLibrary = RuntimeLibrary {
    version: "1.23.2",
    archive_url: "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-x64-1.23.2.tgz",
    archive_size: 8_309_231,
    archive_sha256: "1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b",
    member: "onnxruntime-linux-x64-1.23.2/lib/libonnxruntime.so.1.23.2",
    size: 22_326_072,
    sha256: "13ab8084954fa4a47c777880180b90810d6020f021441395712b48a75b74c68b",
};

/// 64-bit ARM: `onnxruntime-linux-aarch64-1.23.2.tgz`.
pub const AARCH64: RuntimeLibrary = RuntimeLibrary {
    version: "1.23.2",
    archive_url: "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-aarch64-1.23.2.tgz",
    archive_size: 7_254_068,
    archive_sha256: "7c63c73560ed76b1fac6cff8204ffe34fe180e70d6582b5332ec094810241e5c",
    member: "onnxruntime-linux-aarch64-1.23.2/lib/libonnxruntime.so.1.23.2",
    size: 18_693_384,
    sha256: "648ffa64fbe027ae27139109410900cf776a030dec2dbbac51053318cc44c286",
};

/// The pinned build for this CPU, if there is one.
pub fn for_this_cpu() -> Option<&'static RuntimeLibrary> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(&X86_64)
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some(&AARCH64)
    } else {
        None
    }
}

/// The unpacked library, as a pinned file in the models folder.
impl Pinned for RuntimeLibrary {
    fn url(&self) -> &str {
        self.archive_url
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn sha256(&self) -> &str {
        self.sha256
    }
    fn extension(&self) -> &str {
        "so"
    }
    fn display_name(&self) -> &str {
        "ONNX Runtime (Microsoft)"
    }
    fn licence(&self) -> &str {
        "MIT"
    }
}

/// The release archive, while it downloads.
pub struct Archive(pub &'static RuntimeLibrary);

// The archive's own URL, size and hash: deliberately not the library's.
#[allow(clippy::misnamed_getters)]
impl Pinned for Archive {
    fn url(&self) -> &str {
        self.0.archive_url
    }
    fn size(&self) -> u64 {
        self.0.archive_size
    }
    fn sha256(&self) -> &str {
        self.0.archive_sha256
    }
    fn extension(&self) -> &str {
        "tgz"
    }
    fn display_name(&self) -> &str {
        "ONNX Runtime release archive"
    }
    fn licence(&self) -> &str {
        "MIT"
    }
}

/// The verified library in `models`, if it is there (stamp check only).
pub fn is_present(models: &Path) -> bool {
    for_this_cpu().is_some_and(|library| verify::is_present(models, library))
}

/// The verified library's path, hashing it only when its stamp is stale.
pub fn verified(models: &Path) -> Option<PathBuf> {
    for_this_cpu().and_then(|library| verify::verified_model(models, library).ok())
}

/// Bytes the download still has to fetch for the runtime: 0 when it is in
/// place or this CPU has no pinned build.
pub fn download_size(models: &Path) -> u64 {
    match for_this_cpu() {
        Some(library) if !verify::is_present(models, library) => library.archive_size,
        _ => 0,
    }
}

/// Download the archive (checksum-verified, resumable), unpack the library
/// from it, verify the library and keep only it.
pub fn install(
    models: &Path,
    library: &'static RuntimeLibrary,
    progress: impl FnMut(u64, u64),
) -> Result<PathBuf, FetchError> {
    if let Ok(path) = verify::verified_model(models, library) {
        return Ok(path);
    }
    let archive = Archive(library);
    let mut progress = progress;
    let archive_path =
        crate::fetch::fetch_into(models, &archive, &mut progress, &mut crate::fetch::curl)?;
    let target = unpack(models, library, &archive_path)?;
    let _ = crate::fetch::remove_in(models, &archive);
    Ok(target)
}

/// Unpack `library.member` from the archive into `<sha256>.so`, hashing it
/// as it streams; nothing is kept unless the hash and size match.
pub fn unpack(
    models: &Path,
    library: &RuntimeLibrary,
    archive: &Path,
) -> Result<PathBuf, FetchError> {
    let mut child = Command::new("tar")
        .arg("-xzOf")
        .arg(archive)
        .arg("--")
        .arg(library.member)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(FetchError::Downloader)?;
    let partial = paths::partial_file(models, library);
    let written = (|| -> Result<(), FetchError> {
        let mut output = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&partial)
            .map_err(FetchError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = output.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| FetchError::Io(io::Error::other("tar gave no output")))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut length = 0u64;
        loop {
            let read = stdout.read(&mut buffer).map_err(FetchError::Io)?;
            if read == 0 {
                break;
            }
            length += read as u64;
            if length > library.size {
                return Err(FetchError::WrongSize);
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read]).map_err(FetchError::Io)?;
        }
        if length != library.size {
            return Err(FetchError::WrongSize);
        }
        if verify::hex(&hasher.finalize()) != library.sha256 {
            return Err(FetchError::WrongChecksum);
        }
        output.sync_all().map_err(FetchError::Io)
    })();
    let status = child.wait().map_err(FetchError::Downloader)?;
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(FetchError::Network(status.code()));
    }
    let target = paths::model_file(models, library);
    std::fs::rename(&partial, &target).map_err(FetchError::Io)?;
    let _ = verify::record_verified(models, library);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_builds_are_pinned_tagged_releases() {
        for library in [&X86_64, &AARCH64] {
            assert!(library
                .archive_url
                .starts_with("https://github.com/microsoft/onnxruntime/releases/download/v1.23."));
            assert!(library.archive_url.ends_with(".tgz"));
            assert!(library.member.ends_with("/lib/libonnxruntime.so.1.23.2"));
            for sha in [library.sha256, library.archive_sha256] {
                assert_eq!(sha.len(), 64);
                assert!(sha
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
            }
            assert!(library.size > library.archive_size);
            assert_eq!(library.extension(), "so");
            assert_eq!(Archive(library).extension(), "tgz");
            assert!(crate::manifest::ALLOWED_LICENCES.contains(&library.licence()));
        }
    }

    #[test]
    fn unpacking_keeps_only_the_pinned_library() {
        let directory =
            std::env::temp_dir().join(format!("rmac-voice-runtime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let source = directory.join("src/pkg/lib");
        std::fs::create_dir_all(&source).unwrap();
        let content = b"not really a shared library".to_vec();
        std::fs::write(source.join("libonnxruntime.so.1.23.2"), &content).unwrap();
        let archive = directory.join("archive.tgz");
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(directory.join("src"))
            .arg("pkg")
            .status()
            .unwrap();
        assert!(status.success());
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let library = RuntimeLibrary {
            version: "1.23.2",
            archive_url: "https://example.invalid/x.tgz",
            archive_size: 1,
            archive_sha256: "0",
            member: "pkg/lib/libonnxruntime.so.1.23.2",
            size: content.len() as u64,
            sha256: Box::leak(verify::hex(&hasher.finalize()).into_boxed_str()),
        };
        let models = directory.join("models");
        std::fs::create_dir_all(&models).unwrap();
        let path = unpack(&models, &library, &archive).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), content);
        assert!(verify::is_present(&models, &library));
        // A library that does not match its pin is never kept.
        let wrong = RuntimeLibrary {
            sha256: "1111111111111111111111111111111111111111111111111111111111111111",
            ..library
        };
        assert!(matches!(
            unpack(&models, &wrong, &archive),
            Err(FetchError::WrongChecksum)
        ));
        assert!(!paths::model_file(&models, &wrong).exists());
        assert!(!paths::partial_file(&models, &wrong).exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
