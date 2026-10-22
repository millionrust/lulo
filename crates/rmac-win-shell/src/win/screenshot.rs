//! Screenshots on Windows, as Lulo OS takes them (docs/decisions/
//! 0010-screenshots.md): ⇧⌘3 the whole screen under the pointer, ⇧⌘4 an
//! area dragged out with the crosshair (Ctrl+Shift+3 and Ctrl+Shift+4,
//! ⌘ being Ctrl), saved where Lulo's screenshot settings say (the Desktop
//! by default) as "Screenshot 2026-10-22 at
//! 1.36.39 PM.png" with the shutter sound. ⇧⌘5 (Ctrl+Shift+5) opens the
//! area crosshair too; Lulo OS's toolbar is not drawn here yet.
//!
//! The area is chosen on a still picture of the screen taken when the keys
//! are pressed, as the Mac freezes nothing but nothing moves under a drag
//! that lasts a moment. It is a plain Win32 window on a thread of its own,
//! which ends with the capture: nothing stays resident.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreatePen,
    DeleteDC, DeleteObject, EndPaint, GetDC, GetStockObject, InvalidateRect, MonitorFromPoint,
    Rectangle, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TextOutW, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, NULL_BRUSH, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow,
    IDC_CROSS, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN,
    WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

/// One capture at a time, as on the Mac.
static BUSY: AtomicBool = AtomicBool::new(false);

/// ⇧⌘3: the whole screen the pointer is on.
pub fn screen() {
    run(|| {
        let area = monitor_under_pointer()?;
        let shot = Shot::take(area)?;
        save(&shot.pixels, shot.width, shot.height)
    });
}

/// ⇧⌘4 (and ⇧⌘5): drag out an area with the crosshair; Esc cancels.
pub fn selection() {
    run(|| {
        let shot = Shot::take(virtual_screen())?;
        let chosen = choose_area(&shot)?;
        let (pixels, width, height) = crop(&shot.pixels, shot.width, &chosen)?;
        save(&pixels, width, height)
    });
}

fn run(work: impl FnOnce() -> Option<PathBuf> + Send + 'static) {
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("lulo-screenshot".into())
        .spawn(move || {
            if let Some(path) = work() {
                super::trace(|| format!("screenshot saved {}", path.display()));
            }
            BUSY.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        BUSY.store(false, Ordering::Release);
    }
}

/// A picture of part of the screen, BGRA rows top first.
struct Shot {
    area: RECT,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Shot {
    fn take(area: RECT) -> Option<Self> {
        let width = (area.right - area.left).max(0);
        let height = (area.bottom - area.top).max(0);
        if width == 0 || height == 0 {
            return None;
        }
        let info = bitmap_info(width, height);
        // SAFETY: a DIB section sized from `info`, copied into before its
        // rows are read and deleted with the DCs before returning.
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bitmap) =
                CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
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
                area.left,
                area.top,
                windows::Win32::Graphics::Gdi::ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
            )
            .is_ok();
            let pixels = (copied && !bits.is_null()).then(|| {
                std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize)
                    .to_vec()
            });
            SelectObject(memory, previous);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            Some(Self {
                area,
                width: width as u32,
                height: height as u32,
                pixels: pixels?,
            })
        }
    }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
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
    }
}

fn virtual_screen() -> RECT {
    // SAFETY: plain metrics.
    unsafe {
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        RECT {
            left,
            top,
            right: left + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: top + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

fn monitor_under_pointer() -> Option<RECT> {
    let mut pointer = POINT::default();
    // SAFETY: out-parameters of the right types.
    unsafe {
        GetCursorPos(&mut pointer).ok()?;
        let monitor = MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        windows::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut info)
            .as_bool()
            .then_some(info.rcMonitor)
    }
}

/// The rectangle two corners span, in the picture's own pixels.
fn span(start: POINT, end: POINT) -> RECT {
    RECT {
        left: start.x.min(end.x),
        top: start.y.min(end.y),
        right: start.x.max(end.x),
        bottom: start.y.max(end.y),
    }
}

/// `area` (in the picture's pixels) cut out of a `width`-wide BGRA picture.
fn crop(pixels: &[u8], width: u32, area: &RECT) -> Option<(Vec<u8>, u32, u32)> {
    let height = (pixels.len() / (width as usize * 4)) as i32;
    let left = area.left.clamp(0, width as i32);
    let right = area.right.clamp(0, width as i32);
    let top = area.top.clamp(0, height);
    let bottom = area.bottom.clamp(0, height);
    if right - left < 2 || bottom - top < 2 {
        // A click without a drag takes nothing, as on the Mac.
        return None;
    }
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity(((right - left) * (bottom - top) * 4) as usize);
    for row in top..bottom {
        let start = row as usize * stride + left as usize * 4;
        out.extend_from_slice(&pixels[start..start + (right - left) as usize * 4]);
    }
    Some((out, (right - left) as u32, (bottom - top) as u32))
}

/// "Screenshot 2026-10-22 at 1.36.39 PM.png", as on the Mac and Lulo OS.
fn file_name(time: &chrono::NaiveDateTime) -> String {
    format!(
        "Screenshot {} at {}\u{202F}{}.png",
        time.format("%Y-%m-%d"),
        time.format("%-I.%M.%S"),
        time.format("%p"),
    )
}

/// `directory/name`, or "name (2).png"… when that is taken.
fn unique_path(directory: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let candidate = directory.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let stem = name.strip_suffix(".png").unwrap_or(name);
    (2..)
        .map(|index| directory.join(format!("{stem} ({index}).png")))
        .find(|path| !exists(path))
        .unwrap_or(candidate)
}

/// Where a capture is saved, from Lulo's own settings as on Lulo OS: the
/// screenshot settings' `destination` (Desktop, Documents or Downloads;
/// the Desktop otherwise) in `$XDG_CONFIG_HOME/rmac/screenshot.conf`, and
/// that folder from `$XDG_CONFIG_HOME/user-dirs.dirs` when it names one,
/// else Windows' own folder of that kind.
fn save_folder() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let read = |name: &str| {
        config
            .as_ref()
            .and_then(|config| std::fs::read_to_string(config.join(name)).ok())
    };
    let (key, known) = folder_for(read("rmac/screenshot.conf").as_deref());
    user_dir(read("user-dirs.dirs").as_deref(), key).or_else(|| super::catalog::known_folder(known))
}

/// The `user-dirs.dirs` key and Windows' known folder for the settings'
/// destination.
fn folder_for(settings: Option<&str>) -> (&'static str, &'static windows::core::GUID) {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads};
    let destination = settings
        .into_iter()
        .flat_map(str::lines)
        .find_map(|line| line.trim().strip_prefix("destination="))
        .map(str::trim);
    match destination {
        Some("documents") => ("XDG_DOCUMENTS_DIR", &FOLDERID_Documents),
        Some("downloads") => ("XDG_DOWNLOAD_DIR", &FOLDERID_Downloads),
        _ => ("XDG_DESKTOP_DIR", &FOLDERID_Desktop),
    }
}

/// `KEY="C:/absolute/folder"` from `user-dirs.dirs`; anything else (a
/// `$HOME`-relative or relative path) leaves the choice to Windows.
fn user_dir(user_dirs: Option<&str>, key: &str) -> Option<PathBuf> {
    let prefix = format!("{key}=");
    let value = user_dirs?.lines().find_map(|line| {
        let line = line.trim();
        (!line.starts_with('#'))
            .then(|| line.strip_prefix(prefix.as_str()))
            .flatten()
    })?;
    let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    let path = PathBuf::from(value);
    (path.is_absolute() && !value.contains('$')).then_some(path)
}

/// Save BGRA pixels to Lulo's screenshot folder as a PNG, with the shutter
/// sound.
fn save(bgra: &[u8], width: u32, height: u32) -> Option<PathBuf> {
    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 0xFF;
    }
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let folder = save_folder()?;
    let _ = std::fs::create_dir_all(&folder);
    let name = file_name(&chrono::Local::now().naive_local());
    let path = unique_path(&folder, &name, Path::exists);
    let _ = rmac_sound::play(rmac_sound::Cue::Screenshot);
    match image.save_with_format(&path, image::ImageFormat::Png) {
        Ok(()) => Some(path),
        Err(error) => {
            eprintln!("lulo-shell: the screenshot was not saved: {error}");
            None
        }
    }
}

/// The crosshair window's state, on its own thread.
struct Picker {
    /// The still picture, selected into `memory` for painting.
    memory: HDC,
    bitmap: HBITMAP,
    start: Option<POINT>,
    current: POINT,
    chosen: Option<RECT>,
}

thread_local! {
    static PICKER: std::cell::RefCell<Option<Picker>> = const { std::cell::RefCell::new(None) };
}

/// Show the still picture full screen with the crosshair and let the user
/// drag out an area; `None` on Esc, a right click or a click without a drag.
fn choose_area(shot: &Shot) -> Option<RECT> {
    let width = shot.width as i32;
    let height = shot.height as i32;
    // SAFETY: the picture is copied into a bitmap selected into a memory DC
    // owned by this thread's picker and deleted when the window closes; the
    // window and its class live on this thread only.
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        SelectObject(memory, HGDIOBJ(bitmap.0));
        let info = bitmap_info(width, height);
        windows::Win32::Graphics::Gdi::SetDIBitsToDevice(
            memory,
            0,
            0,
            width as u32,
            height as u32,
            0,
            0,
            0,
            height as u32,
            shot.pixels.as_ptr() as *const core::ffi::c_void,
            &info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, screen);
        PICKER.with(|picker| {
            *picker.borrow_mut() = Some(Picker {
                memory,
                bitmap,
                start: None,
                current: POINT::default(),
                chosen: None,
            })
        });
        let class = w!("LuloScreenshotArea");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(on_picker),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            ..Default::default()
        });
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("Screenshot"),
            WS_POPUP | WS_VISIBLE,
            shot.area.left,
            shot.area.top,
            width,
            height,
            None,
            None,
            None,
            None,
        )
        .ok()?;
        let _ = SetForegroundWindow(window);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            DispatchMessageW(&message);
        }
        PICKER.with(|picker| {
            let picker = picker.borrow_mut().take()?;
            let _ = DeleteDC(picker.memory);
            let _ = DeleteObject(HGDIOBJ(picker.bitmap.0));
            picker.chosen
        })
    }
}

fn client_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

unsafe extern "system" fn on_picker(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let close = |chosen: Option<RECT>| {
        PICKER.with(|picker| {
            if let Some(picker) = picker.borrow_mut().as_mut() {
                picker.chosen = chosen;
            }
        });
        // SAFETY: this thread's own window.
        unsafe {
            let _ = ReleaseCapture();
            let _ = DestroyWindow(window);
        }
    };
    match message {
        WM_SETCURSOR => {
            // SAFETY: a system cursor.
            unsafe { SetCursor(LoadCursorW(None, IDC_CROSS).ok()) };
            return LRESULT(1);
        }
        WM_LBUTTONDOWN => {
            let point = client_point(lparam);
            PICKER.with(|picker| {
                if let Some(picker) = picker.borrow_mut().as_mut() {
                    picker.start = Some(point);
                    picker.current = point;
                }
            });
            // SAFETY: this thread's own window.
            unsafe { SetCapture(window) };
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let point = client_point(lparam);
            PICKER.with(|picker| {
                if let Some(picker) = picker.borrow_mut().as_mut() {
                    picker.current = point;
                }
            });
            // SAFETY: repaint this thread's own window.
            unsafe {
                let _ = InvalidateRect(Some(window), None, false);
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let point = client_point(lparam);
            let chosen = PICKER.with(|picker| {
                let picker = picker.borrow();
                let start = picker.as_ref()?.start?;
                Some(span(start, point))
            });
            close(chosen);
            return LRESULT(0);
        }
        WM_RBUTTONDOWN => {
            close(None);
            return LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 == usize::from(VK_ESCAPE.0) => {
            close(None);
            return LRESULT(0);
        }
        WM_PAINT => {
            paint(window);
            return LRESULT(0);
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: the default handling of a message to this window.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The still picture, the dragged area's outline and its size beside the
/// crosshair, as the Mac's ⇧⌘4 readout shows it. Drawn off screen first so
/// nothing flickers.
fn paint(window: HWND) {
    PICKER.with(|picker| {
        let picker = picker.borrow();
        let Some(picker) = picker.as_ref() else {
            return;
        };
        // SAFETY: painting this thread's own window between BeginPaint and
        // EndPaint; the off-screen bitmap and pens are deleted here.
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            let target = BeginPaint(window, &mut paint);
            let mut client = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut client);
            let width = client.right;
            let height = client.bottom;
            let frame = CreateCompatibleDC(Some(target));
            let canvas = CreateCompatibleBitmap(target, width, height);
            let previous = SelectObject(frame, HGDIOBJ(canvas.0));
            let _ = BitBlt(
                frame,
                0,
                0,
                width,
                height,
                Some(picker.memory),
                0,
                0,
                SRCCOPY,
            );
            if let Some(start) = picker.start {
                let area = span(start, picker.current);
                let dark = CreatePen(
                    PS_SOLID,
                    3,
                    windows::Win32::Foundation::COLORREF(0x0000_0000),
                );
                let light = CreatePen(
                    PS_SOLID,
                    1,
                    windows::Win32::Foundation::COLORREF(0x00FF_FFFF),
                );
                let brush = SelectObject(frame, GetStockObject(NULL_BRUSH));
                let pen = SelectObject(frame, HGDIOBJ(dark.0));
                let _ = Rectangle(frame, area.left, area.top, area.right, area.bottom);
                SelectObject(frame, HGDIOBJ(light.0));
                let _ = Rectangle(frame, area.left, area.top, area.right, area.bottom);
                SelectObject(frame, pen);
                SelectObject(frame, brush);
                let _ = DeleteObject(HGDIOBJ(dark.0));
                let _ = DeleteObject(HGDIOBJ(light.0));
                let readout: Vec<u16> =
                    format!("{}  {}", area.right - area.left, area.bottom - area.top)
                        .encode_utf16()
                        .collect();
                SetBkMode(frame, TRANSPARENT);
                SetTextColor(frame, windows::Win32::Foundation::COLORREF(0x0000_0000));
                let _ = TextOutW(
                    frame,
                    picker.current.x + 13,
                    picker.current.y + 13,
                    &readout,
                );
                SetTextColor(frame, windows::Win32::Foundation::COLORREF(0x00FF_FFFF));
                let _ = TextOutW(
                    frame,
                    picker.current.x + 12,
                    picker.current.y + 12,
                    &readout,
                );
            }
            let _ = BitBlt(target, 0, 0, width, height, Some(frame), 0, 0, SRCCOPY);
            SelectObject(frame, previous);
            let _ = DeleteObject(HGDIOBJ(canvas.0));
            let _ = DeleteDC(frame);
            let _ = EndPaint(window, &paint);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_are_named_as_on_the_mac() {
        let time = chrono::NaiveDate::from_ymd_opt(2026, 10, 22)
            .unwrap()
            .and_hms_opt(13, 36, 39)
            .unwrap();
        assert_eq!(
            file_name(&time),
            "Screenshot 2026-10-22 at 1.36.39\u{202F}PM.png"
        );
        let taken = |path: &Path| !path.to_string_lossy().contains("(3)");
        assert_eq!(
            unique_path(Path::new("D"), "Shot.png", taken),
            Path::new("D").join("Shot (3).png")
        );
    }

    #[test]
    fn captures_go_where_lulos_settings_say() {
        let dirs =
            "# comment\nXDG_DESKTOP_DIR=\"E:/lulo/shots\"\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n";
        assert_eq!(
            user_dir(Some(dirs), "XDG_DESKTOP_DIR"),
            Some(PathBuf::from("E:/lulo/shots"))
        );
        assert_eq!(user_dir(Some(dirs), "XDG_DOWNLOAD_DIR"), None);
        assert_eq!(user_dir(None, "XDG_DESKTOP_DIR"), None);
        assert_eq!(
            folder_for(Some("timer=5\ndestination=downloads\n")).0,
            "XDG_DOWNLOAD_DIR"
        );
        assert_eq!(folder_for(None).0, "XDG_DESKTOP_DIR");
    }

    #[test]
    fn a_dragged_area_is_cut_out_of_the_picture() {
        // A 3 × 2 picture, each pixel's blue byte its index.
        let pixels: Vec<u8> = (0..6u8).flat_map(|index| [index, 0, 0, 255]).collect();
        let area = span(POINT { x: 3, y: 2 }, POINT { x: 1, y: 0 });
        let (cut, width, height) = crop(&pixels, 3, &area).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(
            cut.chunks(4).map(|pixel| pixel[0]).collect::<Vec<_>>(),
            [1, 2, 4, 5]
        );
        // A click without a drag takes nothing.
        let click = span(POINT { x: 1, y: 1 }, POINT { x: 1, y: 1 });
        assert!(crop(&pixels, 3, &click).is_none());
    }
}
