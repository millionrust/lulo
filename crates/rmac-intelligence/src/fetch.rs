//! The checksum-verified model download (ADR 0024 §4).
//!
//! The fetcher is the only Lulo Intelligence component that touches the
//! network, and it runs as its own one-shot process
//! (`rmac-intelligence-fetch`), started by System Settings. It downloads
//! from the manifest's revision-pinned URL with the system `curl` (the way
//! Spotlight's currency rates are fetched; no TLS stack is linked into
//! Lulo), streams a SHA-256 over every byte as it arrives, and only renames
//! the file into place when both the size and the hash match. An
//! interrupted download resumes from the partial file with an HTTP range
//! request; phase 0 found that a downloader's own "100 %" is no proof of a
//! complete file, so the final size is always checked here.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::manifest::Model;
use crate::{paths, verify};

#[derive(Debug)]
pub enum FetchError {
    NoDataDirectory,
    /// Another download of the same model is running.
    Busy,
    /// curl is not installed or failed to start.
    Downloader(io::Error),
    /// curl exited with an error (no network, HTTP error).
    Network(Option<i32>),
    /// The server sent more or fewer bytes than the manifest says.
    WrongSize,
    WrongChecksum,
    Io(io::Error),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDataDirectory => {
                formatter.write_str("there is no data folder to download into")
            }
            Self::Busy => formatter.write_str("the model is already downloading"),
            Self::Downloader(_) => formatter.write_str("the downloader (curl) is not available"),
            Self::Network(_) => formatter.write_str("the download failed; check the connection"),
            Self::WrongSize => formatter.write_str("the download was incomplete"),
            Self::WrongChecksum => {
                formatter.write_str("the downloaded file did not match its checksum")
            }
            Self::Io(error) => write!(formatter, "the model could not be saved: {error}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Download `model` into the user's model folder, calling `progress` with
/// (bytes so far, total) whenever another whole percent has arrived.
pub fn fetch(model: &Model, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, FetchError> {
    let models = paths::models_dir().ok_or(FetchError::NoDataDirectory)?;
    fetch_into(&models, model, &mut progress, &mut curl)
}

/// The byte source: `(url, offset) -> a reader of the bytes from offset`,
/// plus a way to learn whether it ended cleanly.
pub trait Source {
    type Reader: Read;
    fn open(&mut self, url: &str, offset: u64) -> Result<Self::Reader, FetchError>;
    fn finish(&mut self, reader: Self::Reader) -> Result<(), FetchError>;
}

impl<F: FnMut(&str, u64) -> Result<std::process::Child, FetchError>> Source for F {
    type Reader = std::process::Child;

    fn open(&mut self, url: &str, offset: u64) -> Result<Self::Reader, FetchError> {
        self(url, offset)
    }

    fn finish(&mut self, mut child: Self::Reader) -> Result<(), FetchError> {
        let status = child.wait().map_err(FetchError::Downloader)?;
        if status.success() {
            Ok(())
        } else {
            Err(FetchError::Network(status.code()))
        }
    }
}

fn curl(url: &str, offset: u64) -> Result<std::process::Child, FetchError> {
    let mut command = Command::new("curl");
    command
        .args([
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--output",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if offset > 0 {
        command.arg("--range").arg(format!("{offset}-"));
    }
    command
        .arg("--")
        .arg(url)
        .spawn()
        .map_err(FetchError::Downloader)
}

/// The whole download, with the byte source injectable for tests.
pub fn fetch_into<S: Source>(
    models: &Path,
    model: &Model,
    progress: &mut impl FnMut(u64, u64),
    source: &mut S,
) -> Result<PathBuf, FetchError>
where
    S::Reader: ChildOutput,
{
    rmac_storage::create_dir_all_private(models).map_err(FetchError::Io)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(models.join(format!("{}.lock", model.sha256)))
        .map_err(FetchError::Io)?;
    lock.try_lock().map_err(|_| FetchError::Busy)?;

    let target = paths::model_file(models, model);
    if verify::verified_model(models, model).is_ok() {
        progress(model.size, model.size);
        return Ok(target);
    }
    let partial = paths::partial_file(models, model);
    let mut hasher = Sha256::new();
    let mut have = match std::fs::symlink_metadata(&partial) {
        Ok(metadata) if metadata.file_type().is_file() && metadata.len() < model.size => {
            // Resume: hash what is already there.
            let mut file = File::open(&partial).map_err(FetchError::Io)?;
            let mut buffer = vec![0u8; 1 << 20];
            let mut length = 0u64;
            loop {
                let read = file.read(&mut buffer).map_err(FetchError::Io)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                length += read as u64;
            }
            length
        }
        Ok(_) => {
            std::fs::remove_file(&partial).map_err(FetchError::Io)?;
            0
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(FetchError::Io(error)),
    };
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .map_err(FetchError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = output.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    progress(have, model.size);
    let mut child = source.open(model.url, have)?;
    let mut reader = child
        .stdout_reader()
        .ok_or(FetchError::Downloader(io::Error::other(
            "the downloader has no output",
        )))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut last_percent = have * 100 / model.size.max(1);
    let streamed: Result<(), FetchError> = (|| loop {
        let read = reader.read(&mut buffer).map_err(FetchError::Io)?;
        if read == 0 {
            return Ok(());
        }
        if have + read as u64 > model.size {
            return Err(FetchError::WrongSize);
        }
        output.write_all(&buffer[..read]).map_err(FetchError::Io)?;
        hasher.update(&buffer[..read]);
        have += read as u64;
        let percent = have * 100 / model.size.max(1);
        if percent != last_percent {
            last_percent = percent;
            progress(have, model.size);
        }
    })();
    drop(reader);
    let finished = source.finish(child);
    if let Err(error) = streamed {
        if matches!(error, FetchError::WrongSize) {
            // A server that ignored the range resent the whole file.
            let _ = std::fs::remove_file(&partial);
        }
        return Err(error);
    }
    finished?;
    if have != model.size {
        return Err(FetchError::WrongSize);
    }
    if verify::hex(&hasher.finalize()) != model.sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err(FetchError::WrongChecksum);
    }
    output.sync_all().map_err(FetchError::Io)?;
    drop(output);
    std::fs::rename(&partial, &target).map_err(FetchError::Io)?;
    let _ = rmac_storage::atomic_write_private(
        &paths::licence_file(models, model),
        licence_notice(model).as_bytes(),
    );
    let _ = verify::record_verified(models, model);
    Ok(target)
}

/// What a download's reader exposes: its byte stream.
pub trait ChildOutput {
    type Output: Read;
    fn stdout_reader(&mut self) -> Option<Self::Output>;
}

impl ChildOutput for std::process::Child {
    type Output = std::process::ChildStdout;

    fn stdout_reader(&mut self) -> Option<Self::Output> {
        self.stdout.take()
    }
}

/// Delete a downloaded model, its partial download, stamp and licence.
pub fn remove(model: &Model) -> io::Result<()> {
    let models = paths::models_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data directory"))?;
    for path in [
        paths::model_file(&models, model),
        paths::partial_file(&models, model),
        paths::stamp_file(&models, model),
        paths::licence_file(&models, model),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Bytes of a partial download, for "Resume (42 %)".
pub fn partial_bytes(model: &Model) -> u64 {
    paths::models_dir()
        .map(|models| paths::partial_file(&models, model))
        .and_then(|path| std::fs::symlink_metadata(path).ok())
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.len().min(model.size))
        .unwrap_or(0)
}

fn licence_notice(model: &Model) -> String {
    format!(
        "{}\n\nSource: {}\nSHA-256: {}\nLicence: {} (the full text is in /usr/share/common-licenses/Apache-2.0)\n\
         The model is used unmodified. It is not part of Lulo OS and is not endorsed by its authors.\n",
        model.display_name, model.url, model.sha256, model.licence
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bytes(io::Cursor<Vec<u8>>);

    impl ChildOutput for Bytes {
        type Output = io::Cursor<Vec<u8>>;
        fn stdout_reader(&mut self) -> Option<Self::Output> {
            Some(std::mem::take(&mut self.0))
        }
    }

    /// Serves `content` from the requested offset; `cut` stops early once.
    struct Server {
        content: Vec<u8>,
        cut: Option<usize>,
        ignore_range: bool,
        offsets: Vec<u64>,
    }

    impl Source for Server {
        type Reader = Bytes;

        fn open(&mut self, _url: &str, offset: u64) -> Result<Bytes, FetchError> {
            self.offsets.push(offset);
            let start = if self.ignore_range {
                0
            } else {
                offset as usize
            };
            let end = self.cut.take().unwrap_or(self.content.len());
            Ok(Bytes(io::Cursor::new(self.content[start..end].to_vec())))
        }

        fn finish(&mut self, _reader: Bytes) -> Result<(), FetchError> {
            Ok(())
        }
    }

    fn model_for(content: &[u8]) -> Model {
        let mut hasher = Sha256::new();
        hasher.update(content);
        Model {
            tier: crate::manifest::Tier::Tiny,
            display_name: "test model",
            url: "https://example.invalid/model.gguf",
            size: content.len() as u64,
            sha256: Box::leak(verify::hex(&hasher.finalize()).into_boxed_str()),
            licence: "Apache-2.0",
            memory_budget_mib: 1,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rmac-intelligence-fetch-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn an_interrupted_download_resumes_and_verifies() {
        let models = scratch("resume");
        let content: Vec<u8> = (0..300_000u32).map(|value| value as u8).collect();
        let model = model_for(&content);
        let mut server = Server {
            content: content.clone(),
            cut: Some(120_000),
            ignore_range: false,
            offsets: Vec::new(),
        };
        let mut seen = Vec::new();
        // The first attempt ends early: short, kept for resuming.
        let first = fetch_into(&models, &model, &mut |done, _| seen.push(done), &mut server);
        assert!(matches!(first, Err(FetchError::WrongSize)));
        assert!(paths::partial_file(&models, &model).exists());
        let path =
            fetch_into(&models, &model, &mut |done, _| seen.push(done), &mut server).unwrap();
        assert_eq!(server.offsets, vec![0, 120_000]);
        assert_eq!(std::fs::read(&path).unwrap(), content);
        assert!(!paths::partial_file(&models, &model).exists());
        assert!(verify::is_present(&models, &model));
        assert_eq!(seen.last(), Some(&model.size));
        std::fs::remove_dir_all(&models).unwrap();
    }

    #[test]
    fn wrong_bytes_are_never_kept() {
        let models = scratch("wrong");
        let content = vec![7u8; 50_000];
        let model = model_for(&content);
        let mut tampered = Server {
            content: vec![8u8; 50_000],
            cut: None,
            ignore_range: false,
            offsets: Vec::new(),
        };
        let result = fetch_into(&models, &model, &mut |_, _| {}, &mut tampered);
        assert!(matches!(result, Err(FetchError::WrongChecksum)));
        assert!(!paths::model_file(&models, &model).exists());
        assert!(!paths::partial_file(&models, &model).exists());

        // A server that ignores the range resends everything: too long,
        // so the partial file is dropped instead of corrupted.
        std::fs::write(paths::partial_file(&models, &model), &content[..10_000]).unwrap();
        let mut careless = Server {
            content: content.clone(),
            cut: None,
            ignore_range: true,
            offsets: Vec::new(),
        };
        let result = fetch_into(&models, &model, &mut |_, _| {}, &mut careless);
        assert!(matches!(result, Err(FetchError::WrongSize)));
        assert!(!paths::partial_file(&models, &model).exists());
        std::fs::remove_dir_all(&models).unwrap();
    }
}
