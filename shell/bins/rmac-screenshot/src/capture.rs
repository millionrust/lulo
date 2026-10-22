//! The capture backend (docs/decisions/0010-screenshots.md): `grim` reads
//! pixels through niri's wlr-screencopy, `wl-copy` serves the clipboard, and
//! a staged file under `$XDG_RUNTIME_DIR` is handed to its destination only
//! after the floating thumbnail leaves, as on macOS. On Windows the pixels
//! come from the screen through GDI (areas in physical pixels there), the
//! staged file sits under `%TEMP%\Lulo\runtime\screenshots`, and a saved
//! capture opens in the default viewer through Explorer.

use std::ffi::OsString;
use std::fs;
use std::io;
#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Clipboard captures need wl-clipboard; without it the menu hides the
/// Clipboard destination.
#[cfg(unix)]
pub fn clipboard_available() -> bool {
    on_path("wl-copy")
}

#[cfg(unix)]
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
    #[cfg(windows)]
    let opener = "explorer.exe";
    #[cfg(not(windows))]
    let opener = "xdg-open";
    match Command::new(opener)
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

/// `%TEMP%\Lulo\runtime\screenshots\capture-<pid>-<n>.png`.
#[cfg(windows)]
pub fn staging_path(sequence: u64) -> io::Result<PathBuf> {
    let directory = std::env::temp_dir()
        .join("Lulo")
        .join("runtime")
        .join("screenshots");
    fs::create_dir_all(&directory)?;
    Ok(directory.join(format!("capture-{}-{sequence}.png", std::process::id())))
}

/// The Windows clipboard keeps no Lulo capture yet: the menu hides the
/// Clipboard destination.
#[cfg(windows)]
pub fn clipboard_available() -> bool {
    false
}

#[cfg(windows)]
pub fn copy_to_clipboard(_path: &Path) -> io::Result<()> {
    Err(io::Error::other("no clipboard captures on Windows yet"))
}

/// Read `region` (physical pixels on the virtual screen) and write it as a
/// PNG. The pointer is not drawn on Windows.
#[cfg(windows)]
pub fn grab(region: &Region, _show_pointer: bool, out: &Path) -> io::Result<()> {
    let Region::Area {
        x,
        y,
        width,
        height,
    } = region
    else {
        return Err(io::Error::other("Windows captures areas only"));
    };
    let left = x.floor() as i32;
    let top = y.floor() as i32;
    let width = ((x + width).ceil() as i32 - left).max(1);
    let height = ((y + height).ceil() as i32 - top).max(1);
    let bgra = windows_pixels(left, top, width, height)
        .ok_or_else(|| io::Error::other("the screen could not be read"))?;
    let mut rgba = bgra;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 0xFF;
    }
    let image = image::RgbaImage::from_raw(width as u32, height as u32, rgba)
        .ok_or_else(|| io::Error::other("the capture had the wrong size"))?;
    image
        .save_with_format(out, image::ImageFormat::Png)
        .map_err(io::Error::other)
}

/// Windows' own Desktop, Documents or Downloads folder.
#[cfg(windows)]
pub fn known_folder(destination: model::Destination) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, SHGetKnownFolderPath,
        KF_FLAG_DEFAULT,
    };
    let id = match destination {
        model::Destination::Desktop => &FOLDERID_Desktop,
        model::Destination::Documents => &FOLDERID_Documents,
        model::Destination::Downloads => &FOLDERID_Downloads,
        model::Destination::Clipboard | model::Destination::Preview => return None,
    };
    // SAFETY: the returned string is copied, then freed with CoTaskMemFree.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const core::ffi::c_void));
        text.filter(|text| !text.is_empty()).map(PathBuf::from)
    }
}

/// The screen's pixels in a rectangle, BGRA rows top first.
#[cfg(windows)]
fn windows_pixels(left: i32, top: i32, width: i32, height: i32) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HGDIOBJ,
        ROP_CODE, SRCCOPY,
    };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows top to bottom.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: a DIB section sized from `info`, copied into before its rows
    // are read, and deleted with the DCs before returning.
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            return None;
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            left,
            top,
            ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
        )
        .is_ok();
        let pixels = (copied && !bits.is_null()).then(|| {
            std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize).to_vec()
        });
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        pixels
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
