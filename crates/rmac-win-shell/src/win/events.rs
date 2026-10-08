//! What happens on the desktop, without polling: WinEvent hooks for the
//! foreground window and for app windows appearing, disappearing and being
//! minimised, plus Spotlight's hotkey. One thread owns the hooks and
//! blocks in `GetMessageW`, so it costs nothing while nothing happens.
//!
//! Spotlight's hotkey follows `model::hotkey`: Alt+Space when no other app
//! holds it, else the first free fallback. Win+Space comes through a
//! low-level keyboard hook (it is not a registrable hotkey); that hook
//! looks only at Space while a Win key is down, swallows that Space, and
//! tells Windows the Win key was used in a chord, so a tap of Win alone
//! still opens Start and Win+Space never does. Another app's hotkey is
//! never unregistered or taken: a refused `RegisterHotKey` just moves on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayoutList, RegisterHotKey, SendInput, UnregisterHotKey,
    HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VIRTUAL_KEY, VK_CONTROL,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, CHILDID_SELF, EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_HIDE, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, GA_ROOT, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, OBJID_WINDOW,
    WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_HOTKEY, WM_KEYDOWN, WM_QUIT,
    WM_SYSKEYDOWN,
};

use crate::model::hotkey::{self, Claim, Hotkey};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopEvent {
    /// Another app's window came to the front.
    Foreground(isize),
    /// App windows came, went, or were minimised or restored; read the
    /// list again. Repeats arriving before it is read are folded into one.
    WindowsChanged,
    /// Spotlight's hotkey.
    Hotkey,
}

const HOTKEY_ID: i32 = 0x4C55;
/// Marks the keystroke Lulo itself sends, so its own hook lets it pass.
const OWN_INPUT: usize = 0x4C55_4C4F;
/// An unassigned virtual key: pressed between Win down and Win up, it
/// tells Windows the Win key was part of a chord (no Start menu).
const VK_CHORD_MARK: u16 = 0xE8;

static SENDER: OnceLock<async_channel::Sender<DesktopEvent>> = OnceLock::new();
static CHANGE_PENDING: AtomicBool = AtomicBool::new(false);
/// A Space the hook swallowed is down; its repeats and release are
/// swallowed too.
static SPACE_SWALLOWED: AtomicBool = AtomicBool::new(false);

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

fn key_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: reads the asynchronous key state; no pointers.
    (unsafe { GetAsyncKeyState(i32::from(key.0)) } as u16) & 0x8000 != 0
}

/// Press and release the chord mark, tagged as Lulo's own input.
fn send_chord_mark() {
    let input = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_CHORD_MARK),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OWN_INPUT,
            },
        },
    };
    let inputs = [input(KEYBD_EVENT_FLAGS(0)), input(KEYEVENTF_KEYUP)];
    // SAFETY: two fully initialised keyboard INPUTs of the right size.
    let _ = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

/// The low-level keyboard hook, installed only while Win+Space is
/// Spotlight's hotkey. It runs on the hook thread for every key, so it
/// does nothing but compare a few numbers for anything but Win+Space.
unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: for HC_ACTION, `lparam` points at the event's
        // KBDLLHOOKSTRUCT for the duration of the call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if info.vkCode == u32::from(VK_SPACE.0) && info.dwExtraInfo != OWN_INPUT {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if !down {
                if SPACE_SWALLOWED.swap(false, Ordering::AcqRel) {
                    return LRESULT(1);
                }
            } else if SPACE_SWALLOWED.load(Ordering::Acquire) {
                // A repeat of the Space already taken.
                return LRESULT(1);
            } else if (key_down(VK_LWIN) || key_down(VK_RWIN))
                && !key_down(VK_CONTROL)
                && !key_down(VK_MENU)
                && !key_down(VK_SHIFT)
            {
                SPACE_SWALLOWED.store(true, Ordering::Release);
                send_chord_mark();
                if let Some(sender) = SENDER.get() {
                    let _ = sender.try_send(DesktopEvent::Hotkey);
                }
                return LRESULT(1);
            }
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The modifiers `RegisterHotKey` takes for a registrable hotkey.
fn modifiers(hotkey: Hotkey) -> HOT_KEY_MODIFIERS {
    match hotkey {
        Hotkey::CtrlAltSpace => MOD_CONTROL | MOD_ALT,
        Hotkey::AltShiftSpace => MOD_ALT | MOD_SHIFT,
        _ => MOD_ALT,
    }
}

/// Number of keyboard layouts the user has (Win+Space switches them).
fn keyboard_layouts() -> usize {
    // SAFETY: asks only for the count.
    unsafe { GetKeyboardLayoutList(None) }.max(0) as usize
}

/// The running hook thread.
pub struct Hooks {
    thread_id: u32,
    /// Spotlight's hotkey in effect, if any could be claimed.
    pub hotkey: Option<Hotkey>,
    /// The hotkeys that were taken by other apps before it.
    pub taken: Vec<Hotkey>,
}

impl Hooks {
    /// Unhook and end the thread.
    pub fn stop(&self) {
        // SAFETY: posts WM_QUIT to the hook thread's queue.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

/// What the hook thread claimed for Spotlight.
struct Claimed {
    hotkey: Option<Hotkey>,
    taken: Vec<Hotkey>,
    registered: bool,
    keyboard_hook: Option<HHOOK>,
}

/// Claim Spotlight's hotkey on the calling (hook) thread.
fn claim_hotkey() -> Claimed {
    let mut claimed = Claimed {
        hotkey: None,
        taken: Vec::new(),
        registered: false,
        keyboard_hook: None,
    };
    let layouts = keyboard_layouts();
    claimed.hotkey = hotkey::choose(layouts, |candidate| {
        let ok = match candidate.claim() {
            Claim::Register => {
                // SAFETY: a thread hotkey (no window): WM_HOTKEY arrives in
                // this thread's queue. Refused when another app has it;
                // nothing of theirs is touched.
                let ok = unsafe {
                    RegisterHotKey(
                        None,
                        HOTKEY_ID,
                        modifiers(candidate) | MOD_NOREPEAT,
                        u32::from(VK_SPACE.0),
                    )
                }
                .is_ok();
                claimed.registered |= ok;
                ok
            }
            Claim::Hook => {
                // SAFETY: a low-level hook whose procedure is in this
                // executable; this thread pumps messages, as such a hook
                // needs, and removes it before ending.
                let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok();
                // SAFETY: as above.
                let hook = unsafe {
                    SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(on_key),
                        module.map(|module| HINSTANCE(module.0)),
                        0,
                    )
                };
                match hook {
                    Ok(hook) => {
                        claimed.keyboard_hook = Some(hook);
                        true
                    }
                    Err(_) => false,
                }
            }
        };
        if !ok {
            claimed.taken.push(candidate);
        }
        ok
    });
    super::trace(|| {
        format!(
            "keyboard layouts {layouts}; hotkeys taken by other apps: {}",
            claimed
                .taken
                .iter()
                .map(|hotkey| hotkey.label())
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    claimed
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
            let claimed = claim_hotkey();
            // SAFETY: no arguments.
            let thread_id = unsafe { GetCurrentThreadId() };
            let _ = ready_tx.send((thread_id, claimed.hotkey, claimed.taken.clone()));
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
                if claimed.registered {
                    let _ = UnregisterHotKey(None, HOTKEY_ID);
                }
                if let Some(hook) = claimed.keyboard_hook {
                    let _ = UnhookWindowsHookEx(hook);
                }
                for hook in hooks {
                    if !hook.is_invalid() {
                        let _ = UnhookWinEvent(hook);
                    }
                }
            }
        })
        .ok()?;
    let (thread_id, hotkey, taken) = ready_rx.recv().ok()?;
    Some(Hooks {
        thread_id,
        hotkey,
        taken,
    })
}
