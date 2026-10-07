//! What happens on the desktop, without polling: WinEvent hooks for the
//! foreground window and for app windows appearing, disappearing and being
//! minimised, plus Spotlight's global hotkey. One thread owns the hooks
//! and blocks in `GetMessageW`, so it costs nothing while nothing happens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetAncestor, GetMessageW, PostThreadMessageW, TranslateMessage, CHILDID_SELF,
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, GA_ROOT, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_HOTKEY, WM_QUIT,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopEvent {
    /// Another app's window came to the front.
    Foreground(isize),
    /// App windows came, went, or were minimised or restored; read the
    /// list again. Repeats arriving before it is read are folded into one.
    WindowsChanged,
    /// Spotlight's hotkey (Alt+Space).
    Hotkey,
}

const HOTKEY_ID: i32 = 0x4C55;

static SENDER: OnceLock<async_channel::Sender<DesktopEvent>> = OnceLock::new();
static CHANGE_PENDING: AtomicBool = AtomicBool::new(false);

/// The UI has read the window list: the next change is reported again.
pub fn windows_read() {
    CHANGE_PENDING.store(false, Ordering::Release);
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    let Some(sender) = SENDER.get() else {
        return;
    };
    if event == EVENT_SYSTEM_FOREGROUND {
        let _ = sender.try_send(DesktopEvent::Foreground(hwnd.0 as isize));
    } else {
        // Only top-level windows are apps' windows.
        // SAFETY: reads a property of the window the event names.
        if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
            return;
        }
    }
    if !CHANGE_PENDING.swap(true, Ordering::AcqRel) {
        let _ = sender.try_send(DesktopEvent::WindowsChanged);
    }
}

/// The running hook thread.
pub struct Hooks {
    thread_id: u32,
    pub hotkey: bool,
}

impl Hooks {
    /// Unhook and end the thread.
    pub fn stop(&self) {
        // SAFETY: posts WM_QUIT to the hook thread's queue.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

/// Start watching the desktop; events arrive on `sender`.
pub fn start(sender: async_channel::Sender<DesktopEvent>) -> Option<Hooks> {
    let _ = SENDER.set(sender);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("lulo-shell-events".into())
        .spawn(move || {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let ranges = [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
                (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
                (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
            ];
            // SAFETY: out-of-context hooks deliver to this thread's queue,
            // which the loop below pumps; they are removed before it ends.
            let hooks = ranges
                .iter()
                .map(|&(first, last)| unsafe {
                    SetWinEventHook(first, last, None, Some(on_event), 0, 0, flags)
                })
                .collect::<Vec<_>>();
            // SAFETY: a thread hotkey (no window): WM_HOTKEY arrives in
            // this thread's queue.
            let hotkey = unsafe {
                RegisterHotKey(
                    None,
                    HOTKEY_ID,
                    MOD_ALT | MOD_NOREPEAT,
                    u32::from(VK_SPACE.0),
                )
            }
            .is_ok();
            // SAFETY: no arguments.
            let thread_id = unsafe { GetCurrentThreadId() };
            let _ = ready_tx.send((thread_id, hotkey));
            let mut message = MSG::default();
            // SAFETY: the standard message loop; `message` is a valid MSG.
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
                if message.message == WM_HOTKEY && message.wParam.0 == HOTKEY_ID as usize {
                    if let Some(sender) = SENDER.get() {
                        let _ = sender.try_send(DesktopEvent::Hotkey);
                    }
                    continue;
                }
                // SAFETY: as above.
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            // SAFETY: undoes the registrations made above.
            unsafe {
                if hotkey {
                    let _ = UnregisterHotKey(None, HOTKEY_ID);
                }
                for hook in hooks {
                    if !hook.is_invalid() {
                        let _ = UnhookWinEvent(hook);
                    }
                }
            }
        })
        .ok()?;
    let (thread_id, hotkey) = ready_rx.recv().ok()?;
    Some(Hooks { thread_id, hotkey })
}
