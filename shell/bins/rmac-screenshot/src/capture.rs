//! The capture backend (docs/decisions/0010-screenshots.md): `grim` reads
//! pixels through niri's wlr-screencopy, `wl-copy` serves the clipboard, and
//! a staged file under `$XDG_RUNTIME_DIR` is handed to its destination only
//! after the floating thumbnail leaves, as on macOS.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::model;

/// What to read, in niri's global logical layout coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Region {
    Output(String),
    Area {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
}

/// `grim [-c] (-o OUTPUT | -g "X,Y WxH") -t png OUT`. Areas round outward
/// to whole points so a selection is never cropped.
pub fn grim_arguments(region: &Region, show_pointer: bool, out: &Path) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = Vec::new();
    if show_pointer {
        arguments.push("-c".into());
    }
    match region {
        Region::Output(name) => {
            arguments.push("-o".into());
            arguments.push(name.into());
        }
        Region::Area {
            x,
            y,
            width,
            height,
        } => {
            let left = x.floor();
            let top = y.floor();
            let right = (x + width).ceil();
            let bottom = (y + height).ceil();
            arguments.push("-g".into());
            arguments.push(
                format!(
                    "{},{} {}x{}",
                    left as i64,
                    top as i64,
                    (right - left).max(1.0) as i64,
                    (bottom - top).max(1.0) as i64
                )
                .into(),
            );
        }
    }
    arguments.push("-t".into());
    arguments.push("png".into());
    arguments.push(out.as_os_str().to_owned());
    arguments
}

/// `$XDG_RUNTIME_DIR/rmac/screenshots/capture-<pid>-<n>.png` (0700 folder).
pub fn staging_path(sequence: u64) -> io::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let directory = runtime.join("rmac/screenshots");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    Ok(directory.join(format!("capture-{}-{sequence}.png", std::process::id())))
}

pub fn grab(region: &Region, show_pointer: bool, out: &Path) -> io::Result<()> {
    let status = Command::new("grim")
        .args(grim_arguments(region, show_pointer, out))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() && out.is_file() {
        Ok(())
    } else {
        Err(io::Error::other(format!("grim failed: {status}")))
    }
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Clipboard captures need wl-clipboard; without it the menu hides the
/// Clipboard destination.
pub fn clipboard_available() -> bool {
    on_path("wl-copy")
}

pub fn copy_to_clipboard(path: &Path) -> io::Result<()> {
    let file = fs::File::open(path)?;
    let status = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(file)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("wl-copy failed: {status}")))
    }
}

/// Copy the staged capture to `directory/name` without ever replacing an
/// existing file ("name (2).png" on a clash), then drop the staged copy.
pub fn deliver(staged: &Path, directory: &Path, name: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    for _ in 0..16 {
        let target = model::unique_path(directory, name, |path| path.exists());
        let mut output = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let mut input = fs::File::open(staged)?;
        if let Err(error) = io::copy(&mut input, &mut output).and_then(|_| output.sync_all()) {
            let _ = fs::remove_file(&target);
            return Err(error);
        }
        discard(staged);
        return Ok(target);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free screenshot name",
    ))
}

pub fn discard(staged: &Path) {
    let _ = fs::remove_file(staged);
}

/// Open a saved capture in the user's image viewer.
pub fn open(path: &Path) {
    match Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("could not open the screenshot: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grim_reads_outputs_and_rounds_areas_outward() {
        let out = Path::new("/run/user/1/rmac/screenshots/c.png");
        assert_eq!(
            grim_arguments(&Region::Output("eDP-1".into()), false, out),
            [
                "-o",
                "eDP-1",
                "-t",
                "png",
                "/run/user/1/rmac/screenshots/c.png"
            ]
            .map(OsString::from)
            .to_vec()
        );
        let area = Region::Area {
            x: 1470.5,
            y: 10.25,
            width: 300.0,
            height: 200.5,
        };
        assert_eq!(
            grim_arguments(&area, true, out)[..3],
            ["-c", "-g", "1470,10 301x201"].map(OsString::from)
        );
    }
}
