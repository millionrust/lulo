#[cfg(target_os = "linux")]
use std::io;
use std::path::Path;
#[cfg(not(target_os = "windows"))]
use std::process::Command;

use crate::Error;

/// Windows: ejecting a drive needs `CM_Request_Device_Eject`/`IOCTL_STORAGE_
/// EJECT_MEDIA`, which can refuse while any file on the volume is open and
/// has no portable, dependency-free binding today; left as an honest gap
/// rather than a silent no-op (ADR 0023 phase 4; tracked in docs/parity.md).
#[cfg(target_os = "windows")]
pub fn unmount(_path: &Path) -> Result<(), Error> {
    Err(Error::Io {
        operation: "eject volume",
        path: _path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "ejecting a drive is not available on Windows yet",
        ),
    })
}

#[cfg(not(target_os = "windows"))]
pub fn unmount(path: &Path) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    let (program, arguments) = (
        "diskutil",
        vec!["eject".to_string(), path.to_string_lossy().into_owned()],
    );
    #[cfg(target_os = "linux")]
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
