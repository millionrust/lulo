//! Windows apps' own icons for the Dock, Spotlight and Lulo mode's
//! desktop, as Explorer shows them (`IShellItemImageFactory`: a picture's
//! thumbnail on the desktop, as the Finder shows one), read on a
//! background thread and cached by the UI.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::RenderImage;
use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};

/// The edge, in physical pixels, icons are read at: the Dock's 48 pt tile
/// at up to 150 % scale.
pub const ICON_PIXELS: i32 = 72;

type Reply = async_channel::Sender<(String, Option<Arc<RenderImage>>)>;

/// One icon to read: the key the reply carries, what to read, at what
/// size, and whether a thumbnail of the contents may stand in.
struct Ask {
    key: String,
    source: String,
    pixels: i32,
    thumbnail: bool,
}

static WORKER: OnceLock<Mutex<Sender<Ask>>> = OnceLock::new();

/// Start the icon thread; each finished icon arrives on `replies` with
/// the source it was asked for.
pub fn start(replies: Reply) {
    let _ = WORKER.get_or_init(move || {
        let (sender, receiver) = std::sync::mpsc::channel::<Ask>();
        let spawned = std::thread::Builder::new()
            .name("lulo-icons".into())
            .spawn(move || {
                super::catalog::init_com();
                while let Ok(ask) = receiver.recv() {
                    let image = load(&ask.source, ask.pixels, ask.thumbnail).map(Arc::new);
                    if replies.send_blocking((ask.key, image)).is_err() {
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("lulo-shell: Windows apps will show no icons: {error}");
        }
        Mutex::new(sender)
    });
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
        thumbnail: true,
    });
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

fn load(source: &str, pixels: i32, thumbnail: bool) -> Option<RenderImage> {
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
        Some(RenderImage::new([image::Frame::new(buffer)]))
    }
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
