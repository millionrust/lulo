//! Other apps' notification-area icons when Lulo is the user's shell (ADR
//! 0023, "Lulo as the shell").
//!
//! An app adds, changes and removes its tray icon with `Shell_NotifyIcon`,
//! which finds the window of class `Shell_TrayWnd` and sends it the icon in
//! a `WM_COPYDATA` (`dwData` 1). Explorer owns that window; when Lulo runs
//! instead of Explorer, nothing would, and every tray icon (OneDrive, a
//! download manager, a chat app) would be lost. So in that mode `lulo-shell`
//! opens a hidden `Shell_TrayWnd` of its own ([`start`]), keeps the icons,
//! and the menu bar shows them as the Mac shows third-party menu extras
//! (`system::tray`). A click goes back to the app as the mouse messages
//! Explorer would have sent, by the icon's protocol version.
//!
//! Nothing polls: icons change only when an app sends them, and windows
//! that are gone are dropped whenever the list is read.

use std::cell::RefCell;
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use gpui::RenderImage;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DrawIconEx, GetCursorPos,
    GetWindowThreadProcessId, IsWindow, PostMessageW, RegisterClassW, RegisterWindowMessageW,
    SendNotifyMessageW, DI_MASK, DI_NORMAL, HICON, HWND_BROADCAST, WM_CONTEXTMENU, WM_COPYDATA,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

/// `Shell_NotifyIcon`'s data, as Explorer receives it in `WM_COPYDATA`
/// (the same for 32- and 64-bit apps: handles travel as 32 bits).
const TRAY_SIGNATURE: u32 = 0x3475_3423;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x01;
const NIF_ICON: u32 = 0x02;
const NIF_TIP: u32 = 0x04;
const NIF_STATE: u32 = 0x08;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x01;
/// `NIN_SELECT`: a version-4 icon's left click, after the button messages.
const NIN_SELECT: u32 = 0x0400;
/// Icons are kept at this size (the bar draws them at 16 points).
const ICON_PIXELS: i32 = 32;

#[repr(C)]
#[derive(Clone, Copy)]
struct NotifyIconData {
    size: u32,
    hwnd: u32,
    id: u32,
    flags: u32,
    callback_message: u32,
    icon: u32,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon_icon: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TrayData {
    signature: u32,
    message: u32,
    data: NotifyIconData,
}

/// One message an app sent, read out of its `WM_COPYDATA`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Message {
    pub message: u32,
    pub hwnd: u32,
    pub id: u32,
    pub flags: u32,
    pub callback_message: u32,
    pub icon: u32,
    pub tip: String,
    pub state: u32,
    pub state_mask: u32,
    pub version: u32,
    pub guid: Option<[u8; 16]>,
}

fn utf16(text: &[u16]) -> String {
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

/// Read a tray message from the bytes an app sent; `None` for anything
/// that is not one (wrong signature, too short).
pub(crate) fn parse(bytes: &[u8]) -> Option<Message> {
    // The header and the fields up to the tip are always there; older
    // apps send a shorter NOTIFYICONDATA, which reads as zeros past its end.
    let minimum = 8 + 24 + 128 * 2;
    if bytes.len() < minimum {
        return None;
    }
    let mut copy = [0u8; std::mem::size_of::<TrayData>()];
    let length = bytes.len().min(copy.len());
    copy[..length].copy_from_slice(&bytes[..length]);
    // SAFETY: `TrayData` is plain old data of exactly `copy`'s size, and
    // every bit pattern is a valid value of it.
    let data: TrayData = unsafe { std::ptr::read_unaligned(copy.as_ptr().cast()) };
    if data.signature != TRAY_SIGNATURE {
        return None;
    }
    let nid = data.data;
    Some(Message {
        message: data.message,
        hwnd: nid.hwnd,
        id: nid.id,
        flags: nid.flags,
        callback_message: nid.callback_message,
        icon: nid.icon,
        tip: utf16(&nid.tip),
        state: nid.state,
        state_mask: nid.state_mask,
        version: nid.version,
        guid: (nid.flags & NIF_GUID != 0 && nid.guid != [0; 16]).then_some(nid.guid),
    })
}

/// One icon in the tray.
#[derive(Clone)]
struct Icon {
    key: u64,
    hwnd: u32,
    id: u32,
    guid: Option<[u8; 16]>,
    callback_message: u32,
    version: u32,
    tip: String,
    hidden: bool,
    image: Option<Arc<RenderImage>>,
}

/// An icon's identity: its GUID when it has one, else its window and id.
pub(crate) fn key(hwnd: u32, id: u32, guid: Option<[u8; 16]>) -> u64 {
    match guid {
        Some(guid) => {
            let mut folded = 0u64;
            for chunk in guid.chunks(8) {
                let mut bytes = [0u8; 8];
                bytes[..chunk.len()].copy_from_slice(chunk);
                folded ^= u64::from_le_bytes(bytes);
            }
            folded | 1 << 63
        }
        None => (u64::from(hwnd) << 32) | u64::from(id),
    }
}

thread_local! {
    static ICONS: RefCell<Vec<Icon>> = const { RefCell::new(Vec::new()) };
    static SUBSCRIBERS: RefCell<Vec<Sender<()>>> = const { RefCell::new(Vec::new()) };
}

/// The icons to show, in the order they were added; windows that are gone
/// are dropped.
pub fn items() -> Vec<crate::system::tray::Item> {
    ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        // SAFETY: asks whether a window handle still names a window.
        icons.retain(|icon| unsafe { IsWindow(Some(handle(icon.hwnd))) }.as_bool());
        icons
            .iter()
            .filter(|icon| !icon.hidden)
            .map(|icon| crate::system::tray::Item {
                key: icon.key,
                tip: icon.tip.clone(),
                image: icon.image.clone(),
            })
            .collect()
    })
}

/// A channel that gets a message whenever the icons change.
pub fn subscribe() -> Receiver<()> {
    let (sender, receiver) = async_channel::bounded(1);
    SUBSCRIBERS.with(|subscribers| subscribers.borrow_mut().push(sender));
    receiver
}

fn changed() {
    SUBSCRIBERS.with(|subscribers| {
        subscribers.borrow_mut().retain(|subscriber| {
            !subscriber.is_closed() && (subscriber.try_send(()).is_ok() || subscriber.is_full())
        })
    });
}

fn handle(raw: u32) -> HWND {
    // Handles travel as 32 bits; Windows sign-extends them.
    HWND(raw as i32 as isize as *mut core::ffi::c_void)
}

/// The icon as BGRA at [`ICON_PIXELS`], with its transparency.
fn picture(icon: u32) -> Option<Arc<RenderImage>> {
    if icon == 0 {
        return None;
    }
    let icon = HICON(icon as i32 as isize as *mut core::ffi::c_void);
    let normal = draw(icon, DI_NORMAL)?;
    let has_alpha = normal.iter().skip(3).step_by(4).any(|&alpha| alpha != 0);
    let mut pixels = normal;
    if !has_alpha {
        // An icon without an alpha channel: its mask says what is clear.
        let mask = draw(icon, DI_MASK)?;
        for (pixel, mask) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
            pixel[3] = 255 - mask[0];
        }
    }
    let size = ICON_PIXELS as u32;
    let buffer = image::RgbaImage::from_raw(size, size, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

fn draw(icon: HICON, flags: windows::Win32::UI::WindowsAndMessaging::DI_FLAGS) -> Option<Vec<u8>> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_PIXELS,
            biHeight: -ICON_PIXELS,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: a DIB section of `info`'s size, drawn into once and read
    // within its bytes, then deleted with its DCs.
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
        let drawn =
            DrawIconEx(memory, 0, 0, icon, ICON_PIXELS, ICON_PIXELS, 0, None, flags).is_ok();
        let length = (ICON_PIXELS * ICON_PIXELS * 4) as usize;
        let pixels = (drawn && !bits.is_null())
            .then(|| std::slice::from_raw_parts(bits as *const u8, length).to_vec());
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        pixels
    }
}

/// Apply one app's message. Returns whether the icons changed.
fn apply(message: Message) -> bool {
    let key = key(message.hwnd, message.id, message.guid);
    ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        let found = icons.iter().position(|icon| icon.key == key);
        match message.message {
            NIM_DELETE => match found {
                Some(index) => {
                    icons.remove(index);
                    true
                }
                None => false,
            },
            NIM_SETVERSION => {
                if let Some(index) = found {
                    icons[index].version = message.version;
                }
                false
            }
            NIM_ADD | NIM_MODIFY => {
                if found.is_none() && message.message == NIM_MODIFY {
                    // Modifying an icon never added: Explorer refuses it.
                    return false;
                }
                let index = match found {
                    Some(index) => index,
                    None => {
                        icons.push(Icon {
                            key,
                            hwnd: message.hwnd,
                            id: message.id,
                            guid: message.guid,
                            callback_message: 0,
                            version: 0,
                            tip: String::new(),
                            hidden: false,
                            image: None,
                        });
                        icons.len() - 1
                    }
                };
                let icon = &mut icons[index];
                icon.hwnd = message.hwnd;
                if message.flags & NIF_MESSAGE != 0 {
                    icon.callback_message = message.callback_message;
                }
                if message.flags & NIF_ICON != 0 {
                    icon.image = picture(message.icon);
                }
                if message.flags & NIF_TIP != 0 {
                    icon.tip = message.tip;
                }
                if message.flags & NIF_STATE != 0 && message.state_mask & NIS_HIDDEN != 0 {
                    icon.hidden = message.state & NIS_HIDDEN != 0;
                }
                true
            }
            _ => false,
        }
    })
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_COPYDATA && lparam.0 != 0 {
        // SAFETY: Windows passes a COPYDATASTRUCT for WM_COPYDATA, valid
        // for the duration of the call, with `cbData` bytes at `lpData`.
        let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        if data.dwData == 1 && !data.lpData.is_null() {
            // SAFETY: as above.
            let bytes = unsafe {
                std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize)
            };
            return match parse(bytes) {
                Some(message) => {
                    if apply(message) {
                        changed();
                    }
                    LRESULT(1)
                }
                None => LRESULT(0),
            };
        }
        // AppBar messages (`dwData` 0) and anything else: not supported
        // here; the bar and the Dock reserve the work area themselves.
        return LRESULT(0);
    }
    // SAFETY: default handling.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Open the hidden `Shell_TrayWnd` and ask every running app to add its
/// icons again (`TaskbarCreated`), as Explorer does when it starts. Call
/// once, on the UI thread, only when no other shell owns the tray.
pub fn start() -> bool {
    // SAFETY: registers a window class with this module's procedure and
    // creates one hidden, never-activated window of it on this thread.
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: GetModuleHandleW(None)
                .ok()
                .map(Into::into)
                .unwrap_or_default(),
            lpszClassName: w!("Shell_TrayWnd"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return false;
        }
        let created = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("Shell_TrayWnd"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            GetModuleHandleW(None).ok().map(Into::into),
            None,
        );
        if created.is_err() {
            return false;
        }
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        if taskbar_created != 0 {
            let _ = SendNotifyMessageW(HWND_BROADCAST, taskbar_created, WPARAM(0), LPARAM(0));
        }
    }
    true
}

/// The `(wParam, lParam)` of a callback message for `mouse`, by the icon's
/// protocol version.
pub(crate) fn callback_parameters(version: u32, id: u32, mouse: u32, at: POINT) -> (usize, isize) {
    if version >= 4 {
        let x = at.x as i16 as u16 as usize;
        let y = at.y as i16 as u16 as usize;
        (
            (y << 16) | x,
            (((id & 0xFFFF) << 16) | (mouse & 0xFFFF)) as isize,
        )
    } else {
        (id as usize, mouse as isize)
    }
}

/// The user clicked the icon `key` in the bar (`secondary`: the right
/// button): tell its app as Explorer would.
pub fn activate(key: u64, secondary: bool) {
    let Some(icon) =
        ICONS.with(|icons| icons.borrow().iter().find(|icon| icon.key == key).cloned())
    else {
        return;
    };
    if icon.callback_message == 0 {
        return;
    }
    let target = handle(icon.hwnd);
    let mut at = POINT::default();
    let mut process = 0u32;
    // SAFETY: reads the pointer and the icon window's process, lets that
    // process take the foreground for the menu it opens, and posts it the
    // mouse messages of the click.
    unsafe {
        let _ = GetCursorPos(&mut at);
        GetWindowThreadProcessId(target, Some(&mut process as *mut u32));
        if process != 0 {
            let _ = AllowSetForegroundWindow(process);
        }
        let (down, up, after) = if secondary {
            (WM_RBUTTONDOWN, WM_RBUTTONUP, WM_CONTEXTMENU)
        } else {
            (WM_LBUTTONDOWN, WM_LBUTTONUP, NIN_SELECT)
        };
        let mut messages = vec![down, up];
        if icon.version >= 4 {
            messages.push(after);
        }
        for mouse in messages {
            let (wparam, lparam) = callback_parameters(icon.version, icon.id, mouse, at);
            let _ = PostMessageW(
                Some(target),
                icon.callback_message,
                WPARAM(wparam),
                LPARAM(lparam),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_bytes(message: u32, flags: u32, tip: &str) -> Vec<u8> {
        let mut nid = NotifyIconData {
            size: std::mem::size_of::<NotifyIconData>() as u32,
            hwnd: 0x1234,
            id: 7,
            flags,
            callback_message: 0x8001,
            icon: 0,
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: [0; 256],
            version: 4,
            info_title: [0; 64],
            info_flags: 0,
            guid: [0; 16],
            balloon_icon: 0,
        };
        for (slot, unit) in nid.tip.iter_mut().zip(tip.encode_utf16()) {
            *slot = unit;
        }
        let data = TrayData {
            signature: TRAY_SIGNATURE,
            message,
            data: nid,
        };
        // SAFETY: reads a plain-old-data struct as its bytes.
        unsafe {
            std::slice::from_raw_parts(
                (&data as *const TrayData).cast::<u8>(),
                std::mem::size_of::<TrayData>(),
            )
        }
        .to_vec()
    }

    #[test]
    fn reads_an_icon_as_explorer_receives_it() {
        let bytes = message_bytes(NIM_ADD, NIF_MESSAGE | NIF_TIP, "OneDrive");
        let message = parse(&bytes).expect("a tray message");
        assert_eq!(message.message, NIM_ADD);
        assert_eq!((message.hwnd, message.id), (0x1234, 7));
        assert_eq!(message.callback_message, 0x8001);
        assert_eq!(message.tip, "OneDrive");
        assert_eq!(message.guid, None);
    }

    #[test]
    fn refuses_what_is_not_a_tray_message() {
        let mut bytes = message_bytes(NIM_ADD, 0, "");
        bytes[0] = 0;
        assert!(parse(&bytes).is_none());
        assert!(parse(&[0; 12]).is_none());
    }

    #[test]
    fn adds_changes_hides_and_removes_icons() {
        let add = parse(&message_bytes(NIM_ADD, NIF_MESSAGE | NIF_TIP, "Chat")).unwrap();
        assert!(apply(add));
        let mut hide = parse(&message_bytes(NIM_MODIFY, NIF_STATE, "")).unwrap();
        hide.state = NIS_HIDDEN;
        hide.state_mask = NIS_HIDDEN;
        assert!(apply(hide));
        ICONS.with(|icons| {
            let icons = icons.borrow();
            assert_eq!(icons.len(), 1);
            assert!(icons[0].hidden);
            assert_eq!(icons[0].tip, "Chat");
        });
        let delete = parse(&message_bytes(NIM_DELETE, 0, "")).unwrap();
        assert!(apply(delete));
        ICONS.with(|icons| assert!(icons.borrow().is_empty()));
        let unknown = parse(&message_bytes(NIM_MODIFY, NIF_TIP, "x")).unwrap();
        assert!(!apply(unknown));
    }

    #[test]
    fn clicks_carry_the_point_and_id_for_version_four_icons() {
        let at = POINT { x: 10, y: 20 };
        assert_eq!(
            callback_parameters(4, 7, WM_LBUTTONUP, at),
            ((20 << 16) | 10, ((7 << 16) | WM_LBUTTONUP) as isize)
        );
        assert_eq!(
            callback_parameters(0, 7, WM_LBUTTONUP, at),
            (7, WM_LBUTTONUP as isize)
        );
    }

    #[test]
    fn icons_with_a_guid_are_known_by_it() {
        let guid = Some([1u8; 16]);
        assert_eq!(key(1, 2, guid), key(3, 4, guid));
        assert_ne!(key(1, 2, None), key(1, 3, None));
    }
}
