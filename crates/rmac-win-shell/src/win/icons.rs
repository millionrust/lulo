//! Windows apps' own icons for the Dock, Spotlight and Lulo mode's
//! desktop, as Explorer shows them (`IShellItemImageFactory`: a picture's
//! thumbnail on the desktop, as the Finder shows one), read off the UI
//! thread and cached by the UI.
//!
//! They are read in a short-lived helper process (`lulo-shell
//! --icon-helper`), not in the shell itself, as Explorer reads thumbnails
//! out of process: the Windows shell libraries, icon handlers and thumbnail
//! providers that reading icons loads cost lulo-shell several megabytes of
//! private memory that stayed for good on the owner's PC (WIN-OS-53), and a
//! crashing third-party handler can no longer take the layer down. The
//! helper starts with the first icon asked for and exits a few seconds
//! after the last, giving all of that back. If it cannot run, icons are
//! read in-process as before.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::windows::process::CommandExt as _;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gpui::RenderImage;
use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};

/// The edge, in physical pixels, icons are read at: the Dock's 48 pt tile
/// at up to 150 % scale.
pub const ICON_PIXELS: i32 = 72;

/// `lulo-shell`'s switch that makes it the icon helper.
pub const HELPER_SWITCH: &str = "--icon-helper";

/// How long the helper stays after the last icon it read.
const HELPER_IDLE: Duration = Duration::from_secs(4);

/// The largest icon edge the helper may send back.
const MAX_EDGE: u32 = 1024;

type Reply = async_channel::Sender<(String, Option<Arc<RenderImage>>)>;

/// One icon to read: the key the reply carries, what to read, at what
/// size, and whether a thumbnail of the contents may stand in.
struct Ask {
    key: String,
    source: String,
    pixels: i32,
    thumbnail: bool,
}

/// Straight-alpha BGRA pixels: width, height, bytes.
type Pixels = (u32, u32, Vec<u8>);

static WORKER: OnceLock<Mutex<Sender<Ask>>> = OnceLock::new();

/// Start the icon thread; each finished icon arrives on `replies` with
/// the source it was asked for.
pub fn start(replies: Reply) {
    let _ = WORKER.get_or_init(move || {
        let (sender, receiver) = std::sync::mpsc::channel::<Ask>();
        let spawned = std::thread::Builder::new()
            .name("lulo-icons".into())
            .spawn(move || serve(receiver, replies));
        if let Err(error) = spawned {
            eprintln!("lulo-shell: Windows apps will show no icons: {error}");
        }
        Mutex::new(sender)
    });
}

/// The running helper process and its pipes.
struct Helper {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl Helper {
    fn spawn() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let mut child = Command::new(exe)
            .arg(HELPER_SWITCH)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .ok()?;
        let input = child.stdin.take()?;
        let output = BufReader::new(child.stdout.take()?);
        super::trace(|| format!("icon helper started, pid {}", child.id()));
        Some(Self {
            child,
            input: Some(input),
            output,
        })
    }

    fn read(&mut self, ask: &Ask) -> std::io::Result<Option<Pixels>> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| std::io::Error::other("helper closed"))?;
        writeln!(
            input,
            "{}\t{}\t{}",
            ask.pixels,
            u8::from(ask.thumbnail),
            ask.source
        )?;
        input.flush()?;
        let mut header = String::new();
        if self.output.read_line(&mut header)? == 0 {
            return Err(std::io::Error::other("helper ended"));
        }
        let mut numbers = header.split_whitespace().map(str::parse::<u32>);
        let (Some(Ok(width)), Some(Ok(height))) = (numbers.next(), numbers.next()) else {
            return Err(std::io::Error::other("bad helper reply"));
        };
        if width == 0 || height == 0 {
            return Ok(None);
        }
        if width > MAX_EDGE || height > MAX_EDGE {
            return Err(std::io::Error::other("helper icon too large"));
        }
        let mut bytes = vec![0u8; width as usize * height as usize * 4];
        self.output.read_exact(&mut bytes)?;
        Ok(Some((width, height, bytes)))
    }

    /// Close its input, so it ends, and wait for it.
    fn close(mut self) {
        drop(self.input.take());
        let _ = self.child.wait();
        super::trace(|| "icon helper ended".into());
    }
}

/// The icon thread: each icon from the helper (started on demand, closed
/// when idle), or read here if the helper cannot run.
fn serve(receiver: Receiver<Ask>, replies: Reply) {
    let mut helper: Option<Helper> = None;
    let mut in_process = false;
    loop {
        let ask = if helper.is_some() {
            match receiver.recv_timeout(HELPER_IDLE) {
                Ok(ask) => ask,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(idle) = helper.take() {
                        idle.close();
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(ask) => ask,
                Err(_) => break,
            }
        };
        if helper.is_none() && !in_process {
            helper = Helper::spawn();
        }
        let mut pixels = None;
        let mut read = false;
        if let Some(running) = helper.as_mut() {
            match running.read(&ask) {
                Ok(result) => {
                    pixels = result;
                    read = true;
                }
                Err(error) => {
                    super::trace(|| format!("icon helper failed: {error}"));
                    if let Some(broken) = helper.take() {
                        broken.close();
                    }
                }
            }
        }
        if !read {
            if !in_process {
                super::catalog::init_com();
                in_process = true;
            }
            pixels = load(&ask.source, ask.pixels, ask.thumbnail);
        }
        let image = pixels
            .and_then(|(width, height, bytes)| image::RgbaImage::from_raw(width, height, bytes))
            .map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)])));
        if replies.send_blocking((ask.key, image)).is_err() {
            break;
        }
    }
    if let Some(running) = helper.take() {
        running.close();
    }
}

/// `lulo-shell --icon-helper`: read each asked-for icon from standard input
/// (`<pixels>\t<thumbnail 0|1>\t<source>` per line) and write it to
/// standard output (`<width> <height>\n` then the BGRA bytes, or `0 0\n`
/// for none) until the input closes.
pub fn run_helper() -> i32 {
    super::catalog::init_com();
    let stdin = std::io::stdin();
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let mut fields = line.splitn(3, '\t');
        let (Some(pixels), Some(thumbnail), Some(source)) =
            (fields.next(), fields.next(), fields.next())
        else {
            break;
        };
        let pixels = pixels
            .parse::<i32>()
            .unwrap_or(ICON_PIXELS)
            .clamp(1, MAX_EDGE as i32);
        let written = match load(source, pixels, thumbnail == "1") {
            Some((width, height, bytes)) => {
                writeln!(output, "{width} {height}").and_then(|()| output.write_all(&bytes))
            }
            None => writeln!(output, "0 0"),
        };
        if written.and_then(|()| output.flush()).is_err() {
            break;
        }
    }
    0
}

/// Ask for the icon of `source`: an executable path (a bare file name is
/// looked for in the Windows folder) or a shell parsing name such as
/// `shell:AppsFolder\…`.
pub fn request(source: &str) {
    send(Ask {
        key: source.to_owned(),
        source: source.to_owned(),
        pixels: ICON_PIXELS,
        thumbnail: false,
    });
}

/// The cache key of a desktop item's icon at `pixels`.
pub fn desktop_key(path: &std::path::Path, pixels: i32) -> String {
    format!("desktop:{pixels}:{}", path.display())
}

/// Ask for a desktop item's icon (or its picture's thumbnail) at `pixels`;
/// it arrives under `key` (see [`desktop_key`]).
pub fn request_desktop(key: &str, path: &str, pixels: i32) {
    send(Ask {
        key: key.to_owned(),
        source: path.to_owned(),
        pixels,
        thumbnail: shows_thumbnail(path),
    });
}

/// Whether the desktop shows `path`'s contents rather than its icon: a
/// picture or a video, as the Finder does. Everything else, shortcuts
/// above all, gets its icon alone (`SIIGBF_ICONONLY`), as Explorer's
/// desktop does. Asked for a thumbnail, the shell drew a shortcut whose
/// icon has no large size as that small icon inside a square framed box
/// ("Google Play Games", WIN-OS-52).
pub fn shows_thumbnail(path: &str) -> bool {
    const CONTENTS: [&str; 14] = [
        "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "avif", "mp4", "mov",
        "mkv", "wmv",
    ];
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            CONTENTS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn send(ask: Ask) {
    if let Some(worker) = WORKER.get().and_then(|worker| worker.lock().ok()) {
        let _ = worker.send(ask);
    }
}

fn resolve(source: &str) -> String {
    if source.contains(['\\', ':']) {
        return source.to_owned();
    }
    let windows = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_owned());
    format!(r"{windows}\{source}")
}

/// Whether `source` is a file-system path (a desktop item, an exe), not a
/// shell parsing name such as `shell:AppsFolder\…`.
fn is_file_path(source: &str) -> bool {
    !source.starts_with("shell:") && !source.starts_with("::")
}

/// Read the icon of `source` (in this process: the Apps folder helper's)
/// and keep it as a PNG at `path`, no larger than `pixels` square. True
/// when it was written.
pub(crate) fn save_png(source: &str, pixels: u32, path: &std::path::Path) -> bool {
    let Some((width, height, mut bytes)) = load(source, pixels as i32, false) else {
        return false;
    };
    // BGRA from Windows; PNG wants RGBA.
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let Some(image) = image::RgbaImage::from_raw(width, height, bytes) else {
        return false;
    };
    let image = if width > pixels || height > pixels {
        image::imageops::thumbnail(&image, pixels, pixels)
    } else {
        image
    };
    image
        .save_with_format(path, image::ImageFormat::Png)
        .is_ok()
}

fn load(source: &str, pixels: i32, thumbnail: bool) -> Option<Pixels> {
    if !thumbnail && is_file_path(source) {
        if let Some(icon) = load_from_image_list(source, pixels) {
            return Some(icon);
        }
    }
    load_from_factory(source, pixels, thumbnail)
}

/// A file's icon as Explorer's desktop draws it: from the system image
/// list, the jumbo (256 px) image when the icon has one, else its 48 px
/// image. `IShellItemImageFactory` drew an icon without large images as
/// its small image inside a square framed box (the thumbnail cache's frame
/// for low-resolution icons), which is how "Google Play Games" and other
/// shortcuts showed on the owner's desktop (WIN-OS-52).
fn load_from_image_list(source: &str, pixels: i32) -> Option<Pixels> {
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHIL_EXTRALARGE,
        SHIL_JUMBO,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let path = HSTRING::from(resolve(source));
    let mut info = SHFILEINFOW::default();
    // SAFETY: the info structure is passed with its own size; the system
    // image list's icons are copies this function destroys.
    unsafe {
        let found = SHGetFileInfoW(
            &path,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        );
        if found == 0 {
            return None;
        }
        let render = |list: u32, edge: i32| -> Option<Pixels> {
            let images: IImageList = SHGetImageList(list as i32).ok()?;
            let icon = images.GetIcon(info.iIcon, ILD_TRANSPARENT.0).ok()?;
            let pixels = draw_icon(icon, edge);
            let _ = DestroyIcon(icon);
            pixels
        };
        let jumbo = if pixels > 48 {
            render(SHIL_JUMBO, 256)
        } else {
            None
        };
        // The jumbo list holds an icon without a 256 px image at its small
        // size in the top-left corner of the canvas: use the 48 px image
        // instead.
        let icon = match jumbo {
            Some(jumbo) if !content_within(&jumbo, 48) => jumbo,
            _ => render(SHIL_EXTRALARGE, 48)?,
        };
        let (width, height, bytes) = icon;
        let buffer = image::RgbaImage::from_raw(width, height, bytes)?;
        let buffer = shrink_to(buffer, pixels);
        let (width, height) = buffer.dimensions();
        Some((width, height, buffer.into_raw()))
    }
}

/// Whether every visible pixel of `icon` lies in its top-left `edge` ×
/// `edge` corner.
fn content_within(icon: &Pixels, edge: u32) -> bool {
    let (width, _, bytes) = icon;
    bytes.chunks_exact(4).enumerate().all(|(index, pixel)| {
        let (x, y) = (index as u32 % width, index as u32 / width);
        pixel[3] == 0 || (x < edge && y < edge)
    })
}

/// Draw `icon` at `edge` × `edge` and read it back as straight-alpha
/// BGRA. It is drawn over black and over white and each pixel's alpha is
/// what the two differ by, which works for icons with an alpha channel and
/// for old masked ones alike.
fn draw_icon(icon: windows::Win32::UI::WindowsAndMessaging::HICON, edge: i32) -> Option<Pixels> {
    use windows::Win32::Graphics::Gdi::{CreateDIBSection, SelectObject};
    use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: edge,
            biHeight: -edge,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let length = edge as usize * edge as usize * 4;
    let draw_over = |background: u8| -> Option<Vec<u8>> {
        // SAFETY: a DIB section of `length` bytes, filled, drawn into
        // through a memory DC, copied out and deleted.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                return None;
            };
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, length);
            pixels.fill(background);
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            let drawn = DrawIconEx(dc, 0, 0, icon, edge, edge, 0, None, DI_NORMAL).is_ok();
            SelectObject(dc, previous);
            let copy = drawn.then(|| pixels.to_vec());
            let _ = DeleteDC(dc);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            copy
        }
    };
    let black = draw_over(0)?;
    let white = draw_over(255)?;
    let mut bytes = vec![0u8; length];
    for ((out, over_black), over_white) in bytes
        .chunks_exact_mut(4)
        .zip(black.chunks_exact(4))
        .zip(white.chunks_exact(4))
    {
        let difference = (0..3)
            .map(|channel| i32::from(over_white[channel]) - i32::from(over_black[channel]))
            .max()
            .unwrap_or(255)
            .clamp(0, 255);
        let alpha = 255 - difference;
        if alpha == 0 {
            continue;
        }
        for channel in 0..3 {
            // Over black the colour is premultiplied by alpha.
            out[channel] =
                ((i32::from(over_black[channel]) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
        out[3] = alpha as u8;
    }
    Some((edge as u32, edge as u32, bytes))
}

fn load_from_factory(source: &str, pixels: i32, thumbnail: bool) -> Option<Pixels> {
    let requested = pixels;
    let path = HSTRING::from(resolve(source));
    // SAFETY: COM and GDI calls on objects this function creates and
    // releases; the pixel buffer is sized from the bitmap's own header.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&path, None::<&IBindCtx>).ok()?;
        let bitmap = factory
            .GetImage(
                SIZE {
                    cx: pixels,
                    cy: pixels,
                },
                if thumbnail {
                    SIIGBF_BIGGERSIZEOK
                } else {
                    SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK
                },
            )
            .ok()?;
        let pixels = read_bitmap(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let (width, height, pixels) = pixels?;
        let buffer = image::RgbaImage::from_raw(width, height, pixels)?;
        // BIGGERSIZEOK may hand back a 256 px jumbo icon for a 64 px ask;
        // keep only what is drawn (a 256 px BGRA icon is 256 KiB of atlas
        // per desktop item, WIN-OS-53).
        let buffer = shrink_to(buffer, requested);
        let (width, height) = buffer.dimensions();
        Some((width, height, buffer.into_raw()))
    }
}

/// `image` scaled down so its longer side is at most `edge` pixels.
fn shrink_to(image: image::RgbaImage, edge: i32) -> image::RgbaImage {
    let edge = edge.max(1) as u32;
    let (width, height) = image.dimensions();
    if width <= edge && height <= edge {
        return image;
    }
    let factor = edge as f32 / width.max(height) as f32;
    let target = |side: u32| ((side as f32 * factor).round() as u32).max(1);
    image::imageops::resize(
        &image,
        target(width),
        target(height),
        image::imageops::FilterType::Triangle,
    )
}

/// The bitmap's pixels, top row first, as straight-alpha BGRA (GPUI's
/// image layout).
fn read_bitmap(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    let mut header = BITMAP::default();
    // SAFETY: a BITMAP-sized out-parameter.
    let read = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut header as *mut BITMAP as *mut core::ffi::c_void),
        )
    };
    if read == 0 || header.bmWidth <= 0 || header.bmHeight == 0 {
        return None;
    }
    let width = header.bmWidth;
    let height = header.bmHeight.abs();
    let mut info = BITMAPINFO {
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
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: a memory DC for the call, deleted after it; `pixels` holds
    // exactly the rows asked for.
    let rows = unsafe {
        let dc = CreateCompatibleDC(None);
        let rows = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        rows
    };
    if rows == 0 {
        return None;
    }
    // Shell bitmaps carry premultiplied alpha; an old icon with none at all
    // is opaque.
    let has_alpha = pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
    for pixel in pixels.chunks_exact_mut(4) {
        if !has_alpha {
            pixel[3] = 255;
            continue;
        }
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    Some((width as u32, height as u32, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_show_their_icon_and_pictures_their_contents() {
        assert!(!shows_thumbnail(
            r"C:\Users\a\Desktop\Google Play Games.lnk"
        ));
        assert!(!shows_thumbnail(r"C:\Users\a\Desktop\Game.url"));
        assert!(!shows_thumbnail(r"C:\Users\a\Desktop\setup.exe"));
        assert!(!shows_thumbnail(r"C:\Users\a\Desktop\notes.txt"));
        assert!(shows_thumbnail(r"C:\Users\a\Desktop\hello.JPEG"));
        assert!(shows_thumbnail(r"C:\Users\a\Desktop\clip.mp4"));
    }

    #[test]
    fn a_small_icon_in_the_jumbo_canvas_is_found() {
        let mut bytes = vec![0u8; 256 * 256 * 4];
        // One visible pixel at (40, 40): inside the top-left 48 px corner.
        bytes[(40 * 256 + 40) * 4 + 3] = 255;
        let small = (256, 256, bytes.clone());
        assert!(content_within(&small, 48));
        bytes[(200 * 256 + 128) * 4 + 3] = 255;
        assert!(!content_within(&(256, 256, bytes), 48));
        assert!(is_file_path(
            r"C:\Users\Public\Desktop\Google Play Games.lnk"
        ));
        assert!(!is_file_path(
            r"shell:AppsFolder\Microsoft.WindowsCalculator"
        ));
    }

    #[test]
    fn a_jumbo_icon_is_shrunk_to_the_size_asked_for() {
        let jumbo = image::RgbaImage::new(256, 256);
        assert_eq!(shrink_to(jumbo, 64).dimensions(), (64, 64));
        let small = image::RgbaImage::new(48, 48);
        assert_eq!(shrink_to(small, 64).dimensions(), (48, 48));
    }
}
