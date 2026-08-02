#[cfg(not(target_os = "macos"))]
use std::io;
use std::path::Path;
use std::process::Command;

use crate::Error;

pub fn unmount(path: &Path) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    let (program, arguments) = (
        "diskutil",
        vec!["eject".to_string(), path.to_string_lossy().into_owned()],
    );
    #[cfg(not(target_os = "macos"))]
    let (program, arguments) = {
        let uri = url::Url::from_directory_path(path)
            .map_err(|()| Error::Io {
                operation: "encode mount path",
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "mount path is not absolute"),
            })?
            .to_string();
        ("gio", vec!["mount".to_string(), "-u".to_string(), uri])
    };

    let output = Command::new(program)
        .args(&arguments)
        .output()
        .map_err(|source| Error::Io {
            operation: "start unmount helper",
            path: path.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exited with {}", output.status)
        };
        Err(Error::Command {
            program,
            path: path.to_path_buf(),
            message,
        })
    }
}
