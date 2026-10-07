//! Windows apps' own icons for the Dock and Spotlight, as Explorer shows
//! them (`IShellItemImageFactory`), read on a background thread and cached
//! by the UI.

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

static WORKER: OnceLock<Mutex<Sender<String>>> = OnceLock::new();

/// Start the icon thread; each finished icon arrives on `replies` with
/// the source it was asked for.
pub fn start(replies: Reply) {
    let _ = WORKER.get_or_init(move || {
        let (sender, receiver) = std::sync::mpsc::channel::<String>();
        let spawned = std::thread::Builder::new()
            .name("lulo-icons".into())
            .spawn(move || {
                super::catalog::init_com();
                while let Ok(source) = receiver.recv() {
                    let image = load(&source).map(Arc::new);
                    if replies.send_blocking((source, image)).is_err() {
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
    if let Some(worker) = WORKER.get().and_then(|worker| worker.lock().ok()) {
        let _ = worker.send(source.to_owned());
    }
}

fn resolve(source: &str) -> String {
    if source.contains(['\\', ':']) {
        return source.to_owned();
    }
    let windows = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_owned());
    format!(r"{windows}\{source}")
}

fn load(source: &str) -> Option<RenderImage> {
    let path = HSTRING::from(resolve(source));
    // SAFETY: COM and GDI calls on objects this function creates and
    // releases; the pixel buffer is sized from the bitmap's own header.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&path, None::<&IBindCtx>).ok()?;
        let bitmap = factory
            .GetImage(
                SIZE {
                    cx: ICON_PIXELS,
                    cy: ICON_PIXELS,
                },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
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
