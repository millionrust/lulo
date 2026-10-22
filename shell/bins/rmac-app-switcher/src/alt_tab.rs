//! Alt+Tab for Lulo's switcher on Windows (ADR 0023, "Notification Centre
//! and the app switcher on Windows").
//!
//! ⌘Tab maps to Alt+Tab on Windows, Windows' own switcher chord: Lulo's
//! keyboard mapping (`rmac_app_menu::windows_keys`) turns ⌘ into Ctrl for
//! shortcuts, but Ctrl+Tab already moves between tabs in browsers and
//! editors, and Alt+Tab is what Windows users press to switch apps. Alt+Tab
//! cannot be registered as a hotkey, so a low-level keyboard hook takes it,
//! and only while this process (the Lulo layer) runs: outside Lulo mode,
//! or with an elevated window in front (whose keys a non-elevated hook
//! never sees), Windows' own switcher answers.
//!
//! The hook runs on a thread of its own, blocked in `GetMessageW`, and
//! compares a few numbers for every key; it takes keys only during a
//! switch (from the first Alt+Tab until Alt goes up):
//!
//! - Tab (Shift+Tab backwards, Alt+` backwards, ←/→) steps; the first one
//!   opens the panel. Each is swallowed with its repeats and release.
//! - Alt going up commits; Return commits too. Esc cancels.
//! - Q and H quit or hide the selected app, as ⌘Q and ⌘H do on the Mac.
//!
//! Outside a switch it takes one more chord: Ctrl+Q (⌘Q) with an app in
//! front that is not Lulo's and does not use Ctrl+Q itself quits that app,
//! as on the Mac (Windows apps otherwise have no such key). Lulo's own apps
//! quit from their own menus.
//!
//! At the first Tab it presses an unassigned key tagged as Lulo's own
//! input, so Windows treats Alt as part of a chord: releasing Alt then
//! never opens the front app's menu bar. Alt's own presses and releases
//! always pass through, so every app's idea of the keyboard stays right.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_H, VK_LEFT, VK_LMENU, VK_LWIN, VK_MENU,
    VK_OEM_3, VK_Q, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RWIN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_SYSKEYDOWN,
};

use crate::model::Command;

/// Marks the keystroke this hook sends, so it lets it pass.
const OWN_INPUT: usize = 0x4C55_5357;
/// An unassigned virtual key: pressed while Alt is down, it tells Windows
/// Alt was part of a chord (no menu bar on release).
const VK_CHORD_MARK: u16 = 0xE8;

static SENDER: OnceLock<async_channel::Sender<Command>> = OnceLock::new();
/// A switch is under way: Alt has been held since an Alt+Tab.
static SWITCHING: AtomicBool = AtomicBool::new(false);
/// The keys whose press was swallowed and whose release is still to come.
static SWALLOWED: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

fn mark_swallowed(key: u32, down: bool) {
    let (word, bit) = ((key as usize / 64) % 4, key % 64);
    if down {
        SWALLOWED[word].fetch_or(1 << bit, Ordering::AcqRel);
    } else {
        SWALLOWED[word].fetch_and(!(1 << bit), Ordering::AcqRel);
    }
}

fn was_swallowed(key: u32) -> bool {
    let (word, bit) = ((key as usize / 64) % 4, key % 64);
    SWALLOWED[word].load(Ordering::Acquire) & (1 << bit) != 0
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: reads the asynchronous key state; no pointers.
    (unsafe { GetAsyncKeyState(i32::from(key.0)) } as u16) & 0x8000 != 0
}

/// Whether an Alt key is down now (the panel's check for a release the
/// hook never saw).
pub(crate) fn alt_held() -> bool {
    key_down(VK_MENU)
}

fn send(command: Command) {
    if let Some(sender) = SENDER.get() {
        let _ = sender.try_send(command);
    }
}

/// Press and release the chord mark, tagged as this hook's own input.
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

/// What the hook does with one key, given the keys held around it. `None`
/// passes the key on; `Some(command)` swallows it and sends the command
/// (if any). Pure, so the decisions are tested on every platform's build
/// of this module's logic (`decide`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Decision {
    Pass,
    Swallow(Option<Command>),
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    alt: bool,
    shift: bool,
    ctrl: bool,
    win: bool,
}

/// The hook's rule for one key event (see the module documentation).
/// Returns the decision and whether a switch is under way afterwards.
/// Ctrl+Q, and only Ctrl: the Mac's ⌘Q.
fn is_quit_chord(key: u32, down: bool, held: Held) -> bool {
    down && key == u32::from(VK_Q.0) && held.ctrl && !held.alt && !held.shift && !held.win
}

/// Apps whose own Ctrl+Q does something else, so it passes to them:
/// Office (Outlook's Mark as Read, Word's paragraph reset), editors and
/// IDEs (Quick Open, Quick Documentation), terminals (resume output) and
/// Firefox (which quits on it itself).
const OWN_CTRL_Q: [&str; 17] = [
    "winword.exe",
    "excel.exe",
    "outlook.exe",
    "olk.exe",
    "powerpnt.exe",
    "onenote.exe",
    "msaccess.exe",
    "code.exe",
    "idea64.exe",
    "pycharm64.exe",
    "rider64.exe",
    "devenv.exe",
    "windowsterminal.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "firefox.exe",
];

/// Whether Ctrl+Q in front of `exe` (its file name) quits it: not a Lulo
/// app (they quit from their own menus) and not one that uses the key.
fn quits_on_ctrl_q(exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    !exe.is_empty()
        && rmac_apps::windows_apps::app_for_exe(&exe).is_none()
        && !matches!(
            exe.as_str(),
            "lulo-shell.exe" | "lulo-session.exe" | "explorer.exe"
        )
        && !OWN_CTRL_Q.contains(&exe.as_str())
}

/// The executable file name of the app in front.
fn front_exe() -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut pid = 0u32;
    // SAFETY: reads the foreground window's process id into a local.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    if pid == 0 {
        return String::new();
    }
    // SAFETY: a query-only handle, closed below; the name is written into
    // a local buffer of the size passed.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buffer = [0u16; 512];
        let mut length = buffer.len() as u32;
        let read = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        if read.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        rmac_apps::windows_apps::exe_key(&path)
    }
}

fn decide(key: u32, down: bool, held: Held, switching: bool) -> (Decision, bool) {
    let is = |vk: VIRTUAL_KEY| key == u32::from(vk.0);
    if is(VK_LMENU) || is(VK_RMENU) || is(VK_MENU) {
        // Alt itself always passes; its release ends a switch.
        if !down && switching {
            return (Decision::Pass, false);
        }
        return (Decision::Pass, switching);
    }
    if is(VK_TAB) {
        if down && held.alt && !held.ctrl && !held.win {
            let command = if held.shift {
                Command::Previous
            } else {
                Command::Next
            };
            return (Decision::Swallow(Some(command)), true);
        }
        return (Decision::Pass, switching);
    }
    if !switching {
        return (Decision::Pass, false);
    }
    if !held.alt {
        // Alt went up where the hook did not see it: the switch is over.
        return (Decision::Pass, false);
    }
    if !down {
        return (Decision::Pass, switching);
    }
    let command = if is(VK_ESCAPE) {
        return (Decision::Swallow(Some(Command::Cancel)), false);
    } else if is(VK_RETURN) {
        return (Decision::Swallow(Some(Command::Commit)), false);
    } else if is(VK_LEFT) || is(VK_OEM_3) {
        Command::Previous
    } else if is(VK_RIGHT) {
        Command::Next
    } else if is(VK_Q) {
        Command::QuitSelected
    } else if is(VK_H) {
        Command::HideSelected
    } else {
        return (Decision::Pass, switching);
    };
    (Decision::Swallow(Some(command)), true)
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: for HC_ACTION, `lparam` points at the event's
        // KBDLLHOOKSTRUCT for the duration of the call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if info.dwExtraInfo != OWN_INPUT {
            let key = info.vkCode;
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if !down && was_swallowed(key) {
                mark_swallowed(key, false);
                return LRESULT(1);
            }
            let held = Held {
                alt: key_down(VK_MENU),
                shift: key_down(VK_SHIFT),
                ctrl: key_down(VK_CONTROL),
                win: key_down(VK_LWIN) || key_down(VK_RWIN),
            };
            let switching = SWITCHING.load(Ordering::Acquire);
            // ⌘Q in a Windows app: the front app's name is read only for
            // this one chord.
            if !switching && is_quit_chord(key, down, held) && quits_on_ctrl_q(&front_exe()) {
                mark_swallowed(key, true);
                send(Command::QuitFront);
                return LRESULT(1);
            }
            let (decision, now) = decide(key, down, held, switching);
            SWITCHING.store(now, Ordering::Release);
            if switching && !now && matches!(decision, Decision::Pass) {
                // Alt went up: switch to the selection.
                send(Command::Commit);
            }
            if let Decision::Swallow(command) = decision {
                if !switching && now {
                    send_chord_mark();
                }
                mark_swallowed(key, true);
                if let Some(command) = command {
                    send(command);
                }
                return LRESULT(1);
            }
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Install the hook on a thread of its own; commands arrive on `sender`.
/// False when Windows refused the hook.
pub(crate) fn start(sender: async_channel::Sender<Command>) -> bool {
    if SENDER.set(sender).is_err() {
        return true;
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("lulo-alt-tab".into())
        .spawn(move || {
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
            let _ = ready_tx.send(hook.is_ok());
            let Ok(hook) = hook else {
                return;
            };
            let mut message = MSG::default();
            // SAFETY: the standard message loop; `message` is a valid MSG.
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
                // SAFETY: as above.
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            // SAFETY: the hook installed above.
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        });
    spawned.is_ok() && ready_rx.recv().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alt() -> Held {
        Held {
            alt: true,
            ..Held::default()
        }
    }

    fn vk(key: VIRTUAL_KEY) -> u32 {
        u32::from(key.0)
    }

    #[test]
    fn ctrl_q_quits_windows_apps_but_not_lulo_apps_or_apps_that_use_it() {
        let ctrl = Held {
            ctrl: true,
            ..Held::default()
        };
        assert!(is_quit_chord(vk(VK_Q), true, ctrl));
        assert!(!is_quit_chord(vk(VK_Q), false, ctrl));
        let ctrl_shift = Held {
            shift: true,
            ..ctrl
        };
        assert!(!is_quit_chord(vk(VK_Q), true, ctrl_shift));
        assert!(!is_quit_chord(vk(VK_Q), true, Held::default()));
        assert!(quits_on_ctrl_q("notepad.exe"));
        assert!(quits_on_ctrl_q("Chrome.exe"));
        assert!(!quits_on_ctrl_q("rmac-calculator.exe"));
        assert!(!quits_on_ctrl_q("OUTLOOK.EXE"));
        assert!(!quits_on_ctrl_q("code.exe"));
        assert!(!quits_on_ctrl_q("explorer.exe"));
        assert!(!quits_on_ctrl_q(""));
    }

    #[test]
    fn alt_tab_opens_steps_and_alt_up_commits() {
        // Alt down passes.
        assert_eq!(
            decide(vk(VK_LMENU), true, alt(), false),
            (Decision::Pass, false)
        );
        // The first Tab opens the switch.
        assert_eq!(
            decide(vk(VK_TAB), true, alt(), false),
            (Decision::Swallow(Some(Command::Next)), true)
        );
        // Shift+Tab and ← go back, → forward, while Alt is held.
        let shifted = Held {
            shift: true,
            ..alt()
        };
        assert_eq!(
            decide(vk(VK_TAB), true, shifted, true),
            (Decision::Swallow(Some(Command::Previous)), true)
        );
        assert_eq!(
            decide(vk(VK_LEFT), true, alt(), true),
            (Decision::Swallow(Some(Command::Previous)), true)
        );
        assert_eq!(
            decide(vk(VK_RIGHT), true, alt(), true),
            (Decision::Swallow(Some(Command::Next)), true)
        );
        // Alt up passes and ends the switch (the hook then commits).
        let released = Held::default();
        assert_eq!(
            decide(vk(VK_LMENU), false, released, true),
            (Decision::Pass, false)
        );
    }

    #[test]
    fn escape_cancels_and_q_and_h_act_on_the_selection() {
        assert_eq!(
            decide(vk(VK_ESCAPE), true, alt(), true),
            (Decision::Swallow(Some(Command::Cancel)), false)
        );
        assert_eq!(
            decide(vk(VK_RETURN), true, alt(), true),
            (Decision::Swallow(Some(Command::Commit)), false)
        );
        assert_eq!(
            decide(vk(VK_Q), true, alt(), true),
            (Decision::Swallow(Some(Command::QuitSelected)), true)
        );
        assert_eq!(
            decide(vk(VK_H), true, alt(), true),
            (Decision::Swallow(Some(Command::HideSelected)), true)
        );
        // Other keys pass, even during a switch.
        assert_eq!(
            decide(u32::from(b'A'), true, alt(), true),
            (Decision::Pass, true)
        );
    }

    #[test]
    fn other_chords_and_keys_outside_a_switch_pass() {
        // Tab alone, Ctrl+Alt+Tab (Windows' sticky switcher) and Win+Alt+Tab.
        assert_eq!(
            decide(vk(VK_TAB), true, Held::default(), false),
            (Decision::Pass, false)
        );
        let ctrl = Held {
            ctrl: true,
            ..alt()
        };
        assert_eq!(
            decide(vk(VK_TAB), true, ctrl, false),
            (Decision::Pass, false)
        );
        let win = Held { win: true, ..alt() };
        assert_eq!(
            decide(vk(VK_TAB), true, win, false),
            (Decision::Pass, false)
        );
        // Esc, Q and the arrows are the app's when no switch is under way.
        for key in [VK_ESCAPE, VK_Q, VK_LEFT, VK_RETURN] {
            assert_eq!(decide(vk(key), true, alt(), false), (Decision::Pass, false));
        }
        // A release of Alt the hook missed ends the switch at the next key.
        assert_eq!(
            decide(vk(VK_ESCAPE), true, Held::default(), true),
            (Decision::Pass, false)
        );
    }
}
